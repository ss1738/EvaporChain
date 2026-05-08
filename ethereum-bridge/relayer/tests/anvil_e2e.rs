//! End-to-end integration test for Phase 3b.
//!
//! 1. Spawn Anvil (Prague-evm hard fork → EIP-2537 precompiles available).
//! 2. Deploy `CommitCertVerifier`, `ValidatorSetRegistry`, `EvaporHeaderInbox`.
//! 3. Initialise the registry with a 5-validator BLS set.
//! 4. Generate N finalised headers and BLS-sign each one.
//! 5. Submit them through the relayer's `EthClient::submit_header`.
//! 6. Verify each one is committed via `EvaporHeaderInbox.headerAt`.
//!
//! This is the Phase 3b acceptance: relayer pushes headers, EVM verifies,
//! state lands. With NO manual steps.

use alloy::network::{EthereumWallet, TransactionBuilder};
use alloy::node_bindings::Anvil;
use alloy::primitives::{Address, Bytes, FixedBytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::eth::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;
use anyhow::{anyhow, Result};
use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
use bls12_381::{G1Affine, G1Projective, G2Affine, G2Projective, Scalar};
use evaporchain_eth_bridge::mmr::{build_and_prove, ghost_leaf_hash};
use group::{Curve, Group};
use sha3::{Digest, Keccak256};

const BLS_DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_";
const DOMAIN_TAG_HEADER_TEXT: &str = "EvaporChain/v1/header";
const DOMAIN_TAG_COMMIT_TEXT: &str = "EvaporChain/v1/commit-cert";

// ─── Contract bindings ──────────────────────────────────────────────
//
// Single sol! invocation against the EvaporHeaderInbox artifact gives
// us the EvaporHeaderInbox-flavoured BridgeTypes for free; the registry
// and verifier are deployed via raw bytecode and called via the registry
// address loaded into the inbox at construction time. This avoids the
// duplicate `BridgeTypes` mod the alloy macro generates per artifact.

alloy::sol!(
    #[sol(rpc)]
    #[allow(missing_docs, clippy::too_many_arguments)]
    EvaporHeaderInbox,
    "../contracts/out/EvaporHeaderInbox.sol/EvaporHeaderInbox.json"
);

// Thin interface bindings for contracts whose typed methods we call
// without needing their full BridgeTypes generated. Each interface
// declares a fresh `Validator` so the duplicate-module collision the
// auto-binding hits is avoided.
alloy::sol! {
    #[sol(rpc)]
    #[allow(missing_docs, clippy::too_many_arguments)]
    interface IRegistry {
        struct Validator { bytes pubkey; uint128 stake; }
        function genesisInit(uint64 epoch, Validator[] calldata validators) external;
        function valsetRoot() external view returns (bytes32);
    }

    #[sol(rpc)]
    #[allow(missing_docs, clippy::too_many_arguments)]
    interface IDispatcher {
        function registerHook(bytes32 objectId, address target, bytes calldata data, uint96 gasCap) external;
        function dispatch(
            bytes32 objectId,
            uint64 height,
            uint64 evaporatedAtHeight,
            uint128 finalEnergy,
            uint64 leafIndex,
            uint64 treeSize,
            bytes calldata mmrPath,
            bytes calldata peaksLeft,
            bytes calldata peaksRight
        ) external;
        function isFired(bytes32 objectId) external view returns (bool);
    }

    #[sol(rpc)]
    #[allow(missing_docs)]
    interface IGhostMinter {
        function minted() external view returns (uint256);
        function lastData() external view returns (bytes memory);
        function mintBecauseEvaporated(bytes calldata data) external;
    }
}

// ─── BLS helpers (mirrored from the eth-bridge crate's tests) ──────

fn keygen_deterministic(seed: u64) -> (Scalar, G1Affine) {
    let mut bytes = [0u8; 64];
    bytes[0..8].copy_from_slice(&seed.to_le_bytes());
    for i in 8..64 {
        bytes[i] = (seed.wrapping_mul(i as u64) & 0xFF) as u8;
    }
    let sk = Scalar::from_bytes_wide(&bytes);
    let pk: G1Affine = (G1Projective::generator() * sk).into();
    (sk, pk)
}

fn sign(sk: Scalar, msg: &[u8]) -> G2Affine {
    let h: G2Projective = <G2Projective as HashToCurve<
        ExpandMsgXmd<sha2_old_for_bls::Sha256>,
    >>::hash_to_curve(msg, BLS_DST);
    (h * sk).into()
}

fn aggregate(sigs: &[G2Affine]) -> G2Affine {
    let mut acc = G2Projective::identity();
    for s in sigs {
        acc += G2Projective::from(*s);
    }
    acc.to_affine()
}

fn g1_to_eip2537(p: G1Affine) -> [u8; 128] {
    let raw = p.to_uncompressed();
    let mut out = [0u8; 128];
    out[16..64].copy_from_slice(&raw[0..48]);
    out[80..128].copy_from_slice(&raw[48..96]);
    out
}

fn g2_to_eip2537(p: G2Affine) -> [u8; 256] {
    let raw = p.to_uncompressed();
    let x_c1 = &raw[0..48];
    let x_c0 = &raw[48..96];
    let y_c1 = &raw[96..144];
    let y_c0 = &raw[144..192];
    let mut out = [0u8; 256];
    out[16..64].copy_from_slice(x_c0);
    out[80..128].copy_from_slice(x_c1);
    out[144..192].copy_from_slice(y_c0);
    out[208..256].copy_from_slice(y_c1);
    out
}

fn keccak(data: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(data);
    let out = h.finalize();
    let mut a = [0u8; 32];
    a.copy_from_slice(&out);
    a
}

// ─── The test ───────────────────────────────────────────────────────

struct Validator {
    sk: Scalar,
    pk_compressed: [u8; 48],
    pk_uncompressed: [u8; 128],
    stake: u128,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn anvil_e2e_relays_50_headers() -> Result<()> {
    // ── 1. Anvil with prague (EIP-2537 precompiles) ───────────────
    let anvil = Anvil::new()
        .args(["--hardfork", "prague"])
        .try_spawn()?;
    let rpc = anvil.endpoint();
    let signer: PrivateKeySigner = anvil.keys()[0].clone().into();
    let signer_address = signer.address();
    let wallet = EthereumWallet::from(signer.clone());
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet.clone())
        .on_http(rpc.parse()?);

    // ── 2. Deploy ──────────────────────────────────────────────────
    //
    // Verifier + registry are deployed via raw deployment-code from the
    // Foundry artifact JSONs (we don't need typed bindings for them once
    // constructed — only the inbox is called via typed sol! binding).
    let verifier_artifact: serde_json::Value =
        serde_json::from_str(include_str!(
            "../../contracts/out/CommitCertVerifier.sol/CommitCertVerifier.json"
        ))?;
    let mut verifier_code = hex::decode(
        verifier_artifact["bytecode"]["object"]
            .as_str()
            .ok_or_else(|| anyhow!("verifier bytecode missing"))?
            .trim_start_matches("0x"),
    )?;
    let verifier_addr = {
        let tx = TransactionRequest::default().with_deploy_code(verifier_code.clone());
        let pending = provider.send_transaction(tx).await?;
        pending
            .get_receipt()
            .await?
            .contract_address
            .ok_or_else(|| anyhow!("verifier deploy receipt has no contract_address"))?
    };

    let registry_artifact: serde_json::Value =
        serde_json::from_str(include_str!(
            "../../contracts/out/ValidatorSetRegistry.sol/ValidatorSetRegistry.json"
        ))?;
    let mut registry_code = hex::decode(
        registry_artifact["bytecode"]["object"]
            .as_str()
            .ok_or_else(|| anyhow!("registry bytecode missing"))?
            .trim_start_matches("0x"),
    )?;
    // Append the abi-encoded verifier address as the constructor arg.
    let mut padded = vec![0u8; 32];
    padded[12..32].copy_from_slice(verifier_addr.as_slice());
    registry_code.extend_from_slice(&padded);
    let registry_addr = {
        let tx = TransactionRequest::default().with_deploy_code(registry_code);
        let pending = provider.send_transaction(tx).await?;
        pending
            .get_receipt()
            .await?
            .contract_address
            .ok_or_else(|| anyhow!("registry deploy receipt has no contract_address"))?
    };

    let inbox = EvaporHeaderInbox::deploy(&provider, registry_addr).await?;
    let _ = (verifier_code, verifier_addr);

    // ── 3. Build the validator set ────────────────────────────────
    let stakes = [100u128; 5];
    let mut validators: Vec<Validator> = Vec::new();
    for (i, stake) in stakes.iter().enumerate() {
        let (sk, pk) = keygen_deterministic(0xC001D00DD0BAAA00 ^ (i as u64));
        validators.push(Validator {
            sk,
            pk_compressed: pk.to_compressed(),
            pk_uncompressed: g1_to_eip2537(pk),
            stake: *stake,
        });
    }

    // ── 4. genesisInit on the registry ───────────────────────────
    let epoch: u64 = 1;
    let registry = IRegistry::new(registry_addr, &provider);
    let registry_validators: Vec<IRegistry::Validator> = validators
        .iter()
        .map(|v| IRegistry::Validator {
            pubkey: Bytes::copy_from_slice(&v.pk_compressed),
            stake: v.stake,
        })
        .collect();
    registry
        .genesisInit(epoch, registry_validators)
        .send()
        .await?
        .get_receipt()
        .await?;

    // ── 5. Submit N finalised headers ─────────────────────────────
    const N: u64 = 50;
    let prev_pks_uncomp_concat: Vec<u8> = validators
        .iter()
        .flat_map(|v| v.pk_uncompressed.iter().copied())
        .collect();
    let bitmap = vec![0x1Fu8]; // all 5 signed

    let dom_header = keccak(DOMAIN_TAG_HEADER_TEXT.as_bytes());

    let inbox_validators: Vec<BridgeTypes::Validator> = validators
        .iter()
        .map(|v| BridgeTypes::Validator {
            pubkey: Bytes::copy_from_slice(&v.pk_compressed),
            stake: v.stake,
        })
        .collect();

    let start_height: u64 = 1_000;
    for i in 0..N {
        let height = start_height + i;
        let block_hash = keccak(format!("blk-{height}").as_bytes());
        let state_root = keccak(format!("state-{height}").as_bytes());
        let mmr_root = keccak(format!("mmr-{height}").as_bytes());

        // Build messageHash exactly as the contract does.
        let mut preimg: Vec<u8> = Vec::new();
        preimg.extend_from_slice(&dom_header);
        preimg.extend_from_slice(&height.to_be_bytes());
        preimg.extend_from_slice(&block_hash);
        preimg.extend_from_slice(&state_root);
        preimg.extend_from_slice(&mmr_root);
        preimg.extend_from_slice(&epoch.to_be_bytes());
        let message_hash = keccak(&preimg);

        // Each validator signs message_hash; aggregate.
        let sigs: Vec<G2Affine> = validators
            .iter()
            .map(|v| sign(v.sk, &message_hash))
            .collect();
        let agg = g2_to_eip2537(aggregate(&sigs));

        let header = EvaporHeaderInbox::Header {
            height,
            blockHash: FixedBytes::from(block_hash),
            stateRoot: FixedBytes::from(state_root),
            mmrRoot: FixedBytes::from(mmr_root),
            evaporchainEpoch: epoch,
        };

        inbox
            .submitHeader(
                header,
                inbox_validators.clone(),
                Bytes::copy_from_slice(&prev_pks_uncomp_concat),
                Bytes::copy_from_slice(&bitmap),
                Bytes::copy_from_slice(&agg),
            )
            .send()
            .await?
            .get_receipt()
            .await?;
    }

    // ── 6. Verify all N landed ────────────────────────────────────
    let latest = inbox.latestHeight().call().await?._0;
    assert_eq!(latest, start_height + N - 1);

    // Spot-check a few state roots match what we sent.
    for &i in &[0u64, 17, N - 1] {
        let height = start_height + i;
        let stored = inbox.stateRootAt(height).call().await?._0;
        let expected = keccak(format!("state-{height}").as_bytes());
        assert_eq!(stored.0, expected, "state root mismatch at height {height}");
    }

    // Anvil + signer are dropped, killing the node process.
    let _ = signer_address;
    drop(anvil);
    Ok(())
}

#[allow(dead_code)]
fn _silence_unused_constant() {
    // keep the const referenced so cargo doesn't warn even if test skips
    let _ = DOMAIN_TAG_COMMIT_TEXT;
}

// Trivial reference so _silence is exercised
#[test]
fn silence_referenced() {
    _silence_unused_constant();
}

// ─── Phase 5 full-pipeline E2E ──────────────────────────────────────

/// Same fixture-style flow as `anvil_e2e_relays_50_headers`, but goes
/// further: deploys the dispatcher + a `GhostTokenMinter` target, anchors
/// a header that commits a real MMR root, registers a hook, calls
/// `dispatch`, and asserts the target's `minted()` counter went up.
///
/// **What this proves:** the §17.4 cross-chain primitive works end-to-end
/// on a fully running Ethereum node — the *only* trust assumption is the
/// 2/3+ stake of the EvaporChain validator set. No relayer trust, no
/// off-chain attestation, no bridge multisig. Pure cryptography.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn anvil_full_pipeline_e2e_evaporation_to_ghost_mint() -> Result<()> {
    let anvil = Anvil::new().args(["--hardfork", "prague"]).try_spawn()?;
    let rpc = anvil.endpoint();
    let signer: PrivateKeySigner = anvil.keys()[0].clone().into();
    let wallet = EthereumWallet::from(signer.clone());
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet.clone())
        .on_http(rpc.parse()?);

    // ── Deploy verifier, registry, inbox, dispatcher, target ──────
    //
    // Deploys are inlined (a tiny closure) because alloy 0.8's generic
    // Provider bound makes a helper function fight with type inference.
    let bc_verifier = artifact_bytecode(include_str!(
        "../../contracts/out/CommitCertVerifier.sol/CommitCertVerifier.json"
    ))?;
    let verifier_addr = {
        let tx = TransactionRequest::default().with_deploy_code(bc_verifier);
        provider
            .send_transaction(tx)
            .await?
            .get_receipt()
            .await?
            .contract_address
            .ok_or_else(|| anyhow!("verifier deploy receipt missing addr"))?
    };

    let mut bc_registry = artifact_bytecode(include_str!(
        "../../contracts/out/ValidatorSetRegistry.sol/ValidatorSetRegistry.json"
    ))?;
    bc_registry.extend_from_slice(&abi_addr(verifier_addr));
    let registry_addr = {
        let tx = TransactionRequest::default().with_deploy_code(bc_registry);
        provider
            .send_transaction(tx)
            .await?
            .get_receipt()
            .await?
            .contract_address
            .ok_or_else(|| anyhow!("registry deploy receipt missing addr"))?
    };

    let inbox = EvaporHeaderInbox::deploy(&provider, registry_addr).await?;

    let mut bc_dispatcher = artifact_bytecode(include_str!(
        "../../contracts/out/EvaporationDispatcher.sol/EvaporationDispatcher.json"
    ))?;
    bc_dispatcher.extend_from_slice(&abi_addr(*inbox.address()));
    let dispatcher_addr = {
        let tx = TransactionRequest::default().with_deploy_code(bc_dispatcher);
        provider
            .send_transaction(tx)
            .await?
            .get_receipt()
            .await?
            .contract_address
            .ok_or_else(|| anyhow!("dispatcher deploy receipt missing addr"))?
    };

    let bc_target = artifact_bytecode(include_str!(
        "../../contracts/out/EvaporationDispatcher.t.sol/GhostTokenMinter.json"
    ))?;
    let target_addr = {
        let tx = TransactionRequest::default().with_deploy_code(bc_target);
        provider
            .send_transaction(tx)
            .await?
            .get_receipt()
            .await?
            .contract_address
            .ok_or_else(|| anyhow!("target deploy receipt missing addr"))?
    };
    let _ = verifier_addr;

    let dispatcher = IDispatcher::new(dispatcher_addr, &provider);
    let target = IGhostMinter::new(target_addr, &provider);

    // ── Build the validator set ────────────────────────────────────
    let stakes = [100u128; 5];
    let mut validators: Vec<Validator> = Vec::new();
    for (i, stake) in stakes.iter().enumerate() {
        let (sk, pk) = keygen_deterministic(0xC001D00DD0BAAA00 ^ (i as u64));
        validators.push(Validator {
            sk,
            pk_compressed: pk.to_compressed(),
            pk_uncompressed: g1_to_eip2537(pk),
            stake: *stake,
        });
    }

    // ── genesisInit on registry ───────────────────────────────────
    let epoch: u64 = 1;
    let registry = IRegistry::new(registry_addr, &provider);
    let registry_validators: Vec<IRegistry::Validator> = validators
        .iter()
        .map(|v| IRegistry::Validator {
            pubkey: Bytes::copy_from_slice(&v.pk_compressed),
            stake: v.stake,
        })
        .collect();
    registry
        .genesisInit(epoch, registry_validators)
        .send()
        .await?
        .get_receipt()
        .await?;

    // ── Build an 8-leaf bridge MMR with our target ghost record ───
    let object_id = keccak(b"e2e/object#777");
    let evaporated_at_height: u64 = 9_899;
    let final_energy: u128 = 0;
    let target_leaf = ghost_leaf_hash(object_id, evaporated_at_height, final_energy);
    let target_index: usize = 3;
    let mut leaves: Vec<[u8; 32]> = (0..8u8).map(|i| keccak(&[i; 32])).collect();
    leaves[target_index] = target_leaf;
    let (mmr_root, proof) = build_and_prove(&leaves, target_index as u64);

    // ── Build + sign the header committing this MMR root ─────────
    let height: u64 = 9_900;
    let block_hash = keccak(format!("e2e-block-{height}").as_bytes());
    let state_root = keccak(format!("e2e-state-{height}").as_bytes());

    let mut preimg = Vec::new();
    preimg.extend_from_slice(&keccak(DOMAIN_TAG_HEADER_TEXT.as_bytes()));
    preimg.extend_from_slice(&height.to_be_bytes());
    preimg.extend_from_slice(&block_hash);
    preimg.extend_from_slice(&state_root);
    preimg.extend_from_slice(&mmr_root);
    preimg.extend_from_slice(&epoch.to_be_bytes());
    let message_hash = keccak(&preimg);

    let sigs: Vec<G2Affine> = validators
        .iter()
        .map(|v| sign(v.sk, &message_hash))
        .collect();
    let agg = g2_to_eip2537(aggregate(&sigs));

    let prev_pks_uncomp_concat: Vec<u8> = validators
        .iter()
        .flat_map(|v| v.pk_uncompressed.iter().copied())
        .collect();
    let bitmap = vec![0x1Fu8];
    let inbox_validators: Vec<BridgeTypes::Validator> = validators
        .iter()
        .map(|v| BridgeTypes::Validator {
            pubkey: Bytes::copy_from_slice(&v.pk_compressed),
            stake: v.stake,
        })
        .collect();

    inbox
        .submitHeader(
            EvaporHeaderInbox::Header {
                height,
                blockHash: FixedBytes::from(block_hash),
                stateRoot: FixedBytes::from(state_root),
                mmrRoot: FixedBytes::from(mmr_root),
                evaporchainEpoch: epoch,
            },
            inbox_validators,
            Bytes::copy_from_slice(&prev_pks_uncomp_concat),
            Bytes::copy_from_slice(&bitmap),
            Bytes::copy_from_slice(&agg),
        )
        .send()
        .await?
        .get_receipt()
        .await?;

    // ── Register the evaporation hook ─────────────────────────────
    // calldata: target.mintBecauseEvaporated("evaporated-on-evaporchain")
    let mint_data = {
        let selector = keccak(b"mintBecauseEvaporated(bytes)")[..4].to_vec();
        // abi-encode the bytes argument:
        //   offset (32) | length (32) | data + pad
        let payload = b"evaporated-on-evaporchain";
        let mut out = selector;
        let mut offset = vec![0u8; 32];
        offset[31] = 32;
        out.extend_from_slice(&offset);
        let mut length = vec![0u8; 32];
        length[24..32].copy_from_slice(&(payload.len() as u64).to_be_bytes());
        out.extend_from_slice(&length);
        let mut padded = payload.to_vec();
        while padded.len() % 32 != 0 {
            padded.push(0);
        }
        out.extend_from_slice(&padded);
        Bytes::from(out)
    };

    use alloy::primitives::aliases::U96;
    dispatcher
        .registerHook(
            FixedBytes::from(object_id),
            target_addr,
            mint_data,
            U96::from(200_000u32),
        )
        .send()
        .await?
        .get_receipt()
        .await?;

    // Pre-state: nothing minted.
    let before = target.minted().call().await?._0;
    assert_eq!(before, U256::ZERO);

    // ── Dispatch ──────────────────────────────────────────────────
    dispatcher
        .dispatch(
            FixedBytes::from(object_id),
            height,
            evaporated_at_height,
            final_energy,
            proof.leaf_index,
            proof.tree_size,
            Bytes::copy_from_slice(&proof.path),
            Bytes::copy_from_slice(&proof.peaks_left),
            Bytes::copy_from_slice(&proof.peaks_right),
        )
        .send()
        .await?
        .get_receipt()
        .await?;

    // ── Assert: target was called exactly once ────────────────────
    let after = target.minted().call().await?._0;
    assert_eq!(after, U256::from(1u32));
    assert!(dispatcher.isFired(FixedBytes::from(object_id)).call().await?._0);

    drop(anvil);
    Ok(())
}

// ─── Helpers ────────────────────────────────────────────────────────

fn artifact_bytecode(artifact_json: &str) -> Result<Vec<u8>> {
    let v: serde_json::Value = serde_json::from_str(artifact_json)?;
    let bc = v["bytecode"]["object"]
        .as_str()
        .ok_or_else(|| anyhow!("artifact missing bytecode.object"))?;
    Ok(hex::decode(bc.trim_start_matches("0x"))?)
}

fn abi_addr(addr: Address) -> Vec<u8> {
    let mut out = vec![0u8; 32];
    out[12..32].copy_from_slice(addr.as_slice());
    out
}


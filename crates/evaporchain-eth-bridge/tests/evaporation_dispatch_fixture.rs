//! Generate the cross-side evaporation-dispatch test fixture.
//!
//! Produces `ethereum-bridge/contracts/fixtures/evaporation_dispatch_8.json`
//! containing:
//!   - 5 BLS validators (same deterministic seeds as other fixtures)
//!   - an 8-leaf bridge MMR with one designated ghost record
//!   - a finalised header committing that MMR root, signed by all 5 validators
//!   - the inclusion proof for the target ghost record
//!
//! Consumed by `EvaporationDispatcher.t.sol::test_evaporationFiresHook`.

use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
use bls12_381::{G1Affine, G1Projective, G2Affine, G2Projective, Scalar};
use evaporchain_eth_bridge::mmr::{build_and_prove, ghost_leaf_hash};
use evaporchain_eth_bridge::valset::{compute_root, Validator};
use group::{Curve, Group};
use serde_json::json;
use sha3::{Digest, Keccak256};
use std::fs;
use std::path::PathBuf;

const BLS_DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_";
const DOMAIN_TAG_HEADER_TEXT: &str = "EvaporChain/v1/header";

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

#[test]
fn generate_evaporation_dispatch_fixture() {
    let epoch: u64 = 1;
    let height: u64 = 9_000;

    // Build the validator set + signatures.
    let stakes = [100u128; 5];
    let mut sks: Vec<Scalar> = Vec::new();
    let mut pks_compressed: Vec<[u8; 48]> = Vec::new();
    let mut pks_uncompressed: Vec<[u8; 128]> = Vec::new();
    for (i, _) in stakes.iter().enumerate() {
        let (sk, pk) = keygen_deterministic(0xC001D00DD0BAAA00 ^ (i as u64));
        sks.push(sk);
        pks_compressed.push(pk.to_compressed());
        pks_uncompressed.push(g1_to_eip2537(pk));
    }
    let validators: Vec<Validator> = pks_compressed
        .iter()
        .zip(stakes.iter())
        .map(|(pk, s)| Validator {
            pubkey: *pk,
            stake: *s,
        })
        .collect();
    let (valset_root, _total) = compute_root(epoch, &validators).unwrap();

    // Build the bridge MMR with 8 ghost-record leaves. The 6th (index=5)
    // is our target — that's the object we'll register a hook for.
    let target_object_id: [u8; 32] = keccak(b"object#42 evaporated 2026");
    let target_evaporated_at_height: u64 = 8_900;
    let target_final_energy: u128 = 0;

    let target_leaf = ghost_leaf_hash(
        target_object_id,
        target_evaporated_at_height,
        target_final_energy,
    );
    let mut leaves: Vec<[u8; 32]> = (0..8u8)
        .map(|i| keccak(&[i; 32]))
        .collect();
    let target_index: usize = 5;
    leaves[target_index] = target_leaf;

    let (mmr_root, proof) = build_and_prove(&leaves, target_index as u64);

    let block_hash = keccak(format!("evaporchain block {height}").as_bytes());
    let state_root = keccak(format!("state @ {height}").as_bytes());

    // Header message hash (same encoding the Solidity contract uses).
    let dom = keccak(DOMAIN_TAG_HEADER_TEXT.as_bytes());
    let mut preimg: Vec<u8> = Vec::new();
    preimg.extend_from_slice(&dom);
    preimg.extend_from_slice(&height.to_be_bytes());
    preimg.extend_from_slice(&block_hash);
    preimg.extend_from_slice(&state_root);
    preimg.extend_from_slice(&mmr_root);
    preimg.extend_from_slice(&epoch.to_be_bytes());
    let message_hash = keccak(&preimg);

    let sigs: Vec<G2Affine> = sks.iter().map(|sk| sign(*sk, &message_hash)).collect();
    let agg_enc = g2_to_eip2537(aggregate(&sigs));

    let bitmap = vec![0x1Fu8];

    let pks_uncomp_concat: Vec<u8> = pks_uncompressed
        .iter()
        .flat_map(|p| p.iter().copied())
        .collect();

    let fixture = json!({
        "epoch": epoch,
        "height": height,
        "block_hash": format!("0x{}", hex::encode(block_hash)),
        "state_root": format!("0x{}", hex::encode(state_root)),
        "mmr_root": format!("0x{}", hex::encode(mmr_root)),
        "valset_root": format!("0x{}", hex::encode(valset_root)),
        "validators": validators.iter().map(|v| json!({
            "pubkey_compressed": format!("0x{}", hex::encode(v.pubkey)),
            "stake": v.stake.to_string(),
        })).collect::<Vec<_>>(),
        "prev_pubkeys_uncompressed": format!("0x{}", hex::encode(&pks_uncomp_concat)),
        "signed_bitmap": format!("0x{}", hex::encode(&bitmap)),
        "agg_signature_uncompressed": format!("0x{}", hex::encode(agg_enc)),
        "message_hash": format!("0x{}", hex::encode(message_hash)),

        // Evaporation-dispatch payload:
        "object_id": format!("0x{}", hex::encode(target_object_id)),
        "target_evaporated_at_height": target_evaporated_at_height,
        "target_final_energy": target_final_energy.to_string(),
        "leaf_index": proof.leaf_index,
        "tree_size": proof.tree_size,
        "mmr_path": format!("0x{}", hex::encode(&proof.path)),
        "peaks_left": format!("0x{}", hex::encode(&proof.peaks_left)),
        "peaks_right": format!("0x{}", hex::encode(&proof.peaks_right)),
    });

    let mut path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.push("ethereum-bridge");
    path.push("contracts");
    path.push("fixtures");
    fs::create_dir_all(&path).unwrap();
    path.push("evaporation_dispatch_8.json");

    fs::write(&path, serde_json::to_vec_pretty(&fixture).unwrap()).unwrap();
    eprintln!("wrote fixture: {}", path.display());
}

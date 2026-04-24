mod api;
mod auth;
mod bench;
mod frontier;
mod oracle_bridge;
mod persistence;
mod shard_bridge;
mod user_db;
mod ws;

use anyhow::Result;
use api::{ApiState, BlockRecord, ChainStats, EpochSnapshot, EventRecord, NftStore, NftToken, TokenStore, DeployedToken, StakingStore, StakingPool, Staker, DAOStore, DAOProposal, DAOVote, ThroughputTracker};
use evaporchain_consensus::MockConsensus;
use evaporchain_consensus::tendermint::{TendermintConsensus, ConsensusMessage, ConsensusAction, ProofVerifier, AnchorHashProvider};
use evaporchain_consensus::validator_set::{ValidatorInfo, ValidatorSet};
use evaporchain_network::service::{cache_block, NetworkConfig, P2pNetworkService};
use evaporchain_proving::{MockProver, ProvingEngine};
use evaporchain_proving::chain_proof::ChainProver;
use evaporchain_state::db::StateDB;
use evaporchain_state::RocksDBStateDB;
use evaporchain_crypto::signatures::{MlDsaKeypair, Signer};
use evaporchain_da::block_da::{BlockDA, BlockDAPackage};
use evaporchain_da::block_da_2d::BlockDA2D;
use evaporchain_types::{
    Account, CreateObjectTx, ObjectState, RefreshTx, StateObject, Transaction, TransferTx,
};
use persistence::ChainStore;
use rand::Rng;
use serde::Deserialize;
use std::collections::{BTreeMap, VecDeque};
use std::io::BufRead;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

// ──────────────────────────── Lock Helper ──────────────────────────────

/// Safely acquire a Mutex lock, recovering from poisoned state.
fn safe_lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        tracing::warn!("Recovered poisoned mutex lock");
        poisoned.into_inner()
    })
}

fn log_persist_err(op: &str, r: Result<(), String>) {
    if let Err(e) = r {
        eprintln!("\x1b[31mPersistence error ({}): {}\x1b[0m", op, e);
    }
}

fn persist_contracts(
    chain_store: &persistence::ChainStore,
    tendermint: &Option<Arc<Mutex<evaporchain_consensus::tendermint::TendermintConsensus>>>,
) {
    if let Some(ref tc_ref) = tendermint {
        let tc = safe_lock(tc_ref);
        let scripts: Vec<_> = tc.script_engine().all_contracts().into_iter().collect();
        let templates: Vec<_> = tc.contract_engine().all_contracts().into_iter().collect();
        log_persist_err("script_contracts", chain_store.save_script_contracts(&scripts));
        log_persist_err("template_contracts", chain_store.save_template_contracts(&templates));
    }
}

// ──────────────── Nova Proof Verifier (bridge to ChainProver) ───────────

/// Implements `ProofVerifier` for consensus by delegating to a `ChainProver`.
struct ChainProofVerifier {
    prover: Arc<Mutex<ChainProver>>,
}

impl ProofVerifier for ChainProofVerifier {
    fn verify_block_proof(
        &self,
        proof_bytes: &[u8],
        block_height: u64,
        genesis_state_root: [u8; 32],
    ) -> bool {
        let p = safe_lock(&self.prover);
        let proof = evaporchain_proving::CompressedProof {
            proof_bytes: proof_bytes.to_vec(),
            num_steps: block_height as usize,
            z0_bytes: genesis_state_root.to_vec(),
        };
        match p.verify_chain_proof(&evaporchain_proving::chain_proof::ChainProof {
            proof,
            genesis_state_root,
            final_state_root: [0u8; 32], // not checked in verify path
            block_height,
            final_epoch: 0,
            created_at: 0,
            proof_size_bytes: proof_bytes.len(),
            num_steps: block_height as usize,
        }) {
            Ok(valid) => valid,
            Err(e) => {
                tracing::warn!("Proof verification error: {}", e);
                false
            }
        }
    }
}

/// Implements `AnchorHashProvider` by reading the latest anchor from FrontierState.
struct FrontierAnchorProvider {
    frontier: Arc<Mutex<frontier::FrontierState>>,
}

impl AnchorHashProvider for FrontierAnchorProvider {
    fn anchor_hash_for_height(&self, height: u64) -> Option<[u8; 32]> {
        let fs = safe_lock(&self.frontier);
        if fs.anchors.is_anchor_height(height) {
            Some(fs.anchors.latest_anchor_hash())
        } else {
            None
        }
    }
}

// ──────────────────────────── Configuration ─────────────────────────────

const GRACE_PERIOD: u64 = 5;
const BLOCK_INTERVAL_MS: u64 = 1000;
const DEMO_TX_CHANCE: f64 = 0.15; // 15% chance of a demo tx each tick

// ──────────────────────────── Genesis State ──────────────────────────────

fn addr(b: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    a[0] = b;
    a
}

fn obj_id(b: u8) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[0] = b;
    id
}

fn seed_demo_accounts(db: &mut RocksDBStateDB, node_tag: &str) {
    use api::{GENESIS_FOUNDATION, GENESIS_CORE_DEV, GENESIS_VALIDATOR1, GENESIS_VALIDATOR2, GENESIS_ECOSYSTEM, GENESIS_COMMUNITY, parse_hex_address};
    use evaporchain_state::db::StateDB;
    let accounts: [(&str, u64); 6] = [
        (GENESIS_FOUNDATION, 487_293),
        (GENESIS_CORE_DEV,   234_851),
        (GENESIS_VALIDATOR1,  128_472),
        (GENESIS_VALIDATOR2,   91_337),
        (GENESIS_ECOSYSTEM,    52_184),
        (GENESIS_COMMUNITY,    38_916),
    ];
    for (hex, balance) in &accounts {
        let address = parse_hex_address(hex).expect("invalid demo address");
        if db.get_account(&address).is_none() {
            db.put_account(Account { address, balance: *balance, nonce: 0 });
        }
    }
    println!("{} \x1b[36mDemo accounts seeded (6 accounts for demo tx generation)\x1b[0m", node_tag);
}

/// Produce 2D erasure encoding with NMT blob commitments for a block.
/// Populates `block.da_row_roots`, `block.da_col_roots`, and `block.blob_commitments`.
/// Returns the 2D data_root which replaces the 1D commitment root in the block header.
fn encode_block_2d(block: &mut evaporchain_types::Block, block_bytes: &[u8]) -> Option<[u8; 32]> {
    use evaporchain_da::block_da_2d::{BlockDA2D, namespace_for_tx_type};
    use evaporchain_da::namespace::NamespacedBlob;

    let da2d = BlockDA2D::new();

    // Build namespace-tagged blobs from transactions
    let blobs: Vec<NamespacedBlob> = block.transactions.iter().filter_map(|tx| {
        let (ns_type, data) = match tx {
            Transaction::Transfer(t) => ("transfer", serde_json::to_vec(t).ok()?),
            Transaction::CreateObject(t) => ("create_object", serde_json::to_vec(t).ok()?),
            Transaction::Refresh(t) => ("refresh", serde_json::to_vec(t).ok()?),
            _ => return None,
        };
        Some(NamespacedBlob {
            namespace: namespace_for_tx_type(ns_type),
            data,
        })
    }).collect();

    match da2d.encode_block_with_blobs(block_bytes, &blobs) {
        Ok(package) => {
            block.da_row_roots = package.header.row_roots;
            block.da_col_roots = package.header.col_roots;
            block.blob_commitments = package.header.blob_commitments;
            Some(package.header.data_root)
        }
        Err(_) => None,
    }
}

fn initialize_genesis(db: &mut RocksDBStateDB, node_tag: &str) {
    use api::{GENESIS_FOUNDATION, GENESIS_CORE_DEV, GENESIS_VALIDATOR1, GENESIS_VALIDATOR2, GENESIS_ECOSYSTEM, GENESIS_COMMUNITY, parse_hex_address};

    // Faucet address (all-zeros) pre-seeded with large supply
    let faucet_addr = [0u8; 32];
    db.put_account(Account {
        address: faucet_addr,
        balance: u64::MAX / 2,
        nonce: 0,
    });
    println!("{} \x1b[36mFaucet (0x0000...)\x1b[0m  balance=MAX/2", node_tag);

    let accounts: Vec<(&str, u64)> = vec![
        (GENESIS_FOUNDATION, 487_293),
        (GENESIS_CORE_DEV,   234_851),
        (GENESIS_VALIDATOR1,  128_472),
        (GENESIS_VALIDATOR2,   91_337),
        (GENESIS_ECOSYSTEM,    52_184),
        (GENESIS_COMMUNITY,    38_916),
    ];
    for (hex, balance) in &accounts {
        let address = parse_hex_address(hex).expect("invalid genesis address");
        db.put_account(Account {
            address,
            balance: *balance,
            nonce: 0,
        });
        println!(
            "{} \x1b[36m0x{}\x1b[0m  balance={}",
            node_tag, hex, balance
        );
    }

    // Realistic objects with diverse use-case names and parameters
    let objects: Vec<(u8, &str, u64, u64, &str)> = vec![
        (0x10, GENESIS_FOUNDATION, 50_000, 50_000, "token:evap-governance"),
        (0x11, GENESIS_CORE_DEV, 30_000, 50_000, "stake:validator-pool-1"),
        (0x12, GENESIS_ECOSYSTEM, 5_000, 10_000, "nft:event-ticket-0x3f"),
        (0x13, GENESIS_ECOSYSTEM, 8_000, 10_000, "escrow:freelance-0x8b"),
        (0x14, GENESIS_VALIDATOR2, 2_000, 5_000, "dao:proposal-0x5e"),
        (0x15, GENESIS_COMMUNITY, 800, 100, "session:auth-0x1a"),       // decays visibly
        (0x16, GENESIS_VALIDATOR1, 400, 50, "cache:price-feed-0x9c"),   // dies in hours
        (0x17, GENESIS_COMMUNITY, 150, 20, "msg:ephemeral-0xd7"),       // dies fast — demo
    ];

    for (oid, owner_hex, energy, half_life, label) in &objects {
        let owner = parse_hex_address(owner_hex).expect("invalid genesis address");
        db.put_object(StateObject {
            id: obj_id(*oid),
            owner,
            energy: *energy,
            half_life: *half_life,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: label.as_bytes().to_vec(),
            decay_curve: None,
        });
        println!(
            "{} \x1b[33m{}\x1b[0m  id=0x{:02x}..  energy={:<6} half_life={}",
            node_tag, label, oid, energy, half_life
        );
    }
}

// ──────────────────────────── Display Helpers ────────────────────────────

fn print_banner(node_tag: &str) {
    println!();
    println!(
        "\x1b[1;35m╔══════════════════════════════════════════════════════════════╗\x1b[0m"
    );
    println!(
        "\x1b[1;35m║           EvaporChain — Multi-Node Devnet v0.2              ║\x1b[0m"
    );
    println!(
        "\x1b[1;35m║       Thermodynamic State Decay in Real Time                ║\x1b[0m"
    );
    println!(
        "\x1b[1;35m╚══════════════════════════════════════════════════════════════╝\x1b[0m"
    );
    println!("{} Node starting...", node_tag);
    println!();
}

#[allow(clippy::too_many_arguments)]
fn print_block_result(
    node_tag: &str,
    source: &str,
    block_num: u64,
    epoch: u64,
    txs_executed: usize,
    txs_failed: usize,
    entered_grace: usize,
    evaporated: usize,
    active_objects: usize,
    ghost_count: usize,
    state_root: &[u8; 32],
    peer_count: usize,
) {
    let root_hex = &hex::encode(state_root)[..16];

    println!();
    println!(
        "{} \x1b[1;32m━━━ Block #{:<4} │ Epoch {:<4} ━━━ {} ━━━━━━━━━━━━━━━━━━━━━━\x1b[0m",
        node_tag, block_num, epoch, source
    );

    if txs_executed > 0 || txs_failed > 0 {
        println!(
            "{}   Transactions:  \x1b[32m{} ok\x1b[0m  \x1b[31m{} failed\x1b[0m",
            node_tag, txs_executed, txs_failed
        );
    }

    if entered_grace > 0 {
        println!(
            "{}   \x1b[33m⚠ {} object(s) entered GRACE period\x1b[0m",
            node_tag, entered_grace
        );
    }
    if evaporated > 0 {
        println!(
            "{}   \x1b[31m💀 {} object(s) EVAPORATED → ghost\x1b[0m",
            node_tag, evaporated
        );
    }

    println!(
        "{}   State: \x1b[36m{} active\x1b[0m  \x1b[90m{} ghosts\x1b[0m  root=\x1b[1m{}…\x1b[0m  peers={}",
        node_tag, active_objects, ghost_count, root_hex, peer_count
    );
}

// ──────────────────────────── Stdin Commands ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum StdinCommand {
    #[serde(rename = "transfer")]
    Transfer {
        from: u8,
        to: u8,
        amount: u64,
        nonce: u64,
    },
    #[serde(rename = "create_object")]
    CreateObject {
        creator: u8,
        object_id: u8,
        energy: u64,
        half_life: u64,
    },
    #[serde(rename = "refresh")]
    Refresh {
        object_id: u8,
        energy_deposit: u64,
    },
}

fn parse_stdin_command(line: &str, signer: &MlDsaKeypair) -> Option<Transaction> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    if let Ok(cmd) = serde_json::from_str::<StdinCommand>(line) {
        let mut tx = match cmd {
            StdinCommand::Transfer {
                from,
                to,
                amount,
                nonce,
            } => Transaction::Transfer(TransferTx {
                from: addr(from),
                to: addr(to),
                amount,
                nonce,
                signature: None,
                public_key: None,
            }),
            StdinCommand::CreateObject {
                creator,
                object_id,
                energy,
                half_life,
            } => Transaction::CreateObject(CreateObjectTx {
                creator: addr(creator),
                object_id: obj_id(object_id),
                energy,
                half_life,
                data: format!("UserObj-{}", object_id).into_bytes(),
                decay_curve: None,
                signature: None,
                public_key: None,
            }),
            StdinCommand::Refresh {
                object_id,
                energy_deposit,
            } => Transaction::Refresh(RefreshTx {
                object_id: obj_id(object_id),
                energy_deposit,
                signature: None,
                public_key: None,
            }),
        };
        // Sign the transaction with the stdin keypair
        let msg = tx.signable_bytes();
        let sig = signer.sign(&msg);
        let pk = signer.public_key_bytes();
        match &mut tx {
            Transaction::Transfer(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
            Transaction::CreateObject(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
            Transaction::Refresh(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
            _ => {}
        }
        return Some(tx);
    }

    eprintln!("\x1b[31mInvalid command: {}\x1b[0m", line);
    None
}

// ──────────────────────────── Demo Mode ──────────────────────────────────

fn generate_demo_tx(
    rng: &mut impl Rng,
    epoch: u64,
    nonces: &mut [u64; 4],
    keypairs: &[MlDsaKeypair; 6],
    validator_id: u64,
    validator_count: u64,
    db: &Arc<Mutex<evaporchain_state::RocksDBStateDB>>,
) -> Option<Transaction> {
    use api::{GENESIS_FOUNDATION, GENESIS_CORE_DEV, GENESIS_VALIDATOR1, GENESIS_VALIDATOR2, GENESIS_ECOSYSTEM, GENESIS_COMMUNITY, parse_hex_address};
    use evaporchain_state::db::StateDB;

    let roll: f64 = rng.gen();
    if roll > DEMO_TX_CHANCE {
        return None;
    }

    let all_hexes: [&str; 6] = [
        GENESIS_FOUNDATION, GENESIS_CORE_DEV, GENESIS_VALIDATOR1,
        GENESIS_VALIDATOR2, GENESIS_ECOSYSTEM, GENESIS_COMMUNITY,
    ];

    // Partition accounts by validator_id to prevent nonce/duplicate collisions.
    // validator_id can be 0-indexed or 1-indexed depending on CLI usage.
    let per_validator = (all_hexes.len() as u64 / validator_count.max(1)) as usize;
    if per_validator == 0 {
        return None;
    }
    let idx = if validator_id == 0 { 0 } else { (validator_id - 1) as usize };
    let start = idx * per_validator;
    let end = if start + per_validator >= all_hexes.len() {
        all_hexes.len()
    } else {
        start + per_validator
    };
    let my_accts = &all_hexes[start..end];
    let my_keypairs = &keypairs[start..end];
    if my_accts.is_empty() {
        return None;
    }

    let obj_ids: [u8; 8] = [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17];
    let prefixes = ["swap:", "lock:", "vote:", "proof:", "cert:", "stream:", "relay:", "index:"];

    let action = rng.gen_range(0u8..10);

    match action {
        0..=4 => {
            let fi = rng.gen_range(0..my_accts.len());
            let mut ti = rng.gen_range(0..all_hexes.len());
            let from_global = start + fi;
            while ti == from_global { ti = rng.gen_range(0..all_hexes.len()); }
            let from = parse_hex_address(my_accts[fi]).unwrap();
            let to = parse_hex_address(all_hexes[ti]).unwrap();
            let amount = rng.gen_range(100..5000);
            // Read on-chain nonce to avoid collisions across validators
            let nonce = {
                let db_guard = safe_lock(db);
                db_guard.get_account(&from).map(|a| a.nonce).unwrap_or(0)
            };
            let slot = (from_global % 4) as usize;
            // Track locally in case multiple txs from same account in one block
            let nonce = nonce + nonces[slot];
            nonces[slot] += 1;
            let mut tx = Transaction::Transfer(TransferTx {
                from,
                to,
                amount,
                nonce,
                signature: None,
                public_key: None,
            });
            let msg = tx.signable_bytes();
            let sig = my_keypairs[fi].sign(&msg);
            let pk = my_keypairs[fi].public_key_bytes();
            if let Transaction::Transfer(ref mut inner) = tx {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Some(tx)
        }
        5 | 6 => {
            // CreateObject: include validator_id in object ID to avoid collisions
            let oid = 0x20 + ((epoch * validator_count + validator_id) % 200) as u8;
            let energy = rng.gen_range(15..120);
            let half_life = rng.gen_range(500..5000);
            let ci = rng.gen_range(0..my_accts.len());
            let creator = parse_hex_address(my_accts[ci]).unwrap();
            let prefix = prefixes[rng.gen_range(0..prefixes.len())];
            let name = format!("{}v{}:0x{:02x}{:02x}", prefix, validator_id, oid, (epoch % 256) as u8);
            let curve = match rng.gen_range(0u8..5) {
                0 => Some(evaporchain_types::DecayCurve::Linear { rate_per_epoch: rng.gen_range(1..10) }),
                1 => Some(evaporchain_types::DecayCurve::Asymptotic { floor: rng.gen_range(5..20), half_life: half_life }),
                _ => None, // default exponential
            };
            let mut tx = Transaction::CreateObject(CreateObjectTx {
                creator,
                object_id: obj_id(oid),
                energy,
                half_life,
                data: name.into_bytes(),
                decay_curve: curve,
                signature: None,
                public_key: None,
            });
            let msg = tx.signable_bytes();
            let sig = my_keypairs[ci].sign(&msg);
            let pk = my_keypairs[ci].public_key_bytes();
            if let Transaction::CreateObject(ref mut inner) = tx {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Some(tx)
        }
        7 | 8 => {
            let target = obj_ids[rng.gen_range(0..obj_ids.len())];
            let deposit = rng.gen_range(100..800);
            let si = rng.gen_range(0..my_keypairs.len());
            let mut tx = Transaction::Refresh(RefreshTx {
                object_id: obj_id(target),
                energy_deposit: deposit,
                signature: None,
                public_key: None,
            });
            let msg = tx.signable_bytes();
            let sig = my_keypairs[si].sign(&msg);
            let pk = my_keypairs[si].public_key_bytes();
            if let Transaction::Refresh(ref mut inner) = tx {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Some(tx)
        }
        _ => {
            let target = obj_ids[rng.gen_range(0..5)];
            let si = rng.gen_range(0..my_keypairs.len());
            let mut tx = Transaction::Refresh(RefreshTx {
                object_id: obj_id(target),
                energy_deposit: rng.gen_range(500..5000),
                signature: None,
                public_key: None,
            });
            let msg = tx.signable_bytes();
            let sig = my_keypairs[si].sign(&msg);
            let pk = my_keypairs[si].public_key_bytes();
            if let Transaction::Refresh(ref mut inner) = tx {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Some(tx)
        }
    }
}

// ──────────────────────────── Arg Parsing ─────────────────────────────────

struct NodeArgs {
    demo_mode: bool,
    prove_mode: bool,
    network_mode: bool,
    api_mode: bool,
    api_port: u16,
    block_ms: u64,
    port: u16,
    node_id: String,
    startup_delay_ms: u64,
    bootstrap_peers: Vec<String>,
    data_dir: String,
    /// Enable Tendermint BFT consensus (requires --network).
    tendermint_mode: bool,
    /// This node's validator ID (for Tendermint consensus).
    validator_id: u64,
    /// Total number of validators in the set (for genesis validator set).
    validator_count: u64,
    /// Stake for each validator (default 1000).
    validator_stake: u64,
    /// Block gas limit (default 500_000; use --high-throughput for 10M).
    block_gas_limit: u64,
    /// High-throughput mode: 10M gas limit, 200ms blocks.
    high_throughput: bool,
    /// Path to a genesis JSON config file (overrides hardcoded genesis).
    genesis_config: Option<String>,
    /// Enable TLS 1.3 transport for peer connections (libp2p-tls).
    use_tls: bool,
    /// Comma-separated list of authorized PeerIds (empty = permissionless).
    allowed_peers: Vec<String>,
}

fn parse_args() -> NodeArgs {
    let args: Vec<String> = std::env::args().collect();
    let demo_mode = args.iter().any(|a| a == "--demo");
    let prove_mode = args.iter().any(|a| a == "--prove");
    let network_mode = args.iter().any(|a| a == "--network");
    let api_mode = args.iter().any(|a| a == "--api");
    let api_port = args
        .iter()
        .position(|a| a == "--api-port")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(8080);
    let block_ms = args
        .iter()
        .position(|a| a == "--interval")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(BLOCK_INTERVAL_MS);
    let port = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(0);
    let node_id = args
        .iter()
        .position(|a| a == "--node-id")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "node".to_string());
    let startup_delay_ms = args
        .iter()
        .position(|a| a == "--startup-delay")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(if network_mode { 5000 } else { 0 });
    let data_dir = args
        .iter()
        .position(|a| a == "--data-dir")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "./evaporchain-data".to_string());

    let mock_consensus = args.iter().any(|a| a == "--mock-consensus");
    let tendermint_mode = !mock_consensus;
    let validator_id = args
        .iter()
        .position(|a| a == "--validator-id")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1);
    let validator_count = args
        .iter()
        .position(|a| a == "--validators")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(4);
    let validator_stake = args
        .iter()
        .position(|a| a == "--stake")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1000);

    let high_throughput = args.iter().any(|a| a == "--high-throughput");
    let block_gas_limit = args
        .iter()
        .position(|a| a == "--block-gas-limit")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(if high_throughput { 10_000_000 } else { 500_000 });

    // High-throughput mode overrides block interval
    let block_ms = if high_throughput && block_ms == BLOCK_INTERVAL_MS {
        200 // 5 blocks/sec
    } else {
        block_ms
    };

    let genesis_config = args
        .iter()
        .position(|a| a == "--genesis-config")
        .and_then(|i| args.get(i + 1))
        .cloned();

    let use_tls = args.iter().any(|a| a == "--tls");

    let allowed_peers: Vec<String> = args
        .iter()
        .position(|a| a == "--allowed-peers")
        .and_then(|i| args.get(i + 1))
        .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    let mut bootstrap_peers = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--bootstrap" {
            if let Some(addr) = args.get(i + 1) {
                bootstrap_peers.push(addr.clone());
            }
            i += 2;
        } else {
            i += 1;
        }
    }

    NodeArgs {
        demo_mode,
        prove_mode,
        network_mode,
        api_mode,
        api_port,
        block_ms,
        port,
        node_id,
        startup_delay_ms,
        bootstrap_peers,
        data_dir,
        tendermint_mode,
        validator_id,
        validator_count,
        validator_stake,
        block_gas_limit,
        high_throughput,
        genesis_config,
        use_tls,
        allowed_peers,
    }
}

// ──────────────────────────── Colors ─────────────────────────────────────

/// Return a color escape code based on node_id for visual distinction.
fn node_color(node_id: &str) -> &'static str {
    match node_id {
        "node-1" => "\x1b[1;36m",  // cyan
        "node-2" => "\x1b[1;33m",  // yellow
        "node-3" => "\x1b[1;35m",  // magenta
        "node-4" => "\x1b[1;32m",  // green
        _ => "\x1b[1;37m",         // white
    }
}

fn make_tag(node_id: &str) -> String {
    let color = node_color(node_id);
    format!("{}[{}]\x1b[0m", color, node_id)
}

// ──────────────────────────── API Recording ──────────────────────────────

use evaporchain_execution::BlockExecutionResult;

/// Record a block into the API shared state (block history, stats, events).
fn record_block(
    block_history: &Arc<Mutex<VecDeque<BlockRecord>>>,
    chain_stats: &Arc<Mutex<ChainStats>>,
    events: &Arc<Mutex<VecDeque<api::EventRecord>>>,
    throughput: &Arc<Mutex<ThroughputTracker>>,
    ws_broadcaster: Option<&Arc<ws::WsBroadcaster>>,
    block: &evaporchain_types::Block,
    execution: &BlockExecutionResult,
    active_objects: usize,
    ghost_count: usize,
    exec_time_us: u64,
) {
    // Record throughput metrics
    {
        let mut t = safe_lock(throughput);
        t.record_block(block.timestamp, block.transactions.len(), exec_time_us, execution.gas_used);
    }
    // Compute total energy from block txs for stats
    let mut tx_creates = 0u64;
    let mut tx_refreshes = 0u64;
    for tx in &block.transactions {
        match tx {
            Transaction::CreateObject(_) => tx_creates += 1,
            Transaction::Refresh(_) => tx_refreshes += 1,
            _ => {}
        }
    }

    let record = BlockRecord {
        number: block.number,
        epoch: block.epoch,
        parent_hash: hex::encode(block.parent_hash),
        state_root: hex::encode(execution.state_root),
        tx_count: block.transactions.len(),
        evaporations: execution.objects_evaporated,
        entered_grace: execution.objects_entered_grace,
        timestamp: block.timestamp,
        active_objects,
        ghost_count,
        gas_used: execution.gas_used,
        base_fee: execution.base_fee,
        total_fees: execution.total_fees,
        transactions: api::tx_records_from_block(block),
        has_nova_proof: block.nova_proof.is_some(),
        nova_proof_size: block.nova_proof.as_ref().map_or(0, |p| p.len()),
        data_root: block.data_root.map(hex::encode),
        da_square_size: block.da_row_roots.len(),
        blob_count: block.blob_commitments.len(),
        has_state_commitment: block.state_function_commitment.is_some(),
        is_anchor: block.state_function_commitment.as_ref().map_or(false, |c| c.is_anchor),
        anchor_epoch: block.state_function_commitment.as_ref().map_or(0, |c| c.anchor_epoch),
    };

    // Push to block history
    {
        let mut history = safe_lock(&block_history);
        history.push_back(record);
        while history.len() > 500 {
            history.pop_front();
        }
    }

    // Update stats
    {
        let mut stats = safe_lock(&chain_stats);
        stats.total_objects_created += tx_creates;
        stats.total_refreshed += tx_refreshes;
        stats.total_evaporated += execution.objects_evaporated as u64;
        stats.total_transactions += block.transactions.len() as u64;

        // Compute total energy across active objects (approximate from active count * avg)
        // We store per-epoch snapshot for the timeline chart
        stats.state_size_trend.push(EpochSnapshot {
            epoch: block.epoch,
            active_count: active_objects,
            ghost_count,
            total_energy: 0, // filled below if needed
        });
        // Cap trend to last 1000 epochs to prevent unbounded memory growth
        if stats.state_size_trend.len() > 1000 {
            let excess = stats.state_size_trend.len() - 1000;
            stats.state_size_trend.drain(..excess);
        }
    }

    // Push events
    if execution.objects_entered_grace > 0 {
        api::push_event(
            events,
            block.epoch,
            "grace",
            format!(
                "{} object(s) entered GRACE period",
                execution.objects_entered_grace
            ),
        );
    }
    if execution.objects_evaporated > 0 {
        api::push_event(
            events,
            block.epoch,
            "evaporated",
            format!(
                "{} object(s) EVAPORATED",
                execution.objects_evaporated
            ),
        );
    }
    if tx_creates > 0 {
        api::push_event(
            events,
            block.epoch,
            "created",
            format!("{} object(s) created", tx_creates),
        );
    }
    if tx_refreshes > 0 {
        api::push_event(
            events,
            block.epoch,
            "refreshed",
            format!("{} object(s) refreshed", tx_refreshes),
        );
    }
    if execution.txs_executed > 0 {
        api::push_event(
            events,
            block.epoch,
            "block",
            format!(
                "Block #{} produced: {} tx, {} evaporated",
                block.number, execution.txs_executed, execution.objects_evaporated
            ),
        );
    }

    // Publish to WebSocket subscribers
    if let Some(broadcaster) = ws_broadcaster {
        broadcaster.publish(ws::WsEvent::NewBlock {
            number: block.number,
            epoch: block.epoch,
            tx_count: block.transactions.len(),
            timestamp: block.timestamp,
            state_root: hex::encode(execution.state_root),
            producer: block.producer_id.map(|id| format!("validator_{}", id)),
        });

        for tx in &block.transactions {
            let (tx_type, from, to, amount) = match tx {
                Transaction::Transfer(t) => (
                    "transfer",
                    hex::encode(t.from),
                    Some(hex::encode(t.to)),
                    Some(t.amount),
                ),
                Transaction::CreateObject(t) => (
                    "create_object",
                    hex::encode(t.creator),
                    None,
                    None,
                ),
                Transaction::Refresh(t) => (
                    "refresh",
                    hex::encode(t.object_id),
                    None,
                    Some(t.energy_deposit),
                ),
                _ => continue,
            };
            let hash = hex::encode(blake3::hash(&serde_json::to_vec(tx).unwrap_or_default()).as_bytes());
            broadcaster.publish(ws::WsEvent::NewTransaction {
                hash,
                tx_type: tx_type.to_string(),
                from,
                to,
                amount,
            });
        }

        if execution.objects_evaporated > 0 {
            broadcaster.publish(ws::WsEvent::ChainEvent {
                event_type: "evaporated".to_string(),
                message: format!("{} object(s) evaporated", execution.objects_evaporated),
                epoch: block.epoch,
                timestamp_ms: block.timestamp,
            });
        }

        if execution.objects_entered_grace > 0 {
            broadcaster.publish(ws::WsEvent::ChainEvent {
                event_type: "grace".to_string(),
                message: format!("{} object(s) entered grace period", execution.objects_entered_grace),
                epoch: block.epoch,
                timestamp_ms: block.timestamp,
            });
        }
    }
}

/// Index structured contract events from a block execution result into RocksDB
/// and broadcast them via WebSocket.
fn index_contract_events_from_exec(
    chain_store: &persistence::ChainStore,
    block: &evaporchain_types::Block,
    execution: &BlockExecutionResult,
) {
    if execution.contract_events.is_empty() {
        return;
    }
    // Group events by contract_id
    let mut grouped: std::collections::HashMap<u64, Vec<&evaporchain_script::ContractEvent>> =
        std::collections::HashMap::new();
    for bce in &execution.contract_events {
        grouped.entry(bce.contract_id).or_default().push(&bce.event);
    }
    for (contract_id, events) in &grouped {
        let tx_hash = format!("block_{}", block.number);
        let owned: Vec<evaporchain_script::ContractEvent> = events.iter().map(|e| (*e).clone()).collect();
        log_persist_err(
            "contract_events",
            chain_store.index_contract_events(
                block.number,
                block.epoch,
                block.timestamp,
                *contract_id,
                &tx_hash,
                &owned,
            ).map(|_| ()),
        );
    }
}

fn broadcast_contract_events(
    ws_broadcaster: &ws::WsBroadcaster,
    block: &evaporchain_types::Block,
    execution: &BlockExecutionResult,
) {
    for bce in &execution.contract_events {
        ws_broadcaster.publish(ws::WsEvent::ContractLog {
            contract_id: bce.contract_id,
            block_number: block.number,
            event_name: bce.event.name.clone(),
            topics: bce.event.topics.iter().map(|v| format!("{v}")).collect(),
            data: bce.event.data.iter().map(|v| format!("{v}")).collect(),
        });
    }
}

// ──────────────────────────── Genesis NFTs ───────────────────────────────

fn initialize_nft_store() -> NftStore {
    use api::{GENESIS_FOUNDATION, GENESIS_CORE_DEV, GENESIS_VALIDATOR1, GENESIS_VALIDATOR2, GENESIS_ECOSYSTEM, GENESIS_COMMUNITY};

    fn mkhash(seed: &str) -> String {
        blake3::hash(seed.as_bytes()).to_hex().to_string()
    }

    let genesis_nfts = vec![
        NftToken {
            id: 1,
            name: "Genesis #001".to_string(),
            collection: "Genesis Collection".to_string(),
            owner: format!("0x{}", GENESIS_FOUNDATION),
            metadata_hash: mkhash("genesis:001:eternal-flame"),
            energy: 100_000,
            max_energy: 100_000,
            half_life: 50_000,   // stays alive for weeks
            minted_epoch: 0,
            last_refreshed: 0,
            state: "Active".to_string(),
            grace_epoch: None,
            evaporated_epoch: None,
            ghost_proof: None,
        },
        NftToken {
            id: 2,
            name: "Genesis #002".to_string(),
            collection: "Genesis Collection".to_string(),
            owner: format!("0x{}", GENESIS_COMMUNITY),
            metadata_hash: mkhash("genesis:002:shooting-star"),
            energy: 80_000,
            max_energy: 80_000,
            half_life: 20_000,   // stays alive for days
            minted_epoch: 0,
            last_refreshed: 0,
            state: "Active".to_string(),
            grace_epoch: None,
            evaporated_epoch: None,
            ghost_proof: None,
        },
        NftToken {
            id: 3,
            name: "Genesis #003".to_string(),
            collection: "Genesis Collection".to_string(),
            owner: format!("0x{}", GENESIS_CORE_DEV),
            metadata_hash: mkhash("genesis:003:sunset-canvas"),
            energy: 50_000,
            max_energy: 50_000,
            half_life: 10_000,   // stays alive for days
            minted_epoch: 0,
            last_refreshed: 0,
            state: "Active".to_string(),
            grace_epoch: None,
            evaporated_epoch: None,
            ghost_proof: None,
        },
        NftToken {
            id: 4,
            name: "Genesis #004".to_string(),
            collection: "Genesis Collection".to_string(),
            owner: format!("0x{}", GENESIS_ECOSYSTEM),
            metadata_hash: mkhash("genesis:004:quantum-bloom"),
            energy: 20_000,
            max_energy: 20_000,
            half_life: 1_000,    // visible decay over hours
            minted_epoch: 0,
            last_refreshed: 0,
            state: "Active".to_string(),
            grace_epoch: None,
            evaporated_epoch: None,
            ghost_proof: None,
        },
        NftToken {
            id: 5,
            name: "Genesis #005".to_string(),
            collection: "Genesis Collection".to_string(),
            owner: format!("0x{}", GENESIS_VALIDATOR1),
            metadata_hash: mkhash("genesis:005:first-light"),
            energy: 5_000,
            max_energy: 5_000,
            half_life: 100,      // decays visibly — shows the concept
            minted_epoch: 0,
            last_refreshed: 0,
            state: "Active".to_string(),
            grace_epoch: None,
            evaporated_epoch: None,
            ghost_proof: None,
        },
        NftToken {
            id: 6,
            name: "Genesis #006".to_string(),
            collection: "Genesis Collection".to_string(),
            owner: format!("0x{}", GENESIS_VALIDATOR2),
            metadata_hash: mkhash("genesis:006:binary-requiem"),
            energy: 500,
            max_energy: 500,
            half_life: 20,       // dies fast — demonstrates evaporation
            minted_epoch: 0,
            last_refreshed: 0,
            state: "Active".to_string(),
            grace_epoch: None,
            evaporated_epoch: None,
            ghost_proof: None,
        },
    ];
    NftStore {
        tokens: genesis_nfts,
        next_id: 7,
    }
}

// ──────────────────────────── Genesis Tokens ─────────────────────────────

fn initialize_token_store() -> TokenStore {
    use api::{GENESIS_FOUNDATION, GENESIS_CORE_DEV, GENESIS_VALIDATOR1, GENESIS_VALIDATOR2, GENESIS_ECOSYSTEM, GENESIS_COMMUNITY};

    let f = |h: &str| format!("0x{}", h);

    let mut evap_balances = std::collections::HashMap::new();
    evap_balances.insert(f(GENESIS_FOUNDATION), 487_293);
    evap_balances.insert(f(GENESIS_CORE_DEV), 234_851);
    evap_balances.insert(f(GENESIS_VALIDATOR1), 128_472);
    evap_balances.insert(f(GENESIS_ECOSYSTEM), 73_184);
    evap_balances.insert(f(GENESIS_COMMUNITY), 38_916);

    let mut flux_balances = std::collections::HashMap::new();
    flux_balances.insert(f(GENESIS_FOUNDATION), 94_217);
    flux_balances.insert(f(GENESIS_CORE_DEV), 47_183);
    flux_balances.insert(f(GENESIS_ECOSYSTEM), 41_872);

    let mut heat_balances = std::collections::HashMap::new();
    heat_balances.insert(f(GENESIS_COMMUNITY), 9_437);
    heat_balances.insert(f(GENESIS_VALIDATOR2), 4_821);

    TokenStore {
        tokens: vec![
            DeployedToken {
                id: 1, name: "EvaporChain".into(), symbol: "EVAP".into(),
                total_supply: 962_716, decay_half_life: 100_000,  // barely decays
                deployed_epoch: 0, deployer: f(GENESIS_FOUNDATION),
                balances: evap_balances, last_decay_epoch: 0,
            },
            DeployedToken {
                id: 2, name: "Flux Token".into(), symbol: "FLUX".into(),
                total_supply: 183_272, decay_half_life: 5_000,  // slow decay over hours
                deployed_epoch: 0, deployer: f(GENESIS_FOUNDATION),
                balances: flux_balances, last_decay_epoch: 0,
            },
            DeployedToken {
                id: 3, name: "Thermal Credits".into(), symbol: "HEAT".into(),
                total_supply: 14_258, decay_half_life: 100,  // decays fast — demo token
                deployed_epoch: 0, deployer: f(GENESIS_COMMUNITY),
                balances: heat_balances, last_decay_epoch: 0,
            },
        ],
        next_id: 4,
    }
}

// ──────────────────────────── Genesis Staking ────────────────────────────

fn initialize_staking_store() -> StakingStore {
    use api::{GENESIS_FOUNDATION, GENESIS_VALIDATOR1, GENESIS_VALIDATOR2};
    let f = |h: &str| format!("0x{}", h);

    StakingStore {
        pools: vec![
            StakingPool {
                id: 1,
                name: "Genesis Validator Pool".into(),
                reward_rate: 100,
                reward_decay_hl: 10_000,  // rewards last long enough to claim
                total_staked: 93_714,
                created_epoch: 0,
                stakers: vec![
                    Staker { address: f(GENESIS_VALIDATOR1), amount: 48_237, staked_epoch: 0, pending_rewards: 0, last_claim_epoch: 0, total_claimed: 0, total_decayed: 0 },
                    Staker { address: f(GENESIS_VALIDATOR2), amount: 31_492, staked_epoch: 0, pending_rewards: 0, last_claim_epoch: 0, total_claimed: 0, total_decayed: 0 },
                    Staker { address: f(GENESIS_FOUNDATION), amount: 13_985, staked_epoch: 0, pending_rewards: 0, last_claim_epoch: 0, total_claimed: 0, total_decayed: 0 },
                ],
            },
        ],
        next_id: 2,
    }
}

// ──────────────────────────── Genesis DAO ────────────────────────────────

fn initialize_dao_store() -> DAOStore {
    use api::{GENESIS_FOUNDATION, GENESIS_CORE_DEV, GENESIS_VALIDATOR1, GENESIS_VALIDATOR2, GENESIS_ECOSYSTEM, GENESIS_COMMUNITY};
    let f = |h: &str| format!("0x{}", h);

    DAOStore {
        proposals: vec![
            DAOProposal {
                id: 1, title: "Increase base reward rate to 150 EVAP/epoch".into(),
                description: "The current reward rate of 100 EVAP/epoch is insufficient to incentivize early validators. This proposal increases it to 150.".into(),
                options: vec!["For".into(), "Against".into(), "Abstain".into()],
                votes: vec![
                    DAOVote { voter: f(GENESIS_VALIDATOR1), option: "For".into(), weight: 48_237, epoch: 5 },
                    DAOVote { voter: f(GENESIS_VALIDATOR2), option: "For".into(), weight: 31_492, epoch: 8 },
                    DAOVote { voter: f(GENESIS_FOUNDATION), option: "Against".into(), weight: 12_841, epoch: 12 },
                    DAOVote { voter: f(GENESIS_COMMUNITY), option: "Abstain".into(), weight: 3_917, epoch: 15 },
                ],
                created_epoch: 0, voting_period: 50_000, creator: f(GENESIS_VALIDATOR1),
                status: "Active".into(), evaporated_epoch: None,
            },
            DAOProposal {
                id: 2, title: "Fund ecosystem grants program".into(),
                description: "Allocate 100,000 EVAP from the treasury to a grants program for developers building on EvaporChain.".into(),
                options: vec!["For".into(), "Against".into(), "Abstain".into()],
                votes: vec![
                    DAOVote { voter: f(GENESIS_FOUNDATION), option: "For".into(), weight: 97_412, epoch: 3 },
                    DAOVote { voter: f(GENESIS_CORE_DEV), option: "For".into(), weight: 46_823, epoch: 6 },
                    DAOVote { voter: f(GENESIS_ECOSYSTEM), option: "Against".into(), weight: 8_271, epoch: 9 },
                ],
                created_epoch: 0, voting_period: 50_000, creator: f(GENESIS_FOUNDATION),
                status: "Active".into(), evaporated_epoch: None,
            },
            DAOProposal {
                id: 3, title: "Reduce minimum object energy to 5".into(),
                description: "Lower the minimum energy for state objects from 10 to 5 to enable more lightweight ephemeral use cases.".into(),
                options: vec!["For".into(), "Against".into(), "Abstain".into()],
                votes: vec![
                    DAOVote { voter: f(GENESIS_ECOSYSTEM), option: "For".into(), weight: 18_493, epoch: 2 },
                    DAOVote { voter: f(GENESIS_COMMUNITY), option: "Abstain".into(), weight: 4_217, epoch: 4 },
                    DAOVote { voter: f(GENESIS_CORE_DEV), option: "For".into(), weight: 23_814, epoch: 7 },
                ],
                created_epoch: 0, voting_period: 100, creator: f(GENESIS_ECOSYSTEM),
                status: "Active".into(), evaporated_epoch: None,  // this one expires fast — demo
            },
            DAOProposal {
                id: 4, title: "Emergency: Patch state root vulnerability".into(),
                description: "Critical fix for a state root computation edge case that could allow replay attacks.".into(),
                options: vec!["For".into(), "Against".into()],
                votes: vec![
                    DAOVote { voter: f(GENESIS_FOUNDATION), option: "For".into(), weight: 193_847, epoch: 1 },
                    DAOVote { voter: f(GENESIS_VALIDATOR1), option: "For".into(), weight: 48_237, epoch: 1 },
                    DAOVote { voter: f(GENESIS_CORE_DEV), option: "For".into(), weight: 41_293, epoch: 2 },
                ],
                created_epoch: 0, voting_period: 10, creator: f(GENESIS_FOUNDATION),
                status: "Passed:For".into(), evaporated_epoch: Some(10),
            },
        ],
        next_id: 5,
    }
}

// ──────────────────────────── Main ───────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Quick exit for --bench mode
    if std::env::args().any(|a| a == "--bench") {
        bench::run_benchmarks();
        return Ok(());
    }

    // Initialize structured logging
    let json_log = std::env::args().any(|a| a == "--json-log");
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    if json_log {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .with_target(true)
            .with_thread_ids(true)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .init();
    }

    let args = parse_args();
    let node_tag = make_tag(&args.node_id);

    print_banner(&node_tag);

    // ── Persistent storage ──
    let state_path = format!("{}/state", args.data_dir);
    let chain_path = format!("{}/chain", args.data_dir);
    println!("{} \x1b[1mData directory:\x1b[0m {}", node_tag, args.data_dir);

    let state_db = RocksDBStateDB::open(&state_path)
        .expect("Failed to open RocksDB state database");
    let chain_store = Arc::new(ChainStore::open(&chain_path)
        .expect("Failed to open ChainStore"));
    let is_fresh = !state_db.has_data();

    let db = Arc::new(Mutex::new(state_db));

    if is_fresh {
        let mut db = safe_lock(&db);
        if let Some(ref genesis_path) = args.genesis_config {
            println!("{} \x1b[1mFresh start — loading genesis from config: {}\x1b[0m", node_tag, genesis_path);
            let json = std::fs::read_to_string(genesis_path)
                .unwrap_or_else(|e| panic!("Failed to read genesis config {}: {}", genesis_path, e));
            let config = evaporchain_execution::genesis::load_genesis_config(&json)
                .unwrap_or_else(|e| panic!("Invalid genesis config: {}", e));
            let result = evaporchain_execution::genesis::initialize_genesis(&mut *db, &config)
                .unwrap_or_else(|e| panic!("Genesis initialization failed: {}", e));
            println!("{} \x1b[1;32mGenesis block #{} created\x1b[0m — {} accounts, {} validators, state_root={}",
                node_tag, result.block.number, result.accounts_created, result.validators_registered,
                hex::encode(&result.state_root[..8]));
            if args.demo_mode {
                seed_demo_accounts(&mut *db, &node_tag);
            }
        } else {
            println!("{} \x1b[1mFresh start — loading genesis state:\x1b[0m", node_tag);
            initialize_genesis(&mut db, &node_tag);
        }
    } else {
        println!("{} \x1b[1;32mResuming from persistent state\x1b[0m", node_tag);
        let db = safe_lock(&db);
        println!(
            "{}   {} accounts, {} objects, {} ghosts loaded from disk",
            node_tag,
            db.all_account_addresses().len(),
            db.object_count(),
            db.ghost_count(),
        );
    }
    println!();

    // ── Prover setup ──
    // Wrap the proving engine in a ChainProver for checkpointing, chain proof
    // generation, and light-client support.  Checkpoint every 100 blocks.
    let genesis_state_root = {
        let mut db_guard = safe_lock(&db);
        db_guard.compute_state_root()
    };
    let chain_prover: Arc<Mutex<ChainProver>> = if args.prove_mode {
        #[cfg(feature = "prove")]
        {
            println!(
                "{} \x1b[1;33mProving mode active\x1b[0m — setting up Nova IVC (real blocks)...",
                node_tag
            );
            let genesis_commitment = {
                let db = safe_lock(&db);
                evaporchain_types::DualCommitment {
                    verkle_root: genesis_state_root,
                    mmr_root: [0u8; 32],
                    epoch: 0,
                    active_count: db.object_count(),
                    ghost_count: db.ghost_count(),
                }
            };
            let real_prover = evaporchain_proving::nova::RealBlockProver::new(&genesis_commitment)
                .expect("Failed to set up RealBlockProver");
            let (primary, secondary) = real_prover.num_constraints();
            println!(
                "{}   Nova ready (real blocks): {} primary, {} secondary constraints",
                node_tag, primary, secondary
            );
            Arc::new(Mutex::new(ChainProver::new(
                Box::new(real_prover) as Box<dyn ProvingEngine>,
                genesis_state_root,
                100, // checkpoint every 100 blocks
            )))
        }
        #[cfg(not(feature = "prove"))]
        {
            eprintln!("\x1b[31m--prove requires the 'prove' feature. Recompile with: cargo build -p evaporchain-node --features prove\x1b[0m");
            std::process::exit(1);
        }
    } else {
        Arc::new(Mutex::new(ChainProver::new(
            Box::new(MockProver::new()) as Box<dyn ProvingEngine>,
            genesis_state_root,
            100,
        )))
    };

    // ── Network setup ──
    let mut peer_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let net_channels = if args.network_mode {
        let peer_authority = if args.allowed_peers.is_empty() {
            evaporchain_network::PeerAuthority::permissionless()
        } else {
            let peer_ids: Vec<evaporchain_network::PeerId> = args
                .allowed_peers
                .iter()
                .filter_map(|s| s.parse::<evaporchain_network::PeerId>().ok())
                .collect();
            println!(
                "{} \x1b[1;36mPeer allowlist\x1b[0m — {} authorized peers",
                node_tag,
                peer_ids.len()
            );
            evaporchain_network::PeerAuthority::with_allowlist(peer_ids)
        };

        if args.use_tls {
            println!("{} \x1b[1;32mTLS 1.3 transport enabled\x1b[0m", node_tag);
        }

        let net_config = NetworkConfig {
            listen_address: format!("/ip4/0.0.0.0/tcp/{}", args.port),
            bootstrap_peers: args.bootstrap_peers.clone(),
            channel_buffer: 256,
            use_tls: args.use_tls,
            tls_certs: None,
            peer_authority,
        };
        println!(
            "{} \x1b[1;33mNetwork mode active\x1b[0m — listening on port {}, {} bootstrap peer(s)",
            node_tag,
            if args.port == 0 {
                "random".to_string()
            } else {
                args.port.to_string()
            },
            args.bootstrap_peers.len()
        );
        let (channels, _handle, peer_id) =
            P2pNetworkService::start(net_config).await.map_err(|e| {
                anyhow::anyhow!("Failed to start network: {}", e)
            })?;
        println!(
            "{} \x1b[36mPeer ID: {}\x1b[0m",
            node_tag,
            peer_id
        );
        // Use the network's live peer count
        peer_count = channels.peer_count.clone();
        Some(channels)
    } else {
        None
    };

    // Determine role
    let is_producer = args.demo_mode || !args.network_mode;

    if args.high_throughput {
        println!(
            "{} \x1b[1;33mHigh-throughput mode\x1b[0m — gas_limit={} interval={}ms (~{} transfers/block)",
            node_tag, args.block_gas_limit, args.block_ms,
            args.block_gas_limit / 21_000,
        );
    }

    if args.demo_mode {
        println!(
            "{} \x1b[1;33mDemo mode active\x1b[0m — auto-generating transactions",
            node_tag
        );
    } else if !args.network_mode {
        println!(
            "{} Submit transactions via stdin as JSON (one per line).",
            node_tag
        );
    }

    let role_str = if args.tendermint_mode {
        format!("Validator-{} (Tendermint BFT)", args.validator_id)
    } else if is_producer {
        "Producer".to_string()
    } else {
        "Follower".to_string()
    };
    println!(
        "{} Role: {} | Block interval: {}ms | Grace: {} epochs | Proving: {} | Network: {} | Consensus: {}",
        node_tag,
        role_str,
        args.block_ms,
        GRACE_PERIOD,
        if args.prove_mode {
            "Nova IVC (real blocks)"
        } else {
            "Mock"
        },
        if args.network_mode { "ON" } else { "OFF" },
        if args.tendermint_mode { "Tendermint BFT" } else { "MockConsensus" }
    );
    println!(
        "{} \x1b[90m──────────────────────────────────────────────────────────────\x1b[0m",
        node_tag
    );

    // ── Startup delay (wait for mDNS peer discovery) ──
    if args.startup_delay_ms > 0 {
        println!(
            "{} Waiting {}ms for peer discovery...",
            node_tag, args.startup_delay_ms
        );
        tokio::time::sleep(Duration::from_millis(args.startup_delay_ms)).await;
        let peers = peer_count.load(std::sync::atomic::Ordering::Relaxed);
        println!(
            "{} Discovery complete — {} peer(s) connected",
            node_tag, peers
        );
    }

    // ── Shared consensus ──
    let consensus = Arc::new(Mutex::new(MockConsensus::new_with_gas_limit(GRACE_PERIOD, args.block_gas_limit)));

    // Build Tendermint consensus if enabled
    let tendermint = if args.tendermint_mode {
        let mut validators = Vec::new();
        for vid in 0..args.validator_count {
            let mut address = [0u8; 32];
            address[0] = (vid + 1) as u8;
            validators.push(ValidatorInfo::new(vid, args.validator_stake, address));
        }
        let vs = ValidatorSet::with_validators(validators);
        let mut tc = TendermintConsensus::new_with_gas_limit(args.validator_id, GRACE_PERIOD, vs, args.block_gas_limit);
        // Inject Nova proof verifier into consensus
        tc.set_proof_verifier(
            Box::new(ChainProofVerifier {
                prover: chain_prover.clone(),
            }),
            genesis_state_root,
        );
        // Load or generate BLS12-381 keypair (persisted to data dir)
        let bls_key_path = format!("{}/bls_key.bin", args.data_dir);
        let bls_kp = if let Ok(secret_bytes) = std::fs::read(&bls_key_path) {
            if secret_bytes.len() == 32 {
                match evaporchain_crypto::signatures::BlsKeypair::from_secret_bytes(&secret_bytes) {
                    Ok(kp) => {
                        println!(
                            "{} \x1b[1;36mBLS12-381 keypair loaded from disk\x1b[0m (pk={}B)",
                            node_tag, kp.public_key_bytes().0.len()
                        );
                        kp
                    }
                    Err(e) => {
                        eprintln!("{} BLS key file corrupt ({}), regenerating", node_tag, e);
                        let kp = evaporchain_crypto::signatures::BlsKeypair::generate();
                        let _ = std::fs::write(&bls_key_path, kp.secret_key_bytes().as_bytes());
                        kp
                    }
                }
            } else {
                eprintln!("{} BLS key file wrong size ({}B), regenerating", node_tag, secret_bytes.len());
                let kp = evaporchain_crypto::signatures::BlsKeypair::generate();
                let _ = std::fs::write(&bls_key_path, kp.secret_key_bytes().as_bytes());
                kp
            }
        } else {
            let kp = evaporchain_crypto::signatures::BlsKeypair::generate();
            let _ = std::fs::write(&bls_key_path, kp.secret_key_bytes().as_bytes());
            println!(
                "{} \x1b[1;36mBLS12-381 keypair generated & saved\x1b[0m (pk={}B)",
                node_tag, kp.public_key_bytes().0.len()
            );
            kp
        };
        tc.set_bls_keypair(bls_kp);
        println!(
            "{} \x1b[1;35mTendermint BFT consensus\x1b[0m — validator_id={}, validators={}, stake={}, BLS=enabled",
            node_tag, args.validator_id, args.validator_count, args.validator_stake
        );
        Some(Arc::new(Mutex::new(tc)))
    } else {
        None
    };

    // Restore consensus state from disk if available
    if !is_fresh {
        if let Some((block_number, epoch, parent_hash)) = chain_store.load_consensus_meta() {
            if let Some(ref tc) = tendermint {
                let mut c = safe_lock(&tc);
                c.restore_state(block_number, epoch, parent_hash);
                println!(
                    "{} \x1b[1;32mTendermint restored:\x1b[0m block={}, epoch={}, parent_hash={}…",
                    node_tag, block_number, epoch, &hex::encode(parent_hash)[..16]
                );
            } else {
                let mut c = safe_lock(&consensus);
                c.restore_state(block_number, epoch, parent_hash);
                println!(
                    "{} \x1b[1;32mConsensus restored:\x1b[0m block={}, epoch={}, parent_hash={}…",
                    node_tag, block_number, epoch, &hex::encode(parent_hash)[..16]
                );
            }
        }
    }

    // Restore mempool from disk
    if !is_fresh {
        let saved_txs = chain_store.load_mempool();
        if !saved_txs.is_empty() {
            if let Some(ref tc) = tendermint {
                let mut c = safe_lock(&tc);
                for tx in &saved_txs {
                    c.mempool.submit(tx.clone());
                }
            } else {
                let mut c = safe_lock(&consensus);
                for tx in &saved_txs {
                    c.mempool.submit(tx.clone());
                }
            }
            println!("{} \x1b[32mRestored {} transactions from mempool\x1b[0m", node_tag, saved_txs.len());
        }
    }

    // ── API shared state ──
    let block_history: Arc<Mutex<VecDeque<BlockRecord>>> = Arc::new(Mutex::new(
        if is_fresh {
            VecDeque::with_capacity(500)
        } else {
            let history = chain_store.load_block_history(500);
            println!("{} \x1b[32mRestored {} blocks from disk\x1b[0m", node_tag, history.len());
            history
        }
    ));
    let chain_stats: Arc<Mutex<ChainStats>> = Arc::new(Mutex::new(
        if is_fresh {
            ChainStats::new()
        } else {
            chain_store.load_chain_stats().unwrap_or_else(ChainStats::new)
        }
    ));
    let throughput: Arc<Mutex<ThroughputTracker>> = Arc::new(Mutex::new(ThroughputTracker::new()));
    let events: Arc<Mutex<VecDeque<EventRecord>>> = Arc::new(Mutex::new(
        if is_fresh {
            VecDeque::with_capacity(200)
        } else {
            chain_store.load_events()
        }
    ));
    let start_time = Instant::now();
    let ws_broadcaster = Arc::new(ws::WsBroadcaster::new(1024));

    // DA shard store: block_number -> BlockDAPackage (keep last 64 blocks)
    let restored_da = chain_store.load_recent_da_packages(64);
    let da_restored_count = restored_da.len();
    let da_store: Arc<Mutex<BTreeMap<u64, BlockDAPackage>>> = Arc::new(Mutex::new(restored_da));
    if da_restored_count > 0 {
        println!("{} \x1b[36mDA: restored {} shard packages from disk\x1b[0m", node_tag, da_restored_count);
    }

    // ── Oracle + Sharding Bridges ──
    let oracle_bridge: Arc<Mutex<oracle_bridge::OracleBridge>> = Arc::new(Mutex::new(
        oracle_bridge::OracleBridge::new(if args.validator_count > 0 {
            ((2 * args.validator_count as usize) / 3) + 1
        } else {
            1
        }),
    ));
    let shard_bridge: Arc<Mutex<shard_bridge::ShardBridge>> = Arc::new(Mutex::new(
        shard_bridge::ShardBridge::new(16),
    ));

    // ── Frontier Primitives ──
    // Energy-Annotated Verkle Trie + PoHA + Anchor-based consensus
    let frontier_state: Arc<Mutex<frontier::FrontierState>> = Arc::new(Mutex::new(
        frontier::FrontierState::new(frontier::FrontierConfig::default()),
    ));
    {
        let mut fs = safe_lock(&frontier_state);
        let poha_loaded = chain_store.load_poha_state(&mut fs.poha);
        if poha_loaded > 0 {
            println!("{} \x1b[35mPoHA: restored {} certs/ghosts from disk\x1b[0m", node_tag, poha_loaded);
        }
    }
    println!(
        "{} \x1b[1;35mFrontier primitives active\x1b[0m — anchors(every 100), PoHA, energy-trie",
        node_tag,
    );

    // Wire anchor hash provider into Tendermint consensus
    if let Some(ref tc) = tendermint {
        let mut c = safe_lock(tc);
        c.set_anchor_provider(Box::new(FrontierAnchorProvider {
            frontier: frontier_state.clone(),
        }));
    }

    // Restore deployed contracts from disk
    if !is_fresh {
        if let Some(ref tc) = tendermint {
            let mut c = safe_lock(tc);
            let scripts = chain_store.load_all_script_contracts();
            let templates = chain_store.load_all_template_contracts();
            for sc in scripts.iter() {
                c.script_engine_mut().restore_contract(sc.clone());
            }
            for ti in templates.iter() {
                c.contract_engine_mut().restore_contract(ti.clone());
            }
            if !scripts.is_empty() || !templates.is_empty() {
                println!(
                    "{} \x1b[32mContracts restored: {} scripts, {} templates\x1b[0m",
                    node_tag, scripts.len(), templates.len()
                );
            }
        }
    }

    // Channel for API-submitted transactions to reach P2P network & all mempools
    let (api_tx_sender, mut api_tx_receiver) = tokio::sync::mpsc::channel::<Transaction>(256);

    // Snapshot info — shared between API server and commit loop
    let snapshot_info: Arc<Mutex<Option<(u64, [u8; 32], usize)>>> =
        Arc::new(Mutex::new(None));

    // ── API server ──
    if args.api_mode {
        // Initialize user database for wallet/auth system
        let user_db = Arc::new(
            user_db::UserDb::open("evaporchain_users.db")
                .expect("Failed to open user database"),
        );
        let auth_state = Arc::new(auth::AuthState {
            user_db,
            sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
            login_rate_limit: Mutex::new(std::collections::HashMap::new()),
            register_rate_limit: Mutex::new((0, std::time::Instant::now())),
        });

        // Initialize or restore DeFi stores
        let (nft_store, token_store, staking_store, dao_store) = if is_fresh {
            println!("{} \x1b[35mDeFi modules: 6 NFTs, 3 tokens, 1 staking pool, 4 DAO proposals\x1b[0m", node_tag);
            let ns = initialize_nft_store();
            let ts = initialize_token_store();
            let ss = initialize_staking_store();
            let ds = initialize_dao_store();
            // Persist genesis DeFi stores
            log_persist_err("nft_store", chain_store.save_nft_store(&ns));
            log_persist_err("token_store", chain_store.save_token_store(&ts));
            log_persist_err("staking_store", chain_store.save_staking_store(&ss));
            log_persist_err("dao_store", chain_store.save_dao_store(&ds));
            (Arc::new(Mutex::new(ns)), Arc::new(Mutex::new(ts)), Arc::new(Mutex::new(ss)), Arc::new(Mutex::new(ds)))
        } else {
            let ns = chain_store.load_nft_store().unwrap_or_else(|| initialize_nft_store());
            let ts = chain_store.load_token_store().unwrap_or_else(|| initialize_token_store());
            let ss = chain_store.load_staking_store().unwrap_or_else(|| initialize_staking_store());
            let ds = chain_store.load_dao_store().unwrap_or_else(|| initialize_dao_store());
            println!("{} \x1b[32mDeFi stores restored: {} NFTs, {} tokens, {} staking pools, {} proposals\x1b[0m",
                node_tag, ns.tokens.len(), ts.tokens.len(), ss.pools.len(), ds.proposals.len());
            (Arc::new(Mutex::new(ns)), Arc::new(Mutex::new(ts)), Arc::new(Mutex::new(ss)), Arc::new(Mutex::new(ds)))
        };

        // Generate node-level ML-DSA keypair for signing API-submitted transactions
        let node_keypair = Arc::new(evaporchain_crypto::signatures::MlDsaKeypair::generate());
        println!("{} \x1b[1;36mNode signing keypair generated\x1b[0m (ML-DSA Dilithium3, pk={}B)",
            node_tag, node_keypair.public_key().len());

        let api_state = Arc::new(ApiState {
            db: Arc::clone(&db),
            consensus: Arc::clone(&consensus),
            peer_count: Arc::clone(&peer_count),
            block_history: Arc::clone(&block_history),
            stats: Arc::clone(&chain_stats),
            events: Arc::clone(&events),
            prove_mode: args.prove_mode,
            start_time,
            faucet_rate_limit: std::sync::Mutex::new(std::collections::HashMap::new()),
            nft_store,
            token_store,
            staking_store,
            dao_store,
            auth_sessions: Some(Arc::clone(&auth_state.sessions)),
            user_db: Some(Arc::clone(&auth_state.user_db)),
            node_keypair,
            tendermint: tendermint.as_ref().map(Arc::clone),
            tx_broadcast: Some(api_tx_sender.clone()),
            chain_prover: Arc::clone(&chain_prover),
            throughput: Arc::clone(&throughput),
            da_store: Arc::clone(&da_store),
            snapshot_info: Arc::clone(&snapshot_info),
            frontier_state: Some(Arc::clone(&frontier_state)),
            oracle_bridge: Some(Arc::clone(&oracle_bridge)),
            shard_bridge: Some(Arc::clone(&shard_bridge)),
            ws_broadcaster: Arc::clone(&ws_broadcaster),
            chain_store: Some(Arc::clone(&chain_store)),
        });
        let api_port = args.api_port;
        tokio::spawn(async move {
            if let Err(e) = api::start_api_server(api_state, auth_state, api_port).await {
                eprintln!("\x1b[31mAPI server error: {}\x1b[0m", e);
            }
        });
        println!(
            "{} \x1b[1;33mAPI mode active\x1b[0m — dashboard on http://localhost:{}",
            node_tag, args.api_port
        );
    }

    // ── Stdin reader (non-demo, non-follower) ──
    if !args.demo_mode && is_producer {
        let consensus_tx = Arc::clone(&consensus);
        let tag = node_tag.clone();
        tokio::task::spawn_blocking(move || {
            let stdin_keypair = MlDsaKeypair::generate();
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                match line {
                    Ok(line) => {
                        if let Some(tx) = parse_stdin_command(&line, &stdin_keypair) {
                            let mut c = safe_lock(&consensus_tx);
                            c.mempool.submit(tx);
                            println!(
                                "{} \x1b[90m→ transaction queued (mempool={})\x1b[0m",
                                tag,
                                c.mempool.len()
                            );
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // Split network channels
    let (
        net_tx_sender,
        mut net_tx_receiver,
        net_block_sender,
        mut net_block_receiver,
        block_cache,
        sync_request_sender,
        mut sync_blocks_receiver,
        mut tip_receiver,
        consensus_net_sender,
        mut consensus_net_receiver,
        mut sample_request_sender,
        mut sample_response_receiver,
        shard_cache,
    ) = if let Some(ch) = net_channels {
        (
            Some(ch.tx_sender),
            Some(ch.tx_receiver),
            Some(ch.block_sender),
            Some(ch.block_receiver),
            Some(ch.block_cache),
            Some(ch.sync_request_sender),
            Some(ch.sync_blocks_receiver),
            Some(ch.tip_receiver),
            Some(ch.consensus_sender),
            Some(ch.consensus_receiver),
            Some(ch.sample_request_sender),
            Some(ch.sample_response_receiver),
            Some(ch.shard_cache),
        )
    } else {
        (None, None, None, None, None, None, None, None, None, None, None, None, None)
    };

    // Broadcast our BLS KeyAnnounce to peers once network is ready
    if let Some(ref tc_ref) = tendermint {
        let tc = safe_lock(tc_ref);
        if let Some(key_msg) = tc.make_key_announce() {
            if let Some(ref sender) = consensus_net_sender {
                if let Ok(data) = serde_json::to_vec(&key_msg) {
                    let _ = sender.try_send(data);
                    println!("{} \x1b[1;36mBLS KeyAnnounce broadcast to peers\x1b[0m", node_tag);
                }
            }
        }
    }

    // Block queue for out-of-order blocks (gap filling)
    let mut pending_blocks: BTreeMap<u64, evaporchain_types::Block> = BTreeMap::new();
    // Track whether we have an outstanding sync request to avoid duplicate requests
    let mut sync_in_flight = false;

    // ── Populate block cache from persistence (enables sync after restart) ──
    if let Some(ref cache) = block_cache {
        let recent = chain_store.load_recent_full_blocks(2000);
        if !recent.is_empty() {
            let count = recent.len();
            let min_h = recent.first().map(|b| b.number).unwrap_or(0);
            let max_h = recent.last().map(|b| b.number).unwrap_or(0);
            for block in &recent {
                cache_block(cache, block);
            }
            println!(
                "{} \x1b[36mPopulated block cache from persistence: {} blocks ({}..{})\x1b[0m",
                node_tag, count, min_h, max_h
            );
        }
    }

    // ── Generate demo signing keypairs (ML-DSA / Dilithium3) ──
    let demo_keypairs: [MlDsaKeypair; 6] = if args.demo_mode {
        println!("{} \x1b[36mGenerating 6 ML-DSA keypairs for demo signatures...\x1b[0m", node_tag);
        let kps = [
            MlDsaKeypair::generate(),
            MlDsaKeypair::generate(),
            MlDsaKeypair::generate(),
            MlDsaKeypair::generate(),
            MlDsaKeypair::generate(),
            MlDsaKeypair::generate(),
        ];
        println!("{} \x1b[32m✓ ML-DSA signature verification ENABLED — all transactions must be signed\x1b[0m", node_tag);
        kps
    } else {
        // Non-demo: still need the array but it won't be used
        [
            MlDsaKeypair::generate(),
            MlDsaKeypair::generate(),
            MlDsaKeypair::generate(),
            MlDsaKeypair::generate(),
            MlDsaKeypair::generate(),
            MlDsaKeypair::generate(),
        ]
    };

    // ── Helper: apply a single block (follower path) ──
    // Returns (obj_count, ghost_count) on success
    fn apply_follower_block(
        node_tag: &str,
        block: &evaporchain_types::Block,
        consensus: &Arc<Mutex<MockConsensus>>,
        db: &Arc<Mutex<RocksDBStateDB>>,
        prover: &Arc<Mutex<ChainProver>>,
        prove_mode: bool,
        block_history: &Arc<Mutex<VecDeque<BlockRecord>>>,
        chain_stats: &Arc<Mutex<ChainStats>>,
        events: &Arc<Mutex<VecDeque<EventRecord>>>,
        throughput: &Arc<Mutex<ThroughputTracker>>,
        chain_store: &Arc<ChainStore>,
        peer_count: &Arc<std::sync::atomic::AtomicUsize>,
        block_cache: &Option<evaporchain_network::service::BlockCache>,
        tendermint: &Option<Arc<Mutex<TendermintConsensus>>>,
        ws_broadcaster: &Arc<ws::WsBroadcaster>,
    ) -> Option<(usize, usize)> {
        // Verify CommitCertificate before applying (lenient if BLS keys not yet received)
        if let Some(ref cert) = block.commit_certificate {
            if let Some(ref tc_ref) = tendermint {
                let tc = safe_lock(tc_ref);
                let has_all_keys = cert.signer_ids.iter().all(|&vid| {
                    tc.validator_set().get(vid)
                        .map_or(false, |v| v.bls_public_key.is_some())
                });
                if has_all_keys && !tc.verify_commit_certificate(cert) {
                    eprintln!(
                        "{} \x1b[31m⚠ REJECTED block #{} — invalid BLS CommitCertificate\x1b[0m",
                        node_tag, block.number
                    );
                    return None;
                }
            }
        }

        let exec_start = Instant::now();
        let mut c = safe_lock(&consensus);
        let mut db_guard = safe_lock(&db);

        // Only apply if this block advances our chain
        if block.number <= c.block_number() {
            return None; // stale
        }

        match c.apply_block(&mut *db_guard, block) {
            Ok(result) => {
                db_guard.flush_accounts();
                db_guard.flush_objects();

                let mut p = safe_lock(&prover);
                match p.fold_block(&result.block, result.execution.state_root) {
                    Ok(fold_res) if prove_mode => {
                        println!(
                            "{}   \x1b[35mProof: fold={:.1}ms  acc={}B  folded={}\x1b[0m",
                            node_tag,
                            fold_res.fold_time_us as f64 / 1000.0,
                            fold_res.accumulator_size,
                            p.blocks_folded(),
                        );
                    }
                    Err(e) => eprintln!("{} \x1b[31mProving error: {}\x1b[0m", node_tag, e),
                    _ => {}
                }
                drop(p);

                let obj_count = db_guard.object_count();
                let ghost_count_val = db_guard.ghost_count();
                let peers = peer_count.load(std::sync::atomic::Ordering::Relaxed);

                let exec_elapsed_us = exec_start.elapsed().as_micros() as u64;
                record_block(
                    block_history, chain_stats, events, throughput,
                    Some(&ws_broadcaster),
                    &result.block, &result.execution,
                    obj_count, ghost_count_val, exec_elapsed_us,
                );

                log_persist_err("consensus_meta", chain_store.save_consensus_meta(
                    result.block.number, result.block.epoch, result.block.parent_hash,
                ));
                log_persist_err("full_block", chain_store.save_full_block(&result.block));
                log_persist_err("tx_index", chain_store.index_block_transactions(&result.block).map(|_| ()));
                index_contract_events_from_exec(&chain_store, &result.block, &result.execution);
                {
                    let history = safe_lock(&block_history);
                    if let Some(record) = history.back() {
                        log_persist_err("block", chain_store.save_block(record));
                    }
                }
                {
                    let stats = safe_lock(&chain_stats);
                    log_persist_err("chain_stats", chain_store.save_chain_stats(&stats));
                }
                {
                    let ev = safe_lock(&events);
                    log_persist_err("events", chain_store.save_events(&ev));
                }
                // Persist mempool
                {
                    let pending: Vec<evaporchain_types::Transaction> = if let Some(ref tc_ref) = tendermint {
                        let c = safe_lock(tc_ref);
                        c.mempool.pending().iter().cloned().collect()
                    } else {
                        let c = safe_lock(consensus);
                        c.mempool.pending().iter().cloned().collect()
                    };
                    log_persist_err("mempool", chain_store.save_mempool(&pending));
                }
                persist_contracts(&chain_store, &tendermint);

                // Cache block for serving to peers
                if let Some(ref cache) = block_cache {
                    cache_block(cache, block);
                }

                let roots_match = result.execution.state_root == block.state_root;
                if !roots_match {
                    eprintln!(
                        "{} \x1b[31m⚠ STATE ROOT MISMATCH! local={} remote={}\x1b[0m",
                        node_tag,
                        &hex::encode(result.execution.state_root)[..16],
                        &hex::encode(block.state_root)[..16],
                    );
                }

                print_block_result(
                    node_tag,
                    if roots_match { "SYNCED ✓" } else { "SYNCED ✗" },
                    result.block.number,
                    result.block.epoch,
                    result.execution.txs_executed,
                    result.execution.txs_failed,
                    result.execution.objects_entered_grace,
                    result.execution.objects_evaporated,
                    obj_count,
                    ghost_count_val,
                    &result.execution.state_root,
                    peers,
                );

                Some((obj_count, ghost_count_val))
            }
            Err(e) => {
                eprintln!(
                    "{} \x1b[31mFailed to apply block #{}: {}\x1b[0m",
                    node_tag, block.number, e
                );
                None
            }
        }
    }

    // ── Helper: broadcast consensus actions to network ──
    async fn broadcast_consensus_actions(
        actions: Vec<ConsensusAction>,
        consensus_sender: &Option<mpsc::Sender<Vec<u8>>>,
        _node_tag: &str,
    ) -> Vec<ConsensusAction> {
        let mut commit_actions = Vec::new();
        for action in actions {
            match action {
                ConsensusAction::BroadcastMessage(ref msg) => {
                    if let Some(ref sender) = consensus_sender {
                        if let Ok(data) = serde_json::to_vec(msg) {
                            let _ = sender.send(data).await;
                        }
                    }
                }
                ConsensusAction::CommitBlock(_) => {
                    commit_actions.push(action);
                }
                ConsensusAction::RequestSync(from, to) => {
                    tracing::info!("State sync requested: height {} -> {}", from, to);
                    // State sync is handled via the sync API endpoints;
                    // the node will initiate sync on the next tick.
                }
            }
        }
        commit_actions
    }

    // ── Block production / follower loop ──
    let mut ticker = interval(Duration::from_millis(args.block_ms));
    let mut rng = rand::thread_rng();
    let mut demo_nonces = [0u64; 4];

    // Tendermint consensus tick interval (faster than block interval)
    let mut consensus_ticker = interval(Duration::from_millis(100));

    loop {
        tokio::select! {
            // ── Tendermint consensus tick ──
            _ = consensus_ticker.tick(), if tendermint.is_some() => {
                let tc_ref = tendermint.as_ref().unwrap();

                // Drain network txs into mempool
                if let Some(ref mut rx) = net_tx_receiver {
                    while let Ok(tx) = rx.try_recv() {
                        let mut tc = safe_lock(&tc_ref);
                        tc.mempool.submit(tx);
                    }
                }

                // Drain API-submitted txs and broadcast via P2P to other validators.
                // (The API handler already added these to the local mempool.)
                while let Ok(tx) = api_tx_receiver.try_recv() {
                    if let Some(ref sender) = net_tx_sender {
                        let _ = sender.try_send(tx);
                    }
                }

                // Generate demo txs only when we're the proposer (avoids stale nonce accumulation)
                if args.demo_mode {
                    let (epoch, is_proposer) = {
                        let tc = safe_lock(&tc_ref);
                        (tc.epoch() + 1, tc.am_i_proposer())
                    };
                    if is_proposer {
                        if let Some(tx) = generate_demo_tx(&mut rng, epoch, &mut demo_nonces, &demo_keypairs, args.validator_id, args.validator_count, &db) {
                            let mut tc = safe_lock(&tc_ref);
                            tc.mempool.submit(tx);
                        }
                        // Submit oracle votes for demo price feeds
                        if epoch % 10 == 0 {
                            let mut ob = safe_lock(&oracle_bridge);
                            let ts = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            for (key, base_price) in &[("btc_usd", 60000.0f64), ("eth_usd", 3000.0), ("evap_usd", 0.50)] {
                                let round_id = ob.start_round(key);
                                let jitter = (rng.gen::<f64>() - 0.5) * base_price * 0.02;
                                let vote = evaporchain_oracle::consensus::make_vote(
                                    args.validator_id, key, base_price + jitter, round_id, ts,
                                );
                                let _ = ob.submit_vote(key, vote);
                            }
                        }
                    }
                }

                // Tick the consensus state machine
                let actions = {
                    let mut tc = safe_lock(&tc_ref);
                    let mut db_guard = safe_lock(&db);
                    let phase = tc.phase();
                    let height = tc.height();
                    let round = tc.round();
                    let actions = tc.tick(&mut *db_guard);
                    if !actions.is_empty() {
                        eprintln!(
                            "{} [consensus] h={} r={} phase={:?} -> {} action(s)",
                            node_tag, height, round, phase, actions.len()
                        );
                    }
                    actions
                };

                // Process actions
                let mut commits = broadcast_consensus_actions(
                    actions, &consensus_net_sender, &node_tag,
                ).await;

                // Handle commits
                for action in commits.drain(..) {
                    if let ConsensusAction::RequestSync(from, to) = action {
                        if !sync_in_flight {
                            println!(
                                "{} \x1b[36mConsensus requests sync: blocks {}..{}\x1b[0m",
                                node_tag, from, to
                            );
                            if let Some(ref sender) = sync_request_sender {
                                let _ = sender.send((from, to)).await;
                                sync_in_flight = true;
                            }
                        }
                        continue;
                    }
                    if let ConsensusAction::CommitBlock(mut block) = action {
                        // Execute the block to get state root
                        let exec_start = Instant::now();
                        let result = {
                            let mut tc = safe_lock(&tc_ref);
                            let mut db_guard = safe_lock(&db);
                            db_guard.begin_batch();
                            tc.execute_block(&mut *db_guard, &block)
                        };

                        match result {
                            Ok(result) => {
                                block.state_root = result.execution.state_root;

                                // Flush state atomically
                                {
                                    let mut db_guard = safe_lock(&db);
                                    db_guard.flush_accounts();
                                    db_guard.flush_objects();
                                    if let Err(e) = db_guard.commit_batch() {
                                        eprintln!("\x1b[31mFATAL: state batch commit failed: {}\x1b[0m", e);
                                    }
                                }

                                // Fold proof & attach to block
                                {
                                    let mut p = safe_lock(&chain_prover);
                                    match p.fold_block(&block, result.execution.state_root) {
                                        Ok(_fold_res) => {
                                            if let Ok(chain_proof) = p.generate_chain_proof() {
                                                block.nova_proof = Some(chain_proof.proof.proof_bytes);
                                            }
                                        }
                                        Err(e) => eprintln!("{} \x1b[31mProving error: {}\x1b[0m", node_tag, e),
                                    }
                                }

                                // DA: erasure-encode block and store shards
                                if block.data_root.is_some() {
                                    if let Ok(block_bytes) = serde_json::to_vec(&block) {
                                        // 2D encoding: populate row/col roots and NMT blob commitments
                                        if let Some(data_root_2d) = encode_block_2d(&mut block, &block_bytes) {
                                            println!(
                                                "{}   \x1b[36mDA-2D: {}x{} matrix, data_root={}\x1b[0m",
                                                node_tag,
                                                block.da_row_roots.len(),
                                                block.da_col_roots.len(),
                                                &hex::encode(data_root_2d)[..16],
                                            );
                                        }
                                        if let Ok(da) = BlockDA::new() {
                                            if let Ok(package) = da.encode_block(&block_bytes) {
                                                let shard_count = package.shards.len() as u32;
                                                println!(
                                                    "{}   \x1b[36mDA: {} shards, root={}\x1b[0m",
                                                    node_tag,
                                                    shard_count,
                                                    &hex::encode(package.header.commitment_root)[..16],
                                                );
                                                let mut store = safe_lock(&da_store);
                                                store.insert(block.number, package.clone());
                                                // Keep only last 64 blocks in memory
                                                while store.len() > 64 {
                                                    if let Some(&oldest) = store.keys().next() {
                                                        store.remove(&oldest);
                                                    }
                                                }
                                                drop(store);
                                                // Persist to disk
                                                log_persist_err("da_package", chain_store.save_da_package(block.number, &package));
                                                if block.number % 100 == 0 {
                                                    chain_store.prune_da_packages(block.number, 500);
                                                }

                                                // Also populate the network shard cache so peers can sample from us
                                                if let Some(ref sc) = shard_cache {
                                                    let mut cache = sc.write().unwrap_or_else(|p| p.into_inner());
                                                    cache.insert(block.number, package.clone());
                                                    while cache.len() > 500 {
                                                        if let Some(&oldest) = cache.keys().next() {
                                                            cache.remove(&oldest);
                                                        }
                                                    }
                                                }

                                                // Request shard samples from peers for DA verification
                                                if let Some(data_root) = block.data_root {
                                                    let queries = evaporchain_da::sampling::DASampler::generate_queries(
                                                        block.number, shard_count as usize, 4, &data_root,
                                                    );
                                                    if let Some(ref sender) = sample_request_sender {
                                                        let _ = sender.try_send(queries);
                                                    }

                                                    // Attest with local verification (peer sample results handled async below)
                                                    let mut tc = safe_lock(&tc_ref);
                                                    if let Some(att_msg) = tc.make_da_attestation(block.number, data_root, shard_count) {
                                                        // Self-register the attestation
                                                        tc.on_message(att_msg.clone());
                                                        // Try to build certificate
                                                        if let Some(cert_bytes) = tc.try_build_da_certificate(block.number, data_root) {
                                                            block.da_certificate = Some(cert_bytes);
                                                            println!(
                                                                "{}   \x1b[1;35mDA Certificate: block #{}, supermajority reached\x1b[0m",
                                                                node_tag, block.number,
                                                            );
                                                        }
                                                        tc.prune_da_attestations();
                                                        drop(tc);
                                                        // Broadcast attestation to peers
                                                        if let Some(ref sender) = consensus_net_sender {
                                                            if let Ok(data) = serde_json::to_vec(&att_msg) {
                                                                let _ = sender.send(data).await;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                let (obj_count, ghost_count) = {
                                    let db_guard = safe_lock(&db);
                                    (db_guard.object_count(), db_guard.ghost_count())
                                };
                                let peers = peer_count.load(std::sync::atomic::Ordering::Relaxed);

                                // Advance consensus state
                                {
                                    let mut tc = safe_lock(&tc_ref);
                                    tc.on_block_committed(&block, result.execution.state_root, result.execution.objects_evaporated);
                                }

                                // ── Frontier primitives update ──
                                {
                                    let da_info = block.da_certificate.as_ref().and_then(|_| {
                                        block.data_root.map(|dr| frontier::DACertInfo {
                                            data_root: dr,
                                            shard_count: 8,
                                            attested_stake: 3000,
                                            total_stake: 4000,
                                            aggregate_signature: vec![],
                                            signer_ids: vec![],
                                        })
                                    });
                                    let mmr_root = result.execution.mmr_root;
                                    let db_guard = safe_lock(&db);
                                    let mut fs = safe_lock(&frontier_state);
                                    let fu = fs.on_block_committed(
                                        block.number,
                                        block.epoch,
                                        result.execution.state_root,
                                        obj_count as u64,
                                        ghost_count as u64,
                                        mmr_root,
                                        da_info.as_ref(),
                                        &*db_guard,
                                    );
                                    if let Some(ref anchor) = fu.anchor_created {
                                        println!(
                                            "{} \x1b[1;35mAnchor #{}\x1b[0m state_root={} objects={} ghosts={}",
                                            node_tag, anchor.height,
                                            &hex::encode(anchor.hash)[..16],
                                            anchor.active_objects, anchor.ghost_count,
                                        );
                                    }
                                    if fu.poha_evaporated > 0 {
                                        println!(
                                            "{} \x1b[33mPoHA: {} certs evaporated\x1b[0m",
                                            node_tag, fu.poha_evaporated,
                                        );
                                    }
                                    // PoHA re-attestation: boost decaying certificates every 50 blocks
                                    if block.number % 50 == 0 {
                                        let to_reattest = fs.poha.select_for_re_attestation(block.epoch, 5);
                                        for bn in &to_reattest {
                                            fs.poha.re_attest(*bn, block.epoch);
                                        }
                                        if !to_reattest.is_empty() {
                                            println!(
                                                "{} \x1b[35mPoHA: re-attested {} certificates\x1b[0m",
                                                node_tag, to_reattest.len(),
                                            );
                                        }
                                    }
                                    if block.number % 50 == 0 {
                                        // Persist PoHA state and run temperature-based shard pruning
                                        log_persist_err("poha", chain_store.save_poha_state(&fs.poha));
                                        {
                                            let mut da_guard = safe_lock(&da_store);
                                            let prune_result = evaporchain_da::pruning::prune_by_temperature(&mut da_guard, &fs.poha);
                                            if prune_result.shards_pruned > 0 {
                                                println!(
                                                    "{} \x1b[33mDA prune: {} shards pruned ({} blocks removed, {} parity-pruned)\x1b[0m",
                                                    node_tag, prune_result.shards_pruned,
                                                    prune_result.blocks_fully_pruned,
                                                    prune_result.blocks_parity_pruned,
                                                );
                                            }
                                        }
                                        println!(
                                            "{} \x1b[35m[frontier] {}\x1b[0m",
                                            node_tag, fs.status_line(),
                                        );
                                    }
                                }

                                // Attach Rule-Based Consensus state function commitment
                                {
                                    let fs = safe_lock(&frontier_state);
                                    let commitment = fs.anchors.build_block_commitment(
                                        block.number,
                                        obj_count as u64,
                                    );
                                    block.state_function_commitment = Some(commitment);
                                }

                                // ── Oracle finalization per block ──
                                {
                                    let mut ob = safe_lock(&oracle_bridge);
                                    let finalized = ob.finalize_all();
                                    if !finalized.is_empty() {
                                        println!(
                                            "{} \x1b[36mOracle: {} feeds finalized\x1b[0m",
                                            node_tag, finalized.len(),
                                        );
                                    }
                                    let root = ob.oracle_state_root();
                                    if root != [0u8; 32] {
                                        block.oracle_state_root = Some(root);
                                    }
                                }

                                // ── Shard metrics recording ──
                                {
                                    let mut sb = safe_lock(&shard_bridge);
                                    let db_guard = safe_lock(&db);
                                    for oid in db_guard.all_object_ids().iter().take(100) {
                                        if let Some(obj) = db_guard.get_object(oid) {
                                            let mut short_id = [0u8; 20];
                                            short_id.copy_from_slice(&oid[..20]);
                                            let alive = obj.state == evaporchain_types::ObjectState::Active
                                                || obj.state == evaporchain_types::ObjectState::Resurrected;
                                            sb.record_object(&short_id, obj.energy, obj.half_life, alive);
                                        }
                                    }
                                    block.shard_count = Some(sb.shard_healths().len() as u16);
                                }

                                // Reset demo nonce offsets — on-chain nonces are now updated
                                demo_nonces = [0u64; 4];

                                // Cache and broadcast the committed block
                                if let Some(ref cache) = block_cache {
                                    cache_block(cache, &block);
                                }
                                if let Some(ref sender) = net_block_sender {
                                    let _ = sender.send(block.clone()).await;
                                }

                                // Record for API
                                let exec_elapsed_us = exec_start.elapsed().as_micros() as u64;
                                record_block(
                                    &block_history, &chain_stats, &events, &throughput,
                                    Some(&ws_broadcaster),
                                    &block, &result.execution,
                                    obj_count, ghost_count, exec_elapsed_us,
                                );

                                // Persist
                                log_persist_err("consensus_meta", chain_store.save_consensus_meta(block.number, block.epoch, block.parent_hash));
                                log_persist_err("full_block", chain_store.save_full_block(&block));
                                log_persist_err("tx_index", chain_store.index_block_transactions(&block).map(|_| ()));
                                {
                                    let history = safe_lock(&block_history);
                                    if let Some(record) = history.back() {
                                        log_persist_err("block", chain_store.save_block(record));
                                    }
                                }
                                {
                                    let stats = safe_lock(&chain_stats);
                                    log_persist_err("chain_stats", chain_store.save_chain_stats(&stats));
                                }
                                {
                                    let ev = safe_lock(&events);
                                    log_persist_err("events", chain_store.save_events(&ev));
                                }
                                // Persist mempool
                                {
                                    let tc = safe_lock(&tc_ref);
                                    let pending: Vec<evaporchain_types::Transaction> = tc.mempool.pending().iter().cloned().collect();
                                    log_persist_err("mempool", chain_store.save_mempool(&pending));
                                }
                                persist_contracts(&chain_store, &tendermint);

                                // DA encode the block for light client sampling
                                if let Ok(da) = evaporchain_da::block_da::BlockDA::new() {
                                    let block_bytes = serde_json::to_vec(&block).unwrap_or_default();
                                    // 2D encoding: populate row/col roots and NMT
                                    encode_block_2d(&mut block, &block_bytes);
                                    if let Ok(package) = da.encode_block(&block_bytes) {
                                        if let Some(ref sc) = shard_cache {
                                            let mut cache = sc.write().unwrap_or_else(|p| p.into_inner());
                                            cache.insert(block.number, package.clone());
                                            while cache.len() > 500 {
                                                if let Some(&oldest) = cache.keys().next() {
                                                    cache.remove(&oldest);
                                                }
                                            }
                                        }
                                        let mut store = da_store.lock().unwrap();
                                        store.insert(block.number, package);
                                        while store.len() > 256 {
                                            if let Some(&oldest) = store.keys().next() {
                                                store.remove(&oldest);
                                            }
                                        }
                                    }
                                }

                                // Create state snapshot every 100 blocks for state sync
                                if block.number % 100 == 0 && block.number > 0 {
                                    let mut db_guard = safe_lock(&db);
                                    match evaporchain_state::snapshot::SnapshotBuilder::create(
                                        &mut *db_guard, block.number, block.epoch,
                                    ) {
                                        Ok(snapshot) => {
                                            if let Ok(bytes) = evaporchain_state::snapshot::serialize_snapshot(&snapshot) {
                                                let size = bytes.len();
                                                log_persist_err("snapshot", chain_store.save_snapshot(block.number, &bytes, result.execution.state_root));
                                                {
                                                    let mut info = snapshot_info.lock().unwrap();
                                                    *info = Some((block.number, result.execution.state_root, size));
                                                }
                                                eprintln!(
                                                    "{} \x1b[1;35mSnapshot created at height {} ({} bytes, {} accounts, {} objects)\x1b[0m",
                                                    node_tag, block.number, size,
                                                    snapshot.header.account_count, snapshot.header.object_count,
                                                );
                                            }
                                        }
                                        Err(e) => eprintln!("{} \x1b[31mSnapshot error: {}\x1b[0m", node_tag, e),
                                    }
                                }

                                // Prune old blocks and snapshots every 100 blocks
                                if block.number % 100 == 0 && block.number > 1000 {
                                    let pruned = chain_store.prune_blocks(block.number, 1000);
                                    chain_store.prune_full_blocks(block.number, 2000);
                                    chain_store.prune_old_snapshots(block.number, 200);
                                    if pruned > 0 {
                                        tracing::info!("Pruned {} old block records (retain last 1000)", pruned);
                                    }
                                }

                                let producer_str = block.producer_id
                                    .map(|id| format!("validator-{}", id))
                                    .unwrap_or_else(|| "unknown".to_string());

                                print_block_result(
                                    &node_tag,
                                    &format!("COMMITTED ({})", producer_str),
                                    block.number,
                                    block.epoch,
                                    result.execution.txs_executed,
                                    result.execution.txs_failed,
                                    result.execution.objects_entered_grace,
                                    result.execution.objects_evaporated,
                                    obj_count,
                                    ghost_count,
                                    &result.execution.state_root,
                                    peers,
                                );
                                // Log BLS aggregate signature status
                                if let Some(ref cert) = block.commit_certificate {
                                    println!(
                                        "{}   \x1b[1;36mBLS CommitCertificate: {} signers, agg_sig={}B\x1b[0m",
                                        node_tag, cert.signer_ids.len(), cert.aggregate_signature.len()
                                    );
                                }
                            }
                            Err(e) => {
                                safe_lock(&db).rollback_batch();
                                eprintln!("{} \x1b[31mBlock execution error: {}\x1b[0m", node_tag, e);
                            }
                        }
                    }
                }
            }

            // ── Receive consensus messages from network ──
            Some(data) = async {
                match consensus_net_receiver.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending::<Option<Vec<u8>>>().await,
                }
            }, if tendermint.is_some() => {
                if let Ok(msg) = serde_json::from_slice::<ConsensusMessage>(&data) {
                    eprintln!(
                        "{} [net-msg] h={} r={} type={}",
                        node_tag, msg.height(), msg.round(),
                        match &msg {
                            ConsensusMessage::Proposal { .. } => "Proposal",
                            ConsensusMessage::Prevote { .. } => "Prevote",
                            ConsensusMessage::Precommit { .. } => "Precommit",
                            ConsensusMessage::KeyAnnounce { .. } => "KeyAnnounce",
                            ConsensusMessage::DAAttestation { .. } => "DAAttestation",
                        }
                    );
                    // Check if this is a DA attestation that might complete a certificate
                    let da_att_info = if let ConsensusMessage::DAAttestation { block_number, data_root, .. } = &msg {
                        Some((*block_number, *data_root))
                    } else {
                        None
                    };

                    let tc_ref = tendermint.as_ref().unwrap();
                    let actions = {
                        let mut tc = safe_lock(&tc_ref);
                        let actions = tc.on_message(msg);
                        // If DA attestation, check if supermajority now reached
                        if let Some((bn, dr)) = da_att_info {
                            if let Some(_cert_bytes) = tc.try_build_da_certificate(bn, dr) {
                                println!(
                                    "{}   \x1b[1;35mDA Certificate: block #{}, supermajority reached (from peer attestation)\x1b[0m",
                                    node_tag, bn,
                                );
                                // Register with PoHA now that DA cert is confirmed
                                let mut fs = safe_lock(&frontier_state);
                                fs.poha.register(
                                    bn, dr, 8, 3000, 4000, bn,
                                    vec![], vec![],
                                );
                            }
                        }
                        actions
                    };
                    let mut commits = broadcast_consensus_actions(
                        actions, &consensus_net_sender, &node_tag,
                    ).await;

                    // Handle any commits from message processing
                    for action in commits.drain(..) {
                        if let ConsensusAction::RequestSync(from, to) = action {
                            if !sync_in_flight {
                                println!(
                                    "{} \x1b[36mConsensus requests sync (msg): blocks {}..{}\x1b[0m",
                                    node_tag, from, to
                                );
                                if let Some(ref sender) = sync_request_sender {
                                    let _ = sender.send((from, to)).await;
                                    sync_in_flight = true;
                                }
                            }
                            continue;
                        }
                        if let ConsensusAction::CommitBlock(mut block) = action {
                            let exec_start = Instant::now();
                            let result = {
                                let mut tc = safe_lock(&tc_ref);
                                let mut db_guard = safe_lock(&db);
                                db_guard.begin_batch();
                                tc.execute_block(&mut *db_guard, &block)
                            };

                            match result {
                                Ok(result) => {
                                    block.state_root = result.execution.state_root;
                                    {
                                        let mut db_guard = safe_lock(&db);
                                        db_guard.flush_accounts();
                                        db_guard.flush_objects();
                                        if let Err(e) = db_guard.commit_batch() {
                                            eprintln!("\x1b[31mFATAL: state batch commit failed: {}\x1b[0m", e);
                                        }
                                    }
                                    {
                                        let mut p = safe_lock(&chain_prover);
                                        if let Ok(_) = p.fold_block(&block, result.execution.state_root) {
                                            if let Ok(chain_proof) = p.generate_chain_proof() {
                                                block.nova_proof = Some(chain_proof.proof.proof_bytes);
                                            }
                                        }
                                    }

                                    // DA: erasure-encode block and store shards
                                    if block.data_root.is_some() {
                                        if let Ok(block_bytes) = serde_json::to_vec(&block) {
                                            // 2D encoding: populate row/col roots and NMT
                                            encode_block_2d(&mut block, &block_bytes);
                                            if let Ok(da) = BlockDA::new() {
                                                if let Ok(package) = da.encode_block(&block_bytes) {
                                                    let shard_count = package.shards.len() as u32;
                                                    println!(
                                                        "{}   \x1b[36mDA: {} shards, root={}\x1b[0m",
                                                        node_tag,
                                                        shard_count,
                                                        &hex::encode(package.header.commitment_root)[..16],
                                                    );
                                                    let mut store = safe_lock(&da_store);
                                                    store.insert(block.number, package.clone());
                                                    while store.len() > 64 {
                                                        if let Some(&oldest) = store.keys().next() {
                                                            store.remove(&oldest);
                                                        }
                                                    }
                                                    drop(store);
                                                    if let Some(ref sc) = shard_cache {
                                                        let mut cache = sc.write().unwrap_or_else(|p| p.into_inner());
                                                        cache.insert(block.number, package.clone());
                                                        while cache.len() > 500 {
                                                            if let Some(&oldest) = cache.keys().next() {
                                                                cache.remove(&oldest);
                                                            }
                                                        }
                                                    }

                                                    // Request peer shard samples + create DA attestation
                                                    if let Some(data_root) = block.data_root {
                                                        let queries = evaporchain_da::sampling::DASampler::generate_queries(
                                                            block.number, shard_count as usize, 4, &data_root,
                                                        );
                                                        if let Some(ref sender) = sample_request_sender {
                                                            let _ = sender.try_send(queries);
                                                        }
                                                        let mut tc = safe_lock(&tc_ref);
                                                        if let Some(att_msg) = tc.make_da_attestation(block.number, data_root, shard_count) {
                                                            tc.on_message(att_msg.clone());
                                                            if let Some(cert_bytes) = tc.try_build_da_certificate(block.number, data_root) {
                                                                block.da_certificate = Some(cert_bytes);
                                                                println!(
                                                                    "{}   \x1b[1;35mDA Certificate: block #{}, supermajority reached\x1b[0m",
                                                                    node_tag, block.number,
                                                                );
                                                            }
                                                            tc.prune_da_attestations();
                                                            drop(tc);
                                                            if let Some(ref sender) = consensus_net_sender {
                                                                if let Ok(data) = serde_json::to_vec(&att_msg) {
                                                                    let _ = sender.send(data).await;
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    let (obj_count, ghost_count) = {
                                        let db_guard = safe_lock(&db);
                                        (db_guard.object_count(), db_guard.ghost_count())
                                    };
                                    let peers = peer_count.load(std::sync::atomic::Ordering::Relaxed);

                                    {
                                        let mut tc = safe_lock(&tc_ref);
                                        tc.on_block_committed(&block, result.execution.state_root, result.execution.objects_evaporated);
                                    }

                                    // ── Frontier primitives update (gossip path) ──
                                    {
                                        let da_info = block.da_certificate.as_ref().and_then(|_| {
                                            block.data_root.map(|dr| frontier::DACertInfo {
                                                data_root: dr,
                                                shard_count: 8,
                                                attested_stake: 3000,
                                                total_stake: 4000,
                                                aggregate_signature: vec![],
                                                signer_ids: vec![],
                                            })
                                        });
                                        let db_guard = safe_lock(&db);
                                        let mut fs = safe_lock(&frontier_state);
                                        let fu = fs.on_block_committed(
                                            block.number,
                                            block.epoch,
                                            result.execution.state_root,
                                            obj_count as u64,
                                            ghost_count as u64,
                                            result.execution.mmr_root,
                                            da_info.as_ref(),
                                            &*db_guard,
                                        );
                                        if let Some(ref anchor) = fu.anchor_created {
                                            println!(
                                                "{} \x1b[1;35mAnchor #{}\x1b[0m hash={} objects={} ghosts={}",
                                                node_tag, anchor.height,
                                                &hex::encode(anchor.hash)[..16],
                                                anchor.active_objects, anchor.ghost_count,
                                            );
                                        }
                                        if block.number % 50 == 0 {
                                            println!(
                                                "{} \x1b[35m[frontier] {}\x1b[0m",
                                                node_tag, fs.status_line(),
                                            );
                                        }

                                        // Attach Rule-Based Consensus state function commitment
                                        let commitment = fs.anchors.build_block_commitment(
                                            block.number,
                                            obj_count as u64,
                                        );
                                        block.state_function_commitment = Some(commitment);
                                    }

                                    // ── Oracle finalization (gossip path) ──
                                    {
                                        let mut ob = safe_lock(&oracle_bridge);
                                        let finalized = ob.finalize_all();
                                        if !finalized.is_empty() {
                                            println!(
                                                "{} \x1b[36mOracle: {} feeds finalized\x1b[0m",
                                                node_tag, finalized.len(),
                                            );
                                        }
                                        let root = ob.oracle_state_root();
                                        if root != [0u8; 32] {
                                            block.oracle_state_root = Some(root);
                                        }
                                    }

                                    // ── Shard metrics (gossip path) ──
                                    {
                                        let mut sb = safe_lock(&shard_bridge);
                                        let db_guard = safe_lock(&db);
                                        for oid in db_guard.all_object_ids().iter().take(100) {
                                            if let Some(obj) = db_guard.get_object(oid) {
                                                let mut short_id = [0u8; 20];
                                                short_id.copy_from_slice(&oid[..20]);
                                                let alive = obj.state == evaporchain_types::ObjectState::Active
                                                    || obj.state == evaporchain_types::ObjectState::Resurrected;
                                                sb.record_object(&short_id, obj.energy, obj.half_life, alive);
                                            }
                                        }
                                        block.shard_count = Some(sb.shard_healths().len() as u16);
                                    }

                                    if let Some(ref cache) = block_cache {
                                        cache_block(cache, &block);
                                    }
                                    if let Some(ref sender) = net_block_sender {
                                        let _ = sender.send(block.clone()).await;
                                    }

                                    let exec_elapsed_us = exec_start.elapsed().as_micros() as u64;
                                    record_block(
                                        &block_history, &chain_stats, &events, &throughput,
                                        Some(&ws_broadcaster),
                                        &block, &result.execution,
                                        obj_count, ghost_count, exec_elapsed_us,
                                    );
                                    log_persist_err("consensus_meta", chain_store.save_consensus_meta(block.number, block.epoch, block.parent_hash));
                                    log_persist_err("full_block", chain_store.save_full_block(&block));
                                    log_persist_err("tx_index", chain_store.index_block_transactions(&block).map(|_| ()));
                                    index_contract_events_from_exec(&chain_store, &block, &result.execution);
                                    {
                                        let history = safe_lock(&block_history);
                                        if let Some(record) = history.back() {
                                            log_persist_err("block", chain_store.save_block(record));
                                        }
                                    }
                                    {
                                        let stats = safe_lock(&chain_stats);
                                        log_persist_err("chain_stats", chain_store.save_chain_stats(&stats));
                                    }
                                    {
                                        let ev = safe_lock(&events);
                                        log_persist_err("events", chain_store.save_events(&ev));
                                    }
                                    // Persist mempool
                                    {
                                        let tc = safe_lock(&tc_ref);
                                        let pending: Vec<evaporchain_types::Transaction> = tc.mempool.pending().iter().cloned().collect();
                                        log_persist_err("mempool", chain_store.save_mempool(&pending));
                                    }
                                    persist_contracts(&chain_store, &tendermint);

                                    // DA encode (follower path)
                                    if let Ok(da) = evaporchain_da::block_da::BlockDA::new() {
                                        let block_bytes = serde_json::to_vec(&block).unwrap_or_default();
                                        // 2D encoding: populate row/col roots and NMT
                                        encode_block_2d(&mut block, &block_bytes);
                                        if let Ok(package) = da.encode_block(&block_bytes) {
                                            if let Some(ref sc) = shard_cache {
                                                let mut cache = sc.write().unwrap_or_else(|p| p.into_inner());
                                                cache.insert(block.number, package.clone());
                                                while cache.len() > 500 {
                                                    if let Some(&oldest) = cache.keys().next() {
                                                        cache.remove(&oldest);
                                                    }
                                                }
                                            }
                                            let mut store = da_store.lock().unwrap();
                                            store.insert(block.number, package);
                                            while store.len() > 256 {
                                                if let Some(&oldest) = store.keys().next() {
                                                    store.remove(&oldest);
                                                }
                                            }
                                        }
                                    }

                                    let producer_str = block.producer_id
                                        .map(|id| format!("validator-{}", id))
                                        .unwrap_or_else(|| "unknown".to_string());

                                    print_block_result(
                                        &node_tag,
                                        &format!("COMMITTED ({})", producer_str),
                                        block.number, block.epoch,
                                        result.execution.txs_executed,
                                        result.execution.txs_failed,
                                        result.execution.objects_entered_grace,
                                        result.execution.objects_evaporated,
                                        obj_count, ghost_count,
                                        &result.execution.state_root, peers,
                                    );
                                }
                                Err(e) => {
                                    safe_lock(&db).rollback_batch();
                                    eprintln!("{} \x1b[31mBlock execution error: {}\x1b[0m", node_tag, e);
                                }
                            }
                        }
                    }
                }
            }

            // ── Tick: producer creates a block (mock consensus only) ──
            _ = ticker.tick(), if is_producer && tendermint.is_none() => {
                // In demo mode, inject random transactions
                if args.demo_mode {
                    let epoch = {
                        let c = safe_lock(&consensus);
                        c.epoch() + 1
                    };
                    if let Some(tx) = generate_demo_tx(&mut rng, epoch, &mut demo_nonces, &demo_keypairs, args.validator_id, args.validator_count, &db) {
                        let mut c = safe_lock(&consensus);
                        c.mempool.submit(tx);
                    }
                }

                // Drain any txs received from the network into the mempool
                if let Some(ref mut rx) = net_tx_receiver {
                    while let Ok(tx) = rx.try_recv() {
                        let mut c = safe_lock(&consensus);
                        c.mempool.submit(tx);
                    }
                }

                // Drain API-submitted txs and broadcast via P2P
                while let Ok(tx) = api_tx_receiver.try_recv() {
                    if let Some(ref sender) = net_tx_sender {
                        let _ = sender.try_send(tx);
                    }
                }

                // Produce block — all synchronous work under locks, then drop before await
                let exec_start = Instant::now();
                let produced = {
                    let mut c = safe_lock(&consensus);
                    let mut db_guard = safe_lock(&db);
                    db_guard.begin_batch();

                    match c.produce_block(&mut *db_guard) {
                        Ok(mut result) => {
                            let mut p = safe_lock(&chain_prover);
                            match p.fold_block(&result.block, result.execution.state_root) {
                                Ok(fold_res) => {
                                    if args.prove_mode {
                                        println!(
                                            "{}   \x1b[35mProof: fold={:.1}ms  acc={}B  folded={}\x1b[0m",
                                            node_tag,
                                            fold_res.fold_time_us as f64 / 1000.0,
                                            fold_res.accumulator_size,
                                            p.blocks_folded(),
                                        );
                                    }
                                    if let Ok(chain_proof) = p.generate_chain_proof() {
                                        result.block.nova_proof = Some(chain_proof.proof.proof_bytes);
                                    }
                                }
                                Err(e) => eprintln!("{} \x1b[31mProving error: {}\x1b[0m", node_tag, e),
                            }
                            drop(p);

                            // Flush mutated state to RocksDB atomically
                            db_guard.flush_accounts();
                            db_guard.flush_objects();
                            if let Err(e) = db_guard.commit_batch() {
                                eprintln!("\x1b[31mFATAL: state batch commit failed: {}\x1b[0m", e);
                            }

                            let obj_count = db_guard.object_count();
                            let ghost_count = db_guard.ghost_count();
                            Some((result, obj_count, ghost_count))
                        }
                        Err(e) => {
                            db_guard.rollback_batch();
                            eprintln!("{} \x1b[31mBlock production error: {}\x1b[0m", node_tag, e);
                            None
                        }
                    }
                }; // all locks dropped here

                if let Some((result, obj_count, ghost_count)) = produced {
                    let peers = peer_count.load(std::sync::atomic::Ordering::Relaxed);

                    // Cache block for serving to peers via block sync
                    if let Some(ref cache) = block_cache {
                        cache_block(cache, &result.block);
                    }

                    // Broadcast block to network (async, no locks held)
                    if let Some(ref sender) = net_block_sender {
                        let _ = sender.send(result.block.clone()).await;
                    }

                    // Record block for API
                    let exec_elapsed_us = exec_start.elapsed().as_micros() as u64;
                    record_block(
                        &block_history,
                        &chain_stats,
                        &events,
                        &throughput,
                        Some(&ws_broadcaster),
                        &result.block,
                        &result.execution,
                        obj_count,
                        ghost_count,
                        exec_elapsed_us,
                    );

                    // Persist chain data to disk
                    log_persist_err("consensus_meta", chain_store.save_consensus_meta(
                        result.block.number,
                        result.block.epoch,
                        result.block.parent_hash,
                    ));
                    log_persist_err("full_block", chain_store.save_full_block(&result.block));
                log_persist_err("tx_index", chain_store.index_block_transactions(&result.block).map(|_| ()));
                    index_contract_events_from_exec(&chain_store, &result.block, &result.execution);
                    {
                        let history = safe_lock(&block_history);
                        if let Some(record) = history.back() {
                            log_persist_err("block", chain_store.save_block(record));
                        }
                    }
                    {
                        let stats = safe_lock(&chain_stats);
                        log_persist_err("chain_stats", chain_store.save_chain_stats(&stats));
                    }
                    {
                        let ev = safe_lock(&events);
                        log_persist_err("events", chain_store.save_events(&ev));
                    }
                    // Persist mempool
                    {
                        let c = safe_lock(&consensus);
                        let pending: Vec<evaporchain_types::Transaction> = c.mempool.pending().iter().cloned().collect();
                        log_persist_err("mempool", chain_store.save_mempool(&pending));
                    }

                    // Prune old blocks every 100 blocks
                    if result.block.number % 100 == 0 && result.block.number > 1000 {
                        let pruned = chain_store.prune_blocks(result.block.number, 1000);
                        chain_store.prune_full_blocks(result.block.number, 2000);
                        chain_store.prune_old_snapshots(result.block.number, 200);
                        if pruned > 0 {
                            tracing::info!("Pruned {} old block records (retain last 1000)", pruned);
                        }
                    }

                    print_block_result(
                        &node_tag,
                        "PRODUCED",
                        result.block.number,
                        result.block.epoch,
                        result.execution.txs_executed,
                        result.execution.txs_failed,
                        result.execution.objects_entered_grace,
                        result.execution.objects_evaporated,
                        obj_count,
                        ghost_count,
                        &result.execution.state_root,
                        peers,
                    );
                }
            }

            // ── Receive block from network (follower path with gap detection) ──
            Some(block) = async {
                match net_block_receiver.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending::<Option<evaporchain_types::Block>>().await,
                }
            } => {
                let local_height = if let Some(ref tc_ref) = tendermint {
                    // In Tendermint mode, height() is the next height to decide,
                    // so the last committed block = height - 1.
                    let tc = safe_lock(tc_ref);
                    tc.height().saturating_sub(1)
                } else {
                    let c = safe_lock(&consensus);
                    c.block_number()
                };

                // Skip blocks already committed
                if block.number <= local_height {
                    continue;
                }

                // In Tendermint mode: if this block is ahead of us, trigger sync
                // instead of blindly skipping. This fixes the state-sync-on-rejoin bug
                // where a late-joining node never catches up.
                if tendermint.is_some() {
                    let expected_next = local_height + 1;
                    if block.number > expected_next && !sync_in_flight {
                        println!(
                            "{} [33m⚠ Tendermint sync gap: at #{}, received #{} — requesting backfill[0m",
                            node_tag, local_height, block.number
                        );
                        // Queue this block for later
                        let block_num = block.number;
                        pending_blocks.insert(block_num, block);
                        while pending_blocks.len() > 500 {
                            if let Some(&oldest) = pending_blocks.keys().next() {
                                pending_blocks.remove(&oldest);
                            }
                        }
                        // Request missing blocks
                        if let Some(ref sender) = sync_request_sender {
                            let gap_end = block_num.saturating_sub(1);
                            println!(
                                "{} [36mRequesting blocks {}..{} from peers[0m",
                                node_tag, expected_next, gap_end
                            );
                            let _ = sender.send((expected_next, gap_end)).await;
                            sync_in_flight = true;
                        }
                    } else if block.number > expected_next {
                        // Already have a sync in flight, just queue
                        let block_num = block.number;
                        pending_blocks.insert(block_num, block);
                        while pending_blocks.len() > 500 {
                            if let Some(&oldest) = pending_blocks.keys().next() {
                                pending_blocks.remove(&oldest);
                            }
                        }
                    }
                    // For blocks at expected_next in Tendermint mode,
                    // consensus round handles them — skip to avoid double-apply
                    continue;
                }

                let expected_next = local_height + 1;

                if block.number == expected_next {
                    // Perfect: next block in sequence — apply it
                    apply_follower_block(
                        &node_tag, &block, &consensus, &db, &chain_prover,
                        args.prove_mode, &block_history, &chain_stats, &events,
                        &throughput, &chain_store, &peer_count, &block_cache,
                        &tendermint, &ws_broadcaster,
                    );

                    // After applying, drain any pending blocks that are now in sequence
                    loop {
                        let next = {
                            let c = safe_lock(&consensus);
                            c.block_number() + 1
                        };
                        if let Some(queued) = pending_blocks.remove(&next) {
                            apply_follower_block(
                                &node_tag, &queued, &consensus, &db, &chain_prover,
                                args.prove_mode, &block_history, &chain_stats, &events,
                                &throughput, &chain_store, &peer_count, &block_cache,
                                &tendermint, &ws_broadcaster,
                            );
                        } else {
                            break;
                        }
                    }
                    sync_in_flight = false;
                } else {
                    // Gap detected: block.number > expected_next
                    println!(
                        "{} \x1b[33m⚠ Gap detected: have block #{}, expected #{} — queuing and requesting backfill\x1b[0m",
                        node_tag, block.number, expected_next
                    );

                    // Queue this future block
                    pending_blocks.insert(block.number, block);

                    // Cap pending queue to prevent memory bloat
                    while pending_blocks.len() > 500 {
                        if let Some(&oldest) = pending_blocks.keys().next() {
                            pending_blocks.remove(&oldest);
                        }
                    }

                    // Request the missing blocks from peers (if not already in flight)
                    if !sync_in_flight {
                        if let Some(ref sender) = sync_request_sender {
                            let gap_end = pending_blocks.keys().next().copied().unwrap_or(expected_next) - 1;
                            println!(
                                "{} \x1b[36mRequesting blocks {}..{} from peers\x1b[0m",
                                node_tag, expected_next, gap_end
                            );
                            let _ = sender.send((expected_next, gap_end)).await;
                            sync_in_flight = true;
                        }
                    }
                }
            }

            // ── Receive synced blocks from peer (backfill response) ──
            Some(blocks) = async {
                match sync_blocks_receiver.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending::<Option<Vec<evaporchain_types::Block>>>().await,
                }
            } => {
                sync_in_flight = false;
                let count = blocks.len();
                if count > 0 {
                    println!(
                        "{} \x1b[36mReceived {} sync blocks — applying...\x1b[0m",
                        node_tag, count
                    );

                    // Sort by block number and apply in order
                    let mut sorted = blocks;
                    sorted.sort_by_key(|b| b.number);

                    for block in &sorted {
                        let local_height = if let Some(ref tc_ref) = tendermint {
                            let tc = safe_lock(tc_ref);
                            tc.height().saturating_sub(1)
                        } else {
                            let c = safe_lock(&consensus);
                            c.block_number()
                        };

                        if block.number <= local_height {
                            continue; // Already have this block
                        }
                        if block.number > local_height + 1 {
                            // Gap — queue for later
                            pending_blocks.insert(block.number, block.clone());
                            continue;
                        }

                        // block.number == local_height + 1
                        // Verify CommitCertificate before applying sync block.
                        // Require valid certificate — never apply unverified blocks.
                        if let Some(ref tc_ref) = tendermint {
                            let tc = safe_lock(tc_ref);
                            match &block.commit_certificate {
                                Some(cert) => {
                                    let has_all_keys = cert.signer_ids.iter().all(|&vid| {
                                        tc.validator_set().get(vid)
                                            .map_or(false, |v| v.bls_public_key.is_some())
                                    });
                                    if !has_all_keys {
                                        // Missing keys — defer until KeyAnnounce arrives
                                        drop(tc);
                                        pending_blocks.insert(block.number, block.clone());
                                        continue;
                                    }
                                    if !tc.verify_commit_certificate(cert) {
                                        eprintln!(
                                            "{} \x1b[31mREJECTED sync block #{} - invalid BLS CommitCertificate\x1b[0m",
                                            node_tag, block.number
                                        );
                                        continue;
                                    }
                                }
                                None => {
                                    eprintln!(
                                        "{} \x1b[31mREJECTED sync block #{} - missing CommitCertificate\x1b[0m",
                                        node_tag, block.number
                                    );
                                    continue;
                                }
                            }
                        }

                        if let Some(ref tc_ref) = tendermint {
                            // Apply via Tendermint consensus for state consistency
                            let result = {
                                let mut tc = safe_lock(tc_ref);
                                let mut db_guard = safe_lock(&db);
                                tc.apply_block(&mut *db_guard, block)
                            };
                            match result {
                                Ok(result) => {
                                    {
                                        let mut db_guard = safe_lock(&db);
                                        db_guard.flush_accounts();
                                        db_guard.flush_objects();
                                    }
                                    {
                                        let mut p = safe_lock(&chain_prover);
                                        let _ = p.fold_block(block, result.execution.state_root);
                                    }
                                    let (obj_count, ghost_count) = {
                                        let db_guard = safe_lock(&db);
                                        (db_guard.object_count(), db_guard.ghost_count())
                                    };
                                    let peers = peer_count.load(std::sync::atomic::Ordering::Relaxed);
                                    if let Some(ref cache) = block_cache {
                                        cache_block(cache, block);
                                    }
                                    // Record in block history & chain store
                                    {
                                        let record = BlockRecord {
                                            number: block.number,
                                            epoch: block.epoch,
                                            state_root: hex::encode(result.execution.state_root),
                                            parent_hash: hex::encode(block.parent_hash),
                                            tx_count: block.transactions.len(),
                                            evaporations: result.execution.objects_evaporated,
                                            entered_grace: result.execution.objects_entered_grace,
                                            timestamp: block.timestamp,
                                            active_objects: obj_count,
                                            ghost_count,
                                            gas_used: result.execution.gas_used,
                                            base_fee: result.execution.base_fee,
                                            total_fees: result.execution.total_fees,
                                            transactions: api::tx_records_from_block(block),
                                            has_nova_proof: block.nova_proof.is_some(),
                                            nova_proof_size: block.nova_proof.as_ref().map_or(0, |p| p.len()),
                                            data_root: block.data_root.map(hex::encode),
                                            da_square_size: block.da_row_roots.len(),
                                            blob_count: block.blob_commitments.len(),
                                            has_state_commitment: block.state_function_commitment.is_some(),
                                            is_anchor: block.state_function_commitment.as_ref().map_or(false, |c| c.is_anchor),
                                            anchor_epoch: block.state_function_commitment.as_ref().map_or(0, |c| c.anchor_epoch),
                                        };
                                        let mut history = safe_lock(&block_history);
                                        history.push_back(record.clone());
                                        if history.len() > 500 { history.pop_front(); }
                                        log_persist_err("block", chain_store.save_block(&record));
                                        log_persist_err("full_block", chain_store.save_full_block(block));
                                        log_persist_err("tx_index", chain_store.index_block_transactions(block).map(|_| ()));
                                    }
                                    // Update chain stats (same as record_block_production)
                                    {
                                        let mut tx_creates = 0u64;
                                        let mut tx_refreshes = 0u64;
                                        for tx in &block.transactions {
                                            match tx {
                                                Transaction::CreateObject(_) => tx_creates += 1,
                                                Transaction::Refresh(_) => tx_refreshes += 1,
                                                _ => {}
                                            }
                                        }
                                        let mut stats = safe_lock(&chain_stats);
                                        stats.total_objects_created += tx_creates;
                                        stats.total_refreshed += tx_refreshes;
                                        stats.total_evaporated += result.execution.objects_evaporated as u64;
                                        stats.total_transactions += block.transactions.len() as u64;
                                        stats.state_size_trend.push(api::EpochSnapshot {
                                            epoch: block.epoch,
                                            active_count: obj_count,
                                            ghost_count,
                                            total_energy: 0,
                                        });
                                        if stats.state_size_trend.len() > 1000 {
                                            let excess = stats.state_size_trend.len() - 1000;
                                            stats.state_size_trend.drain(..excess);
                                        }
                                    }
                                    println!(
                                        "\n{} \x1b[1;32m━━━ Block #{:<4} │ Epoch {:<4} ━━━ SYNCED ━━━━━━━━━━━━━━━━━━━━━━\x1b[0m",
                                        node_tag, block.number, block.epoch,
                                    );
                                    println!(
                                        "{}   State: \x1b[36m{} active\x1b[0m  \x1b[90m{} ghosts\x1b[0m  root=\x1b[1m{}…\x1b[0m  peers={}",
                                        node_tag, obj_count, ghost_count,
                                        &hex::encode(result.execution.state_root)[..16], peers,
                                    );
                                }
                                Err(e) => {
                                    eprintln!("{} \x1b[31mSync apply error: {}\x1b[0m", node_tag, e);
                                }
                            }
                        } else {
                            apply_follower_block(
                                &node_tag, block, &consensus, &db, &chain_prover,
                                args.prove_mode, &block_history, &chain_stats, &events,
                                &throughput, &chain_store, &peer_count, &block_cache,
                                &tendermint, &ws_broadcaster,
                            );
                        }
                    }

                    // Drain pending queue after applying synced blocks
                    loop {
                        let next = if let Some(ref tc_ref) = tendermint {
                            let tc = safe_lock(tc_ref);
                            tc.height()
                        } else {
                            let c = safe_lock(&consensus);
                            c.block_number() + 1
                        };
                        if let Some(queued) = pending_blocks.remove(&next) {
                            // Verify CommitCertificate on queued block (lenient — skip if missing BLS keys)
                            if let Some(ref cert) = queued.commit_certificate {
                                if let Some(ref tc_ref) = tendermint {
                                    let tc = safe_lock(tc_ref);
                                    let has_all_keys = cert.signer_ids.iter().all(|&vid| {
                                        tc.validator_set().get(vid)
                                            .map_or(false, |v| v.bls_public_key.is_some())
                                    });
                                    if has_all_keys && !tc.verify_commit_certificate(cert) {
                                        eprintln!(
                                            "{} \x1b[31m⚠ REJECTED queued block #{} — invalid BLS CommitCertificate\x1b[0m",
                                            node_tag, queued.number
                                        );
                                        continue;
                                    }
                                }
                            }
                            if let Some(ref tc_ref) = tendermint {
                                let result = {
                                    let mut tc = safe_lock(tc_ref);
                                    let mut db_guard = safe_lock(&db);
                                    tc.apply_block(&mut *db_guard, &queued)
                                };
                                if let Ok(result) = result {
                                    let mut db_guard = safe_lock(&db);
                                    db_guard.flush_accounts();
                                    db_guard.flush_objects();
                                    let obj_count = db_guard.object_count();
                                    let gh_count = db_guard.ghost_count();
                                    drop(db_guard);
                                    let _ = safe_lock(&chain_prover).fold_block(&queued, result.execution.state_root);
                                    // Update stats for queued/pending blocks
                                    {
                                        let mut tx_creates = 0u64;
                                        let mut tx_refreshes = 0u64;
                                        for tx in &queued.transactions {
                                            match tx {
                                                Transaction::CreateObject(_) => tx_creates += 1,
                                                Transaction::Refresh(_) => tx_refreshes += 1,
                                                _ => {}
                                            }
                                        }
                                        let mut stats = safe_lock(&chain_stats);
                                        stats.total_objects_created += tx_creates;
                                        stats.total_refreshed += tx_refreshes;
                                        stats.total_evaporated += result.execution.objects_evaporated as u64;
                                        stats.total_transactions += queued.transactions.len() as u64;
                                        stats.state_size_trend.push(api::EpochSnapshot {
                                            epoch: queued.epoch,
                                            active_count: obj_count,
                                            ghost_count: gh_count,
                                            total_energy: 0,
                                        });
                                        if stats.state_size_trend.len() > 1000 {
                                            let excess = stats.state_size_trend.len() - 1000;
                                            stats.state_size_trend.drain(..excess);
                                        }
                                    }
                                }
                            } else {
                                apply_follower_block(
                                    &node_tag, &queued, &consensus, &db, &chain_prover,
                                    args.prove_mode, &block_history, &chain_stats, &events,
                                    &throughput, &chain_store, &peer_count, &block_cache,
                                    &tendermint, &ws_broadcaster,
                                );
                            }
                        } else {
                            break;
                        }
                    }

                    // Check if there are still pending blocks (more gaps)
                    if !pending_blocks.is_empty() {
                        let local_height = if let Some(ref tc_ref) = tendermint {
                            let tc = safe_lock(tc_ref);
                            tc.height().saturating_sub(1)
                        } else {
                            let c = safe_lock(&consensus);
                            c.block_number()
                        };
                        let next_needed = local_height + 1;
                        let first_pending = *pending_blocks.keys().next().unwrap();
                        if first_pending > next_needed {
                            if let Some(ref sender) = sync_request_sender {
                                println!(
                                    "{} \x1b[36mStill missing blocks {}..{} — requesting more\x1b[0m",
                                    node_tag, next_needed, first_pending - 1
                                );
                                let _ = sender.send((next_needed, first_pending - 1)).await;
                                sync_in_flight = true;
                            }
                        }
                    }
                }
            }

            // ── Receive peer tip height (on new connection, triggers initial sync) ──
            Some(tip_height) = async {
                match tip_receiver.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending::<Option<u64>>().await,
                }
            } => {
                if tip_height == 0 {
                    continue; // Empty tip probe response
                }
                let local_height = if let Some(ref tc) = tendermint {
                    let tc = safe_lock(tc);
                    tc.height().saturating_sub(1)
                } else {
                    let c = safe_lock(&consensus);
                    c.block_number()
                };
                if tip_height > local_height && !sync_in_flight {
                    println!(
                        "{} \x1b[36mPeer tip is #{}, local is #{} — requesting catch-up\x1b[0m",
                        node_tag, tip_height, local_height
                    );
                    if let Some(ref sender) = sync_request_sender {
                        let _ = sender.send((local_height + 1, tip_height)).await;
                        sync_in_flight = true;
                    }
                }
            }

            // ── Receive transactions from network (producer adds to mempool) ──
            Some(tx) = async {
                if !is_producer {
                    // Followers don't need to collect txs into mempool
                    return std::future::pending::<Option<Transaction>>().await;
                }
                match net_tx_receiver.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending::<Option<Transaction>>().await,
                }
            } => {
                let mut c = safe_lock(&consensus);
                c.mempool.submit(tx);
            }

            // ── Receive DA shard sample responses from peers ──
            Some(samples) = async {
                match sample_response_receiver.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending::<Option<Vec<evaporchain_da::sampling::SampleResponse>>>().await,
                }
            } => {
                let verified = samples.iter().all(|s| {
                    let computed: [u8; 32] = blake3::hash(&s.shard.data).into();
                    computed == s.shard.hash && evaporchain_da::sampling::DASampler::verify_proof(&s.shard, &s.proof)
                });
                if verified && !samples.is_empty() {
                    println!(
                        "{} \x1b[36mDA: verified {} peer shard samples\x1b[0m",
                        node_tag, samples.len()
                    );
                    // Create and broadcast DA attestation based on verified peer samples
                    if let Some(ref tc_ref) = tendermint {
                        let mut tc = safe_lock(tc_ref);
                        let current_height = tc.height();
                        // Attest to the previous block (the one we just sampled)
                        let sampled_block = current_height.saturating_sub(1);
                        if let Some(data_root) = {
                            let store = safe_lock(&da_store);
                            store.get(&sampled_block).map(|p| p.header.commitment_root)
                        } {
                            let shard_count = {
                                let store = safe_lock(&da_store);
                                store.get(&sampled_block).map(|p| p.shards.len() as u32).unwrap_or(8)
                            };
                            if let Some(att_msg) = tc.make_da_attestation(sampled_block, data_root, shard_count) {
                                tc.on_message(att_msg.clone());
                                if let Some(cert_bytes) = tc.try_build_da_certificate(sampled_block, data_root) {
                                    println!(
                                        "{}   \x1b[1;35mDA Certificate: block #{}, supermajority via peer samples\x1b[0m",
                                        node_tag, sampled_block,
                                    );
                                    let mut fs = safe_lock(&frontier_state);
                                    fs.poha.register(sampled_block, data_root, 8, 3000, 4000, sampled_block, vec![], vec![]);
                                    drop(fs);
                                }
                                drop(tc);
                                if let Some(ref sender) = consensus_net_sender {
                                    if let Ok(data) = serde_json::to_vec(&att_msg) {
                                        let _ = sender.send(data).await;
                                    }
                                }
                            }
                        }
                    }
                } else if !samples.is_empty() {
                    eprintln!(
                        "{} \x1b[31mDA: peer shard sample verification FAILED ({} samples)\x1b[0m",
                        node_tag, samples.len()
                    );
                }
            }
        }
    }
}

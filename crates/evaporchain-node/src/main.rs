mod api;
mod auth;
mod persistence;
mod user_db;

use anyhow::Result;
use api::{ApiState, BlockRecord, ChainStats, EpochSnapshot, EventRecord, NftStore, NftToken, TokenStore, DeployedToken, StakingStore, StakingPool, Staker, DAOStore, DAOProposal, DAOVote};
use evaporchain_consensus::MockConsensus;
use evaporchain_network::service::{cache_block, NetworkConfig, P2pNetworkService};
use evaporchain_proving::{MockProver, ProvingEngine};
use evaporchain_state::db::StateDB;
use evaporchain_state::RocksDBStateDB;
use evaporchain_crypto::signatures::{MlDsaKeypair, Signer};
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
use tokio::time::{interval, Duration};

// ──────────────────────────── Configuration ─────────────────────────────

const GRACE_PERIOD: u64 = 5;
const BLOCK_INTERVAL_MS: u64 = 1000;
const DEMO_TX_CHANCE: f64 = 0.4; // 40% chance of a demo tx each block

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

fn initialize_genesis(db: &mut RocksDBStateDB, node_tag: &str) {
    use api::{GENESIS_FOUNDATION, GENESIS_CORE_DEV, GENESIS_VALIDATOR1, GENESIS_VALIDATOR2, GENESIS_ECOSYSTEM, GENESIS_COMMUNITY, parse_hex_address};

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
        (0x10, GENESIS_FOUNDATION, 50_000, 200, "token:evap-governance"),
        (0x11, GENESIS_CORE_DEV, 30_000, 150, "stake:validator-pool-1"),
        (0x12, GENESIS_ECOSYSTEM, 5_000, 50, "nft:event-ticket-0x3f"),
        (0x13, GENESIS_ECOSYSTEM, 8_000, 60, "escrow:freelance-0x8b"),
        (0x14, GENESIS_VALIDATOR2, 2_000, 30, "dao:proposal-0x5e"),
        (0x15, GENESIS_COMMUNITY, 80, 4, "session:auth-0x1a"),
        (0x16, GENESIS_VALIDATOR1, 40, 3, "cache:price-feed-0x9c"),
        (0x17, GENESIS_COMMUNITY, 15, 2, "msg:ephemeral-0xd7"),
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
) -> Option<Transaction> {
    use api::{GENESIS_FOUNDATION, GENESIS_CORE_DEV, GENESIS_VALIDATOR1, GENESIS_VALIDATOR2, GENESIS_ECOSYSTEM, GENESIS_COMMUNITY, parse_hex_address};

    let roll: f64 = rng.gen();
    if roll > DEMO_TX_CHANCE {
        return None;
    }

    let acct_hexes: [&str; 6] = [
        GENESIS_FOUNDATION, GENESIS_CORE_DEV, GENESIS_VALIDATOR1,
        GENESIS_VALIDATOR2, GENESIS_ECOSYSTEM, GENESIS_COMMUNITY,
    ];
    let obj_ids: [u8; 8] = [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17];
    let prefixes = ["swap:", "lock:", "vote:", "proof:", "cert:", "stream:", "relay:", "index:"];

    let action = rng.gen_range(0u8..10);

    match action {
        0..=4 => {
            let fi = rng.gen_range(0..acct_hexes.len());
            let mut ti = rng.gen_range(0..acct_hexes.len());
            while ti == fi { ti = rng.gen_range(0..acct_hexes.len()); }
            let from = parse_hex_address(acct_hexes[fi]).unwrap();
            let to = parse_hex_address(acct_hexes[ti]).unwrap();
            let amount = rng.gen_range(100..5000);
            let slot = (fi % 4) as usize;
            let nonce = nonces[slot];
            nonces[slot] += 1;
            let mut tx = Transaction::Transfer(TransferTx {
                from,
                to,
                amount,
                nonce,
                signature: None,
                public_key: None,
            });
            // Sign with sender's keypair
            let msg = tx.signable_bytes();
            let sig = keypairs[fi].sign(&msg);
            let pk = keypairs[fi].public_key_bytes();
            if let Transaction::Transfer(ref mut inner) = tx {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Some(tx)
        }
        5 | 6 => {
            let oid = 0x20 + (epoch % 200) as u8;
            let energy = rng.gen_range(15..120);
            let half_life = rng.gen_range(2..8);
            let ci = rng.gen_range(0..acct_hexes.len());
            let creator = parse_hex_address(acct_hexes[ci]).unwrap();
            let prefix = prefixes[rng.gen_range(0..prefixes.len())];
            let name = format!("{}0x{:02x}{:02x}", prefix, oid, (epoch % 256) as u8);
            let mut tx = Transaction::CreateObject(CreateObjectTx {
                creator,
                object_id: obj_id(oid),
                energy,
                half_life,
                data: name.into_bytes(),
                signature: None,
                public_key: None,
            });
            let msg = tx.signable_bytes();
            let sig = keypairs[ci].sign(&msg);
            let pk = keypairs[ci].public_key_bytes();
            if let Transaction::CreateObject(ref mut inner) = tx {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Some(tx)
        }
        7 | 8 => {
            let target = obj_ids[rng.gen_range(0..obj_ids.len())];
            let deposit = rng.gen_range(100..800);
            let si = rng.gen_range(0..keypairs.len());
            let mut tx = Transaction::Refresh(RefreshTx {
                object_id: obj_id(target),
                energy_deposit: deposit,
                signature: None,
                public_key: None,
            });
            let msg = tx.signable_bytes();
            let sig = keypairs[si].sign(&msg);
            let pk = keypairs[si].public_key_bytes();
            if let Transaction::Refresh(ref mut inner) = tx {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Some(tx)
        }
        _ => {
            let target = obj_ids[rng.gen_range(0..5)];
            let si = rng.gen_range(0..keypairs.len());
            let mut tx = Transaction::Refresh(RefreshTx {
                object_id: obj_id(target),
                energy_deposit: rng.gen_range(500..5000),
                signature: None,
                public_key: None,
            });
            let msg = tx.signable_bytes();
            let sig = keypairs[si].sign(&msg);
            let pk = keypairs[si].public_key_bytes();
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
    block: &evaporchain_types::Block,
    execution: &BlockExecutionResult,
    active_objects: usize,
    ghost_count: usize,
) {
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
        transactions: api::tx_records_from_block(block),
    };

    // Push to block history
    {
        let mut history = block_history.lock().unwrap();
        history.push_back(record);
        while history.len() > 500 {
            history.pop_front();
        }
    }

    // Update stats
    {
        let mut stats = chain_stats.lock().unwrap();
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
            half_life: 500,
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
            energy: 800,
            max_energy: 800,
            half_life: 10,
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
            energy: 20_000,
            max_energy: 20_000,
            half_life: 50,
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
            energy: 5_000,
            max_energy: 5_000,
            half_life: 25,
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
            energy: 50_000,
            max_energy: 50_000,
            half_life: 100,
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
            energy: 200,
            max_energy: 200,
            half_life: 5,
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
                total_supply: 962_716, decay_half_life: 1000,
                deployed_epoch: 0, deployer: f(GENESIS_FOUNDATION),
                balances: evap_balances, last_decay_epoch: 0,
            },
            DeployedToken {
                id: 2, name: "Flux Token".into(), symbol: "FLUX".into(),
                total_supply: 183_272, decay_half_life: 20,
                deployed_epoch: 0, deployer: f(GENESIS_FOUNDATION),
                balances: flux_balances, last_decay_epoch: 0,
            },
            DeployedToken {
                id: 3, name: "Thermal Credits".into(), symbol: "HEAT".into(),
                total_supply: 14_258, decay_half_life: 5,
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
                reward_decay_hl: 50,
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
                created_epoch: 0, voting_period: 200, creator: f(GENESIS_VALIDATOR1),
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
                created_epoch: 0, voting_period: 150, creator: f(GENESIS_FOUNDATION),
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
                created_epoch: 0, voting_period: 300, creator: f(GENESIS_ECOSYSTEM),
                status: "Active".into(), evaporated_epoch: None,
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
        println!("{} \x1b[1mFresh start — loading genesis state:\x1b[0m", node_tag);
        let mut db = db.lock().unwrap();
        initialize_genesis(&mut db, &node_tag);
    } else {
        println!("{} \x1b[1;32mResuming from persistent state\x1b[0m", node_tag);
        let db = db.lock().unwrap();
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
    let prover: Arc<Mutex<Box<dyn ProvingEngine>>> = if args.prove_mode {
        #[cfg(feature = "prove")]
        {
            println!(
                "{} \x1b[1;33mProving mode active\x1b[0m — setting up Nova IVC (real blocks)...",
                node_tag
            );
            let genesis_commitment = {
                let db = db.lock().unwrap();
                evaporchain_types::DualCommitment {
                    verkle_root: db.compute_state_root(),
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
            Arc::new(Mutex::new(
                Box::new(real_prover) as Box<dyn ProvingEngine>
            ))
        }
        #[cfg(not(feature = "prove"))]
        {
            eprintln!("\x1b[31m--prove requires the 'prove' feature. Recompile with: cargo build -p evaporchain-node --features prove\x1b[0m");
            std::process::exit(1);
        }
    } else {
        Arc::new(Mutex::new(
            Box::new(MockProver::new()) as Box<dyn ProvingEngine>
        ))
    };

    // ── Network setup ──
    let mut peer_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let net_channels = if args.network_mode {
        let net_config = NetworkConfig {
            listen_address: format!("/ip4/0.0.0.0/tcp/{}", args.port),
            bootstrap_peers: args.bootstrap_peers.clone(),
            channel_buffer: 256,
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

    let role_str = if is_producer { "Producer" } else { "Follower" };
    println!(
        "{} Role: {} | Block interval: {}ms | Grace: {} epochs | Proving: {} | Network: {}",
        node_tag,
        role_str,
        args.block_ms,
        GRACE_PERIOD,
        if args.prove_mode {
            "Nova IVC (real blocks)"
        } else {
            "Mock"
        },
        if args.network_mode { "ON" } else { "OFF" }
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
    let consensus = Arc::new(Mutex::new(MockConsensus::new(GRACE_PERIOD)));

    // Restore consensus state from disk if available
    if !is_fresh {
        if let Some((block_number, epoch, parent_hash)) = chain_store.load_consensus_meta() {
            let mut c = consensus.lock().unwrap();
            c.restore_state(block_number, epoch, parent_hash);
            println!(
                "{} \x1b[1;32mConsensus restored:\x1b[0m block={}, epoch={}, parent_hash={}…",
                node_tag, block_number, epoch, &hex::encode(parent_hash)[..16]
            );
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
    let events: Arc<Mutex<VecDeque<EventRecord>>> = Arc::new(Mutex::new(
        if is_fresh {
            VecDeque::with_capacity(200)
        } else {
            chain_store.load_events()
        }
    ));
    let start_time = Instant::now();

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
            chain_store.save_nft_store(&ns);
            chain_store.save_token_store(&ts);
            chain_store.save_staking_store(&ss);
            chain_store.save_dao_store(&ds);
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
                            let mut c = consensus_tx.lock().unwrap();
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
        )
    } else {
        (None, None, None, None, None, None, None, None)
    };

    // Block queue for out-of-order blocks (gap filling)
    let mut pending_blocks: BTreeMap<u64, evaporchain_types::Block> = BTreeMap::new();
    // Track whether we have an outstanding sync request to avoid duplicate requests
    let mut sync_in_flight = false;

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
        prover: &Arc<Mutex<Box<dyn ProvingEngine>>>,
        prove_mode: bool,
        block_history: &Arc<Mutex<VecDeque<BlockRecord>>>,
        chain_stats: &Arc<Mutex<ChainStats>>,
        events: &Arc<Mutex<VecDeque<EventRecord>>>,
        chain_store: &Arc<ChainStore>,
        peer_count: &Arc<std::sync::atomic::AtomicUsize>,
        block_cache: &Option<evaporchain_network::service::BlockCache>,
    ) -> Option<(usize, usize)> {
        let mut c = consensus.lock().unwrap();
        let mut db_guard = db.lock().unwrap();

        // Only apply if this block advances our chain
        if block.number <= c.block_number() {
            return None; // stale
        }

        match c.apply_block(&mut *db_guard, block) {
            Ok(result) => {
                db_guard.flush_accounts();
                db_guard.flush_objects();

                let old_root = block.parent_hash;
                let new_root = result.execution.state_root;
                let mut p = prover.lock().unwrap();
                if let Err(e) = p.fold_block(&result.block, old_root, new_root) {
                    eprintln!("{} \x1b[31mProving error: {}\x1b[0m", node_tag, e);
                } else if prove_mode {
                    println!(
                        "{}   \x1b[35mProof: fold={:.1}ms  acc={}B  folded={}\x1b[0m",
                        node_tag,
                        p.last_fold_time_us() as f64 / 1000.0,
                        p.accumulator_size(),
                        p.num_blocks_folded(),
                    );
                }
                drop(p);

                let obj_count = db_guard.object_count();
                let ghost_count_val = db_guard.ghost_count();
                let peers = peer_count.load(std::sync::atomic::Ordering::Relaxed);

                record_block(
                    block_history, chain_stats, events,
                    &result.block, &result.execution,
                    obj_count, ghost_count_val,
                );

                chain_store.save_consensus_meta(
                    result.block.number, result.block.epoch, result.block.parent_hash,
                );
                {
                    let history = block_history.lock().unwrap();
                    if let Some(record) = history.back() {
                        chain_store.save_block(record);
                    }
                }
                {
                    let stats = chain_stats.lock().unwrap();
                    chain_store.save_chain_stats(&stats);
                }
                {
                    let ev = events.lock().unwrap();
                    chain_store.save_events(&ev);
                }

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

    // ── Block production / follower loop ──
    let mut ticker = interval(Duration::from_millis(args.block_ms));
    let mut rng = rand::thread_rng();
    let mut demo_nonces = [0u64; 4];

    loop {
        tokio::select! {
            // ── Tick: producer creates a block ──
            _ = ticker.tick(), if is_producer => {
                // In demo mode, inject random transactions
                if args.demo_mode {
                    let epoch = {
                        let c = consensus.lock().unwrap();
                        c.epoch() + 1
                    };
                    if let Some(tx) = generate_demo_tx(&mut rng, epoch, &mut demo_nonces, &demo_keypairs) {
                        // Also broadcast the tx to peers
                        if let Some(ref sender) = net_tx_sender {
                            let _ = sender.send(tx.clone()).await;
                        }
                        let mut c = consensus.lock().unwrap();
                        c.mempool.submit(tx);
                    }
                }

                // Drain any txs received from the network into the mempool
                if let Some(ref mut rx) = net_tx_receiver {
                    while let Ok(tx) = rx.try_recv() {
                        let mut c = consensus.lock().unwrap();
                        c.mempool.submit(tx);
                    }
                }

                // Produce block — all synchronous work under locks, then drop before await
                let produced = {
                    let mut c = consensus.lock().unwrap();
                    let mut db_guard = db.lock().unwrap();

                    match c.produce_block(&mut *db_guard) {
                        Ok(result) => {
                            let old_root = result.block.parent_hash;
                            let new_root = result.execution.state_root;
                            let mut p = prover.lock().unwrap();
                            if let Err(e) = p.fold_block(&result.block, old_root, new_root) {
                                eprintln!("{} \x1b[31mProving error: {}\x1b[0m", node_tag, e);
                            } else if args.prove_mode {
                                println!(
                                    "{}   \x1b[35mProof: fold={:.1}ms  acc={}B  folded={}\x1b[0m",
                                    node_tag,
                                    p.last_fold_time_us() as f64 / 1000.0,
                                    p.accumulator_size(),
                                    p.num_blocks_folded(),
                                );
                            }
                            drop(p);

                            // Flush mutated state to RocksDB
                            db_guard.flush_accounts();
                            db_guard.flush_objects();

                            let obj_count = db_guard.object_count();
                            let ghost_count = db_guard.ghost_count();
                            Some((result, obj_count, ghost_count))
                        }
                        Err(e) => {
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
                    record_block(
                        &block_history,
                        &chain_stats,
                        &events,
                        &result.block,
                        &result.execution,
                        obj_count,
                        ghost_count,
                    );

                    // Persist chain data to disk
                    chain_store.save_consensus_meta(
                        result.block.number,
                        result.block.epoch,
                        result.block.parent_hash,
                    );
                    {
                        let history = block_history.lock().unwrap();
                        if let Some(record) = history.back() {
                            chain_store.save_block(record);
                        }
                    }
                    {
                        let stats = chain_stats.lock().unwrap();
                        chain_store.save_chain_stats(&stats);
                    }
                    {
                        let ev = events.lock().unwrap();
                        chain_store.save_events(&ev);
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
                let local_height = {
                    let c = consensus.lock().unwrap();
                    c.block_number()
                };

                // Skip stale blocks
                if block.number <= local_height {
                    println!(
                        "{} \x1b[90mSkipping stale block #{} (local={})\x1b[0m",
                        node_tag, block.number, local_height
                    );
                    continue;
                }

                let expected_next = local_height + 1;

                if block.number == expected_next {
                    // Perfect: next block in sequence — apply it
                    apply_follower_block(
                        &node_tag, &block, &consensus, &db, &prover,
                        args.prove_mode, &block_history, &chain_stats, &events,
                        &chain_store, &peer_count, &block_cache,
                    );

                    // After applying, drain any pending blocks that are now in sequence
                    loop {
                        let next = {
                            let c = consensus.lock().unwrap();
                            c.block_number() + 1
                        };
                        if let Some(queued) = pending_blocks.remove(&next) {
                            apply_follower_block(
                                &node_tag, &queued, &consensus, &db, &prover,
                                args.prove_mode, &block_history, &chain_stats, &events,
                                &chain_store, &peer_count, &block_cache,
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
                        let local_height = {
                            let c = consensus.lock().unwrap();
                            c.block_number()
                        };
                        if block.number == local_height + 1 {
                            apply_follower_block(
                                &node_tag, block, &consensus, &db, &prover,
                                args.prove_mode, &block_history, &chain_stats, &events,
                                &chain_store, &peer_count, &block_cache,
                            );
                        } else if block.number > local_height + 1 {
                            // Still have a gap — queue and request more
                            pending_blocks.insert(block.number, block.clone());
                        }
                    }

                    // Drain pending queue after applying synced blocks
                    loop {
                        let next = {
                            let c = consensus.lock().unwrap();
                            c.block_number() + 1
                        };
                        if let Some(queued) = pending_blocks.remove(&next) {
                            apply_follower_block(
                                &node_tag, &queued, &consensus, &db, &prover,
                                args.prove_mode, &block_history, &chain_stats, &events,
                                &chain_store, &peer_count, &block_cache,
                            );
                        } else {
                            break;
                        }
                    }

                    // Check if there are still pending blocks (more gaps)
                    if !pending_blocks.is_empty() {
                        let local_height = {
                            let c = consensus.lock().unwrap();
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
                let local_height = {
                    let c = consensus.lock().unwrap();
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
                let mut c = consensus.lock().unwrap();
                c.mempool.submit(tx);
            }
        }
    }
}

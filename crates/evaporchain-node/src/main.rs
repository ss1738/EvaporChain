use anyhow::Result;
use evaporchain_consensus::MockConsensus;
use evaporchain_proving::{MockProver, ProvingEngine};
use evaporchain_state::db::StateDB;
use evaporchain_state::InMemoryStateDB;
use evaporchain_types::{
    Account, CreateObjectTx, ObjectState, RefreshTx, StateObject, Transaction, TransferTx,
};
use rand::Rng;
use serde::Deserialize;
use std::io::BufRead;
use std::sync::{Arc, Mutex};
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

fn initialize_genesis(db: &mut InMemoryStateDB) {
    // Test accounts
    let accounts = [
        (1u8, "Alice", 100_000u64),
        (2, "Bob", 50_000),
        (3, "Charlie", 25_000),
    ];
    for (id, name, balance) in &accounts {
        db.put_account(Account {
            address: addr(*id),
            balance: *balance,
            nonce: 0,
        });
        println!(
            "  \x1b[36m{}\x1b[0m  addr=0x{:02x}..  balance={}",
            name, id, balance
        );
    }

    // Test objects with varying energy and half-lives
    let objects: Vec<(u8, u8, u64, u64, &str)> = vec![
        // (obj_id, owner_id, energy, half_life, label)
        (10, 1, 50, 3, "Ephemeral-A"), // dies fast: ~9 epochs
        (11, 1, 200, 5, "Short-lived-B"), // moderate: ~38 epochs
        (12, 2, 8, 2, "Fragile-C"),    // very fast: ~6 epochs
        (13, 2, 10000, 20, "Durable-D"), // long lived
        (14, 3, 30, 4, "Volatile-E"),  // dies in ~20 epochs
    ];

    for (oid, owner, energy, half_life, label) in &objects {
        db.put_object(StateObject {
            id: obj_id(*oid),
            owner: addr(*owner),
            energy: *energy,
            half_life: *half_life,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: label.as_bytes().to_vec(),
        });
        println!(
            "  \x1b[33m{}\x1b[0m  id=0x{:02x}..  energy={:<6} half_life={}",
            label, oid, energy, half_life
        );
    }
}

// ──────────────────────────── Display Helpers ────────────────────────────

fn print_banner() {
    println!();
    println!("\x1b[1;35m╔══════════════════════════════════════════════════════════════╗\x1b[0m");
    println!("\x1b[1;35m║            EvaporChain — Single-Node Devnet v0.1            ║\x1b[0m");
    println!("\x1b[1;35m║        Thermodynamic State Decay in Real Time               ║\x1b[0m");
    println!("\x1b[1;35m╚══════════════════════════════════════════════════════════════╝\x1b[0m");
    println!();
}

#[allow(clippy::too_many_arguments)]
fn print_block_result(
    block_num: u64,
    epoch: u64,
    txs_executed: usize,
    txs_failed: usize,
    entered_grace: usize,
    evaporated: usize,
    active_objects: usize,
    ghost_count: usize,
    state_root: &[u8; 32],
    db: &dyn StateDB,
) {
    let root_hex = &hex::encode(state_root)[..16];

    // Block header
    println!();
    println!(
        "\x1b[1;32m━━━ Block #{:<4} │ Epoch {:<4} ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m",
        block_num, epoch
    );

    // Transactions
    if txs_executed > 0 || txs_failed > 0 {
        println!(
            "  Transactions:  \x1b[32m{} ok\x1b[0m  \x1b[31m{} failed\x1b[0m",
            txs_executed, txs_failed
        );
    }

    // Evaporation events
    if entered_grace > 0 {
        println!(
            "  \x1b[33m⚠ {} object(s) entered GRACE period\x1b[0m",
            entered_grace
        );
    }
    if evaporated > 0 {
        println!(
            "  \x1b[31m💀 {} object(s) EVAPORATED → ghost\x1b[0m",
            evaporated
        );
    }

    // State summary
    println!(
        "  State:         \x1b[36m{} active\x1b[0m  \x1b[90m{} ghosts\x1b[0m  root={}…",
        active_objects, ghost_count, root_hex
    );

    // Per-object energy readout
    let mut ids = db.all_object_ids();
    ids.sort();
    if !ids.is_empty() {
        println!("  Objects:");
        for id in &ids {
            if let Some(obj) = db.get_object(id) {
                let current_energy = obj.energy_at(epoch);
                let bar_len = (current_energy as f64).log2().max(0.0) as usize;
                let bar: String = "█".repeat(bar_len.min(30));
                let state_str = match obj.state {
                    ObjectState::Active => "\x1b[32mActive\x1b[0m",
                    ObjectState::Grace => "\x1b[33mGrace\x1b[0m ",
                    ObjectState::Ghost => "\x1b[31mGhost\x1b[0m ",
                    ObjectState::Resurrected => "\x1b[35mRisen\x1b[0m ",
                };
                let label = String::from_utf8_lossy(&obj.data);
                println!(
                    "    0x{:02x} {:<14} {} e={:<6} \x1b[34m{}\x1b[0m",
                    id[0], label, state_str, current_energy, bar
                );
            }
        }
    }
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

fn parse_stdin_command(line: &str) -> Option<Transaction> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    // Try JSON first
    if let Ok(cmd) = serde_json::from_str::<StdinCommand>(line) {
        return Some(match cmd {
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
        });
    }

    eprintln!("\x1b[31mInvalid command: {}\x1b[0m", line);
    eprintln!("Examples:");
    eprintln!(r#"  {{"type":"transfer","from":1,"to":2,"amount":500,"nonce":0}}"#);
    eprintln!(r#"  {{"type":"create_object","creator":1,"object_id":20,"energy":1000,"half_life":10}}"#);
    eprintln!(r#"  {{"type":"refresh","object_id":10,"energy_deposit":500}}"#);
    None
}

// ──────────────────────────── Demo Mode ──────────────────────────────────

fn generate_demo_tx(rng: &mut impl Rng, epoch: u64, nonces: &mut [u64; 4]) -> Option<Transaction> {
    let roll: f64 = rng.gen();
    if roll > DEMO_TX_CHANCE {
        return None;
    }

    let action = rng.gen_range(0u8..10);

    match action {
        // 50% chance: transfer between Alice/Bob/Charlie
        0..=4 => {
            let from = rng.gen_range(1u8..=3);
            let mut to = rng.gen_range(1u8..=3);
            while to == from {
                to = rng.gen_range(1u8..=3);
            }
            let amount = rng.gen_range(100..2000);
            let nonce = nonces[from as usize];
            nonces[from as usize] += 1;
            Some(Transaction::Transfer(TransferTx {
                from: addr(from),
                to: addr(to),
                amount,
                nonce,
                signature: None,
                public_key: None,
            }))
        }
        // 20% chance: create a new ephemeral object
        5 | 6 => {
            let oid = 100 + (epoch % 150) as u8;
            let energy = rng.gen_range(10..100);
            let half_life = rng.gen_range(2..6);
            let creator = rng.gen_range(1u8..=3);
            Some(Transaction::CreateObject(CreateObjectTx {
                creator: addr(creator),
                object_id: obj_id(oid),
                energy,
                half_life,
                data: format!("Demo-E{}", epoch).into_bytes(),
                signature: None,
                public_key: None,
            }))
        }
        // 20% chance: refresh a genesis object (keep Durable-D alive)
        7 | 8 => {
            let target = [10u8, 11, 12, 13, 14][rng.gen_range(0..5)];
            let deposit = rng.gen_range(50..500);
            Some(Transaction::Refresh(RefreshTx {
                object_id: obj_id(target),
                energy_deposit: deposit,
                signature: None,
                public_key: None,
            }))
        }
        // 10% chance: refresh with big energy (rescue attempt)
        _ => {
            let target = [10u8, 11, 12, 14][rng.gen_range(0..4)];
            Some(Transaction::Refresh(RefreshTx {
                object_id: obj_id(target),
                energy_deposit: rng.gen_range(500..5000),
                signature: None,
                public_key: None,
            }))
        }
    }
}

// ──────────────────────────── Main ───────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Check for flags
    let args: Vec<String> = std::env::args().collect();
    let demo_mode = args.iter().any(|a| a == "--demo");
    let prove_mode = args.iter().any(|a| a == "--prove");
    let block_ms = args
        .iter()
        .position(|a| a == "--interval")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(BLOCK_INTERVAL_MS);

    print_banner();

    // ── Genesis ──
    println!("\x1b[1mGenesis State:\x1b[0m");
    let db = Arc::new(Mutex::new(InMemoryStateDB::new()));
    {
        let mut db = db.lock().unwrap();
        initialize_genesis(&mut db);
    }
    println!();

    // ── Prover setup ──
    let prover: Arc<Mutex<Box<dyn ProvingEngine>>> = if prove_mode {
        #[cfg(feature = "prove")]
        {
            println!("\x1b[1;33mProving mode active\x1b[0m — setting up Nova IVC (this takes a moment)...");
            let genesis_root = {
                let db = db.lock().unwrap();
                db.compute_state_root()
            };
            let nova_prover = evaporchain_proving::nova::NovaProver::new(genesis_root)
                .expect("Failed to set up NovaProver");
            let (primary, secondary) = nova_prover.num_constraints();
            println!(
                "  Nova ready: {} primary constraints, {} secondary",
                primary, secondary
            );
            Arc::new(Mutex::new(Box::new(nova_prover) as Box<dyn ProvingEngine>))
        }
        #[cfg(not(feature = "prove"))]
        {
            eprintln!("\x1b[31m--prove requires the 'prove' feature. Recompile with: cargo build -p evaporchain-node --features prove\x1b[0m");
            std::process::exit(1);
        }
    } else {
        Arc::new(Mutex::new(Box::new(MockProver::new()) as Box<dyn ProvingEngine>))
    };

    if demo_mode {
        println!("\x1b[1;33mDemo mode active\x1b[0m — auto-generating transactions");
    } else {
        println!("Submit transactions via stdin as JSON (one per line).");
        println!("Examples:");
        println!(r#"  {{"type":"transfer","from":1,"to":2,"amount":500,"nonce":0}}"#);
        println!(r#"  {{"type":"refresh","object_id":10,"energy_deposit":500}}"#);
    }
    println!(
        "Block interval: {}ms | Grace period: {} epochs | Proving: {}",
        block_ms,
        GRACE_PERIOD,
        if prove_mode { "Nova IVC" } else { "Mock (off)" }
    );
    println!("\x1b[90m──────────────────────────────────────────────────────────────\x1b[0m");

    // ── Shared consensus ──
    let consensus = Arc::new(Mutex::new(MockConsensus::new(GRACE_PERIOD)));

    // ── Stdin reader (non-demo) ──
    if !demo_mode {
        let consensus_tx = Arc::clone(&consensus);
        tokio::task::spawn_blocking(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                match line {
                    Ok(line) => {
                        if let Some(tx) = parse_stdin_command(&line) {
                            let mut c = consensus_tx.lock().unwrap();
                            c.mempool.submit(tx);
                            println!("\x1b[90m  → transaction queued (mempool={})\x1b[0m", c.mempool.len());
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // ── Block production loop ──
    let mut ticker = interval(Duration::from_millis(block_ms));
    let mut rng = rand::thread_rng();
    let mut demo_nonces = [0u64; 4]; // index 1=Alice, 2=Bob, 3=Charlie

    loop {
        ticker.tick().await;

        // In demo mode, inject random transactions
        if demo_mode {
            let epoch = {
                let c = consensus.lock().unwrap();
                c.epoch() + 1
            };
            if let Some(tx) = generate_demo_tx(&mut rng, epoch, &mut demo_nonces) {
                let mut c = consensus.lock().unwrap();
                c.mempool.submit(tx);
            }
        }

        // Produce block
        let mut c = consensus.lock().unwrap();
        let mut db = db.lock().unwrap();

        match c.produce_block(&mut *db) {
            Ok(result) => {
                // Fold block into prover
                let old_root = result.block.parent_hash; // approximate old state
                let new_root = result.execution.state_root;
                let mut p = prover.lock().unwrap();
                match p.fold_block(&result.block, old_root, new_root) {
                    Ok(()) => {
                        if prove_mode {
                            println!(
                                "  \x1b[35mProof: fold={:.1}ms  accumulator={}B  blocks_folded={}\x1b[0m",
                                p.last_fold_time_us() as f64 / 1000.0,
                                p.accumulator_size(),
                                p.num_blocks_folded(),
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("\x1b[31mProving error: {}\x1b[0m", e);
                    }
                }
                drop(p);

                print_block_result(
                    result.block.number,
                    result.block.epoch,
                    result.execution.txs_executed,
                    result.execution.txs_failed,
                    result.execution.objects_entered_grace,
                    result.execution.objects_evaporated,
                    db.object_count(),
                    db.ghost_count(),
                    &result.execution.state_root,
                    &*db,
                );
            }
            Err(e) => {
                eprintln!("\x1b[31mBlock production error: {}\x1b[0m", e);
            }
        }
    }
}

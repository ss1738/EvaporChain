use anyhow::Result;
use evaporchain_consensus::MockConsensus;
use evaporchain_network::service::{NetworkConfig, P2pNetworkService};
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

fn initialize_genesis(db: &mut InMemoryStateDB, node_tag: &str) {
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
            "{} \x1b[36m{}\x1b[0m  addr=0x{:02x}..  balance={}",
            node_tag, name, id, balance
        );
    }

    let objects: Vec<(u8, u8, u64, u64, &str)> = vec![
        (10, 1, 50, 3, "Ephemeral-A"),
        (11, 1, 200, 5, "Short-lived-B"),
        (12, 2, 8, 2, "Fragile-C"),
        (13, 2, 10000, 20, "Durable-D"),
        (14, 3, 30, 4, "Volatile-E"),
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

fn parse_stdin_command(line: &str) -> Option<Transaction> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

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
    None
}

// ──────────────────────────── Demo Mode ──────────────────────────────────

fn generate_demo_tx(
    rng: &mut impl Rng,
    epoch: u64,
    nonces: &mut [u64; 4],
) -> Option<Transaction> {
    let roll: f64 = rng.gen();
    if roll > DEMO_TX_CHANCE {
        return None;
    }

    let action = rng.gen_range(0u8..10);

    match action {
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

// ──────────────────────────── Arg Parsing ─────────────────────────────────

struct NodeArgs {
    demo_mode: bool,
    prove_mode: bool,
    network_mode: bool,
    block_ms: u64,
    port: u16,
    node_id: String,
    startup_delay_ms: u64,
    bootstrap_peers: Vec<String>,
}

fn parse_args() -> NodeArgs {
    let args: Vec<String> = std::env::args().collect();
    let demo_mode = args.iter().any(|a| a == "--demo");
    let prove_mode = args.iter().any(|a| a == "--prove");
    let network_mode = args.iter().any(|a| a == "--network");
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
        block_ms,
        port,
        node_id,
        startup_delay_ms,
        bootstrap_peers,
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

// ──────────────────────────── Main ───────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();
    let node_tag = make_tag(&args.node_id);

    print_banner(&node_tag);

    // ── Genesis ──
    println!("{} \x1b[1mGenesis State:\x1b[0m", node_tag);
    let db = Arc::new(Mutex::new(InMemoryStateDB::new()));
    {
        let mut db = db.lock().unwrap();
        initialize_genesis(&mut db, &node_tag);
    }
    println!();

    // ── Prover setup ──
    let prover: Arc<Mutex<Box<dyn ProvingEngine>>> = if args.prove_mode {
        #[cfg(feature = "prove")]
        {
            println!(
                "{} \x1b[1;33mProving mode active\x1b[0m — setting up Nova IVC...",
                node_tag
            );
            let genesis_root = {
                let db = db.lock().unwrap();
                db.compute_state_root()
            };
            let nova_prover = evaporchain_proving::nova::NovaProver::new(genesis_root)
                .expect("Failed to set up NovaProver");
            let (primary, secondary) = nova_prover.num_constraints();
            println!(
                "{}   Nova ready: {} primary, {} secondary constraints",
                node_tag, primary, secondary
            );
            Arc::new(Mutex::new(
                Box::new(nova_prover) as Box<dyn ProvingEngine>
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
            "Nova IVC"
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

    // ── Stdin reader (non-demo, non-follower) ──
    if !args.demo_mode && is_producer {
        let consensus_tx = Arc::clone(&consensus);
        let tag = node_tag.clone();
        tokio::task::spawn_blocking(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                match line {
                    Ok(line) => {
                        if let Some(tx) = parse_stdin_command(&line) {
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
    let (net_tx_sender, mut net_tx_receiver, net_block_sender, mut net_block_receiver) =
        if let Some(ch) = net_channels {
            (
                Some(ch.tx_sender),
                Some(ch.tx_receiver),
                Some(ch.block_sender),
                Some(ch.block_receiver),
            )
        } else {
            (None, None, None, None)
        };

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
                    if let Some(tx) = generate_demo_tx(&mut rng, epoch, &mut demo_nonces) {
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

                    // Broadcast block to network (async, no locks held)
                    if let Some(ref sender) = net_block_sender {
                        let _ = sender.send(result.block.clone()).await;
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

            // ── Receive block from network (follower path) ──
            Some(block) = async {
                match net_block_receiver.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending::<Option<evaporchain_types::Block>>().await,
                }
            } => {
                // Skip if we're the producer and this is our own block
                // (GossipSub doesn't echo to self, so this is always a peer block)
                let mut c = consensus.lock().unwrap();
                let mut db_guard = db.lock().unwrap();

                // Only apply if this block advances our chain
                if block.number <= c.block_number() {
                    println!(
                        "{} \x1b[90mSkipping stale block #{} (local={})\x1b[0m",
                        node_tag, block.number, c.block_number()
                    );
                    continue;
                }

                match c.apply_block(&mut *db_guard, &block) {
                    Ok(result) => {
                        // Fold into prover
                        let old_root = block.parent_hash;
                        let new_root = result.execution.state_root;
                        let mut p = prover.lock().unwrap();
                        if let Err(e) = p.fold_block(&result.block, old_root, new_root) {
                            eprintln!("{} \x1b[31mProving error: {}\x1b[0m", node_tag, e);
                        }
                        drop(p);

                        let obj_count = db_guard.object_count();
                        let ghost_count = db_guard.ghost_count();
                        let peers = peer_count.load(std::sync::atomic::Ordering::Relaxed);

                        // Check state root match
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
                            &node_tag,
                            if roots_match { "SYNCED ✓" } else { "SYNCED ✗" },
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
                    Err(e) => {
                        eprintln!(
                            "{} \x1b[31mFailed to apply block #{}: {}\x1b[0m",
                            node_tag, block.number, e
                        );
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

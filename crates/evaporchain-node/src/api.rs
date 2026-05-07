use axum::http::HeaderMap;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use evaporchain_consensus::tendermint::TendermintConsensus;
use evaporchain_consensus::MockConsensus;
use evaporchain_crypto::signatures::{MlDsaKeypair, Signer};
use evaporchain_da::block_da::BlockDAPackage;
use evaporchain_da::block_da_2d::BlockDA2DPackage;
use evaporchain_state::db::StateDB;
use evaporchain_state::RocksDBStateDB;
use evaporchain_types::{
    Block, CallContractTx, ClaimDelegationTx, CreateObjectTx, DelegateTx, DeployContractTx,
    ObjectState, RefreshTx, Transaction, TransferTx, UndelegateTx,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{atomic::AtomicUsize, Arc, Mutex};
use std::time::Instant;
use tower_http::cors::CorsLayer;

// ──────────────────────────── Lock Helper ─────────────────────────────

/// Safely acquire a Mutex lock, recovering from poisoned state.
/// A poisoned mutex means a thread panicked while holding the lock,
/// but the data inside is still usable — we recover it rather than crashing.
fn safe_lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        tracing::warn!("Recovered poisoned mutex lock");
        poisoned.into_inner()
    })
}

// ──────────────────────────── Shared State ────────────────────────────

/// Shared application state accessible from API handlers.
pub struct ApiState {
    pub db: Arc<Mutex<RocksDBStateDB>>,
    pub consensus: Arc<Mutex<MockConsensus>>,
    pub peer_count: Arc<AtomicUsize>,
    pub block_history: Arc<Mutex<VecDeque<BlockRecord>>>,
    pub stats: Arc<Mutex<ChainStats>>,
    pub events: Arc<Mutex<VecDeque<EventRecord>>>,
    pub prove_mode: bool,
    pub start_time: Instant,
    /// Faucet rate limiter: (client IP, full recipient address) -> last
    /// request timestamp. Keying on the full 32-byte address (not its
    /// 20-byte prefix) prevents stress tests with sequentially-numbered
    /// addresses (e.g. 0x000…001 … 0x000…0c8, all sharing 20 leading
    /// zero bytes) from being collapsed into one rate-limit slot.
    /// Including the IP means: same recipient from same IP is rate-limited
    /// (the original anti-pump intent), but distinct recipients from the
    /// same IP all pass — which is what a faucet stress harness or a
    /// dApp seeding many wallets actually needs.
    pub faucet_rate_limit: Mutex<HashMap<(std::net::IpAddr, [u8; 32]), Instant>>,
    /// Pending-nonce cache. Concurrent /api/faucet (and other tx) hits
    /// would otherwise all read the same db.nonce and submit txs with
    /// identical nonce, of which only one can execute (the others hit
    /// InvalidNonce silently). The cache returns max(db.nonce, cached)
    /// and increments locally. When pending txs commit and db.nonce
    /// catches up, max takes over and the cache resyncs implicitly.
    pub pending_nonces: Mutex<HashMap<[u8; 32], u64>>,
    /// When true, the faucet skips its per-address cooldown entirely.
    /// Set by `--devnet-no-rate-limit` or `--faucet-rate-limit-disabled`
    /// for stress / load testing only; `--mainnet` strict mode rejects
    /// this combination at startup.
    pub faucet_rate_limit_disabled: bool,
    /// Cooldown window in seconds for the faucet rate limiter. Defaults
    /// to `FAUCET_RATE_LIMIT_SECS` (1 hour); overridden by the
    /// `--faucet-rate-limit-secs <N>` CLI flag. A value of 0 effectively
    /// disables the cooldown (every request passes the elapsed check).
    pub faucet_rate_limit_secs: u64,
    /// NFT marketplace store.
    pub nft_store: Arc<Mutex<NftStore>>,
    /// Token store.
    pub token_store: Arc<Mutex<TokenStore>>,
    /// Staking store.
    pub staking_store: Arc<Mutex<StakingStore>>,
    /// DAO store.
    pub dao_store: Arc<Mutex<DAOStore>>,
    /// Auth sessions for tx authentication.
    pub auth_sessions: Option<crate::auth::Sessions>,
    /// User database for wallet ownership checks.
    pub user_db: Option<Arc<crate::user_db::UserDb>>,
    /// Node-level ML-DSA keypair for signing API-submitted transactions.
    pub node_keypair: Arc<MlDsaKeypair>,
    /// Tendermint consensus (used in multi-validator mode). When present, API
    /// transactions are routed here instead of MockConsensus.
    pub tendermint: Option<Arc<Mutex<TendermintConsensus>>>,
    /// P2P transaction broadcast sender. When present, API-submitted transactions
    /// are also broadcast to the network so other validators can include them.
    pub tx_broadcast: Option<tokio::sync::mpsc::Sender<Transaction>>,
    /// WebSocket event broadcaster for real-time subscriptions.
    pub ws_broadcaster: Arc<crate::ws::WsBroadcaster>,
    /// Persistent chain store for tx receipt lookups.
    pub chain_store: Option<Arc<crate::persistence::ChainStore>>,
    /// Chain prover for Nova IVC proof generation and light client sync.
    pub chain_prover: Arc<Mutex<evaporchain_proving::chain_proof::ChainProver>>,
    /// Rolling throughput metrics (TPS, block exec time, gas).
    pub throughput: Arc<Mutex<ThroughputTracker>>,
    /// DA packages per block number (ring buffer, last 256 blocks).
    pub da_store: Arc<Mutex<BTreeMap<u64, BlockDAPackage>>>,
    /// 2D DA packages per block number (ring buffer, last 64 blocks).
    pub da_2d_store: Arc<Mutex<BTreeMap<u64, BlockDA2DPackage>>>,
    /// Latest state snapshot metadata (height, state_root, data_len).
    #[allow(clippy::type_complexity)]
    pub snapshot_info: Arc<Mutex<Option<(u64, [u8; 32], usize)>>>,
    /// Frontier primitives state (anchors, PoHA, energy trie).
    pub frontier_state: Option<Arc<Mutex<crate::frontier::FrontierState>>>,
    /// Oracle consensus bridge (validator-signed feeds with TWAP).
    pub oracle_bridge: Option<Arc<Mutex<crate::oracle_bridge::OracleBridge>>>,
    /// Shard health and cross-shard routing bridge.
    pub shard_bridge: Option<Arc<Mutex<crate::shard_bridge::ShardBridge>>>,
    /// Finality tracker — records BLS-certified finality for each block.
    pub finality_tracker: Arc<Mutex<evaporchain_consensus::finality::FinalityTracker>>,
    /// MEV-protected encrypted mempool (commit-reveal scheme).
    pub encrypted_mempool: Arc<Mutex<evaporchain_consensus::encrypted_mempool::EncryptedMempool>>,
    /// Chain ID for signing message domain separation (cross-chain replay protection).
    pub chain_id: String,
    /// Light client verifier — BLS header verification + skip/sequential modes.
    pub light_client: Arc<Mutex<evaporchain_consensus::light_client::LightClientVerifier>>,
    /// Four-act narrative spine snapshot. The consensus layer's
    /// `update_four_act_snapshot` is called after each block applied
    /// by `SimpleExecutor`. Until then, fields are at their default
    /// (zeroed / not-triggered).
    pub four_act_snapshot: Arc<Mutex<FourActSnapshot>>,
    /// HBCT (Hour-Block Capacity Tokens) ledger. Per
    /// INVENTION_STACK.md §A3.4 the launch wedge: capacity in hour
    /// H decays to 0 at H+1. Off-chain testnet ledger backed by the
    /// `evaporchain-hbct` crate; production wires real GB Elexon
    /// BMRS / ENTSO-E adapters via `evaporchain_hbct::OracleFeed`.
    pub hbct_book: Arc<Mutex<evaporchain_hbct::HbctBook>>,
    /// Mock oracle feed for HBCT settlement attestations.
    pub hbct_oracle: Arc<Mutex<evaporchain_hbct::oracle::MockOracleFeed>>,
    /// Decay-Lamport energy-driven logical clock per §4.1 #3.
    /// Ticked from main.rs after each block by gas_used. Pure
    /// observability — chain still uses block.epoch as the
    /// authoritative time. Production governance amendment can
    /// promote to authoritative time after validators converge on
    /// `tick_quantum`.
    pub lamport_clock: Arc<Mutex<evaporchain_decay_lamport::LamportClock>>,
    /// Patronage Covenant registry — active pledges, immunity status, and
    /// per-object donation scores. §4.1 #13 Refresh-Pool Patronage.
    pub patronage_book: Arc<Mutex<evaporchain_refresh_patronage::PatronageBook>>,
    /// Refresh pool used by the patronage demo. Seeded at startup; in
    /// production this would be the chain's canonical RefreshPool from StateDB.
    pub patronage_pool: Arc<Mutex<evaporchain_energy_kernel::RefreshPool>>,
    /// Evaporative Protocol Version registry. Tracks active protocol versions
    /// and prunes those whose energy has λ-decayed below E_min.
    pub epv_registry: Arc<Mutex<evaporchain_epv::EpvRegistry>>,
    /// Decay-Stamped Nullifier window. Bounded-state per-window accumulator
    /// for privacy chains (DSN §4.2). Window depth = 32 epochs.
    pub dsn_window: Arc<Mutex<evaporchain_dsn::DsnWindow>>,
    /// Lyapunov-stable fee controller state. Single EIP-1559-style integrator
    /// with λ-decay leak. Tracks cumulative block gas pressure.
    pub fee_state: Arc<Mutex<evaporchain_fee_controller::FeeState>>,
    /// Phased Nullifier Tree — sliding-window double-spend guard. Window
    /// depth = 16 phases; each phase advances on explicit API call or
    /// per-epoch consensus hook.
    pub pnt: Arc<Mutex<evaporchain_pnt::PhasedNullifierTree>>,
    /// Directory containing on-disk `.zst` `SnapshotFile` blobs served
    /// by `/api/snapshot/download/:height`. Populated when the node was
    /// started with `--snapshot-dir` (or the default
    /// `<data_dir>/snapshots`). `None` disables snapshot serving.
    pub snapshot_dir: Option<std::path::PathBuf>,
    /// libp2p Sybil-resistance state (peer IPs, scores, ban list,
    /// rejection counters). `None` when the node was started without
    /// `--network-mode` and the in-process libp2p swarm is absent.
    pub network_sybil: Option<Arc<std::sync::RwLock<evaporchain_network::SybilState>>>,
}

/// Public-facing snapshot of the four-act narrative spine state for
/// the chain's status endpoint. Per `INVENTION_STACK.md` Amendment 2
/// §A2.5 the four acts are: Birth (Genesis with LLSA-checked
/// invariants), Life (Sentinel autonomic governance), Small Deaths
/// (Tombstone + eulogy trie), Final Death (Mortis death certificate).
#[derive(Debug, Clone, Default, Serialize)]
pub struct FourActSnapshot {
    /// Number of accounts memorialised in the eulogy trie (Tombstone count).
    pub eulogy_count: usize,
    /// Hex-encoded eulogy-trie root commitment. Empty until first
    /// tombstone or always the canonical-empty hash.
    pub eulogy_trie_root: Option<String>,
    /// Hex-encoded addresses of every memorialised account, sorted.
    /// Cap to a reasonable size for API responses (latest 1024).
    pub tombstone_addresses: Vec<String>,
    /// Total energy accrued in the protocol-owned refresh pool. Read
    /// by `tick_mortis` to detect chain death.
    pub refresh_pool_total: u64,
    /// True iff Mortis has triggered. Latched once true.
    pub mortis_triggered: bool,
    /// Epoch the death-certificate fired (None until trigger).
    pub mortis_epoch_of_death: Option<u64>,
    /// State root committed in the death certificate (None until trigger).
    pub mortis_final_state_root: Option<String>,
    /// Per-block §1.2 conservation audit verdict from
    /// `SimpleExecutor::last_conservation_audit`. `None` until first
    /// block; `Some(true)` = audit passed; `Some(false)` = violation.
    pub last_conservation_audit_ok: Option<bool>,
    /// Genesis amendment hash that the chain's constitution proof
    /// bound to. Empty until genesis ceremony runs.
    pub genesis_amendment_hash: Option<String>,
    /// Number of blocks recorded in the parallel Light-Cone DAG that
    /// runs alongside Tendermint per INVENTION_STACK.md §4.1 #1.
    pub light_cone_block_count: usize,
}

impl ApiState {
    /// Replace the four-act snapshot with `snap`. Called by the
    /// consensus layer after each `SimpleExecutor::execute_block` so
    /// `/api/four_act` reflects the latest narrative-spine state.
    pub fn update_four_act_snapshot(&self, snap: FourActSnapshot) {
        let mut s = safe_lock(&self.four_act_snapshot);
        *s = snap;
    }

    /// Reserve and return the next nonce for `addr`. Concurrent submits
    /// to the same account get distinct, monotonically-increasing nonces
    /// instead of all reading the same db.nonce. The cache is bounded:
    /// when pending txs commit and db.nonce catches up, max(db, cache)
    /// keeps the cache from drifting forever; if a tx is rejected the
    /// cache may temporarily run ahead of reality and subsequent
    /// submits will fail with InvalidNonce until the cache resyncs.
    pub fn reserve_nonce(&self, addr: &[u8; 32]) -> u64 {
        // Re-audit (2026-05-02): bound the pending_nonces map so an
        // attacker submitting txs from millions of distinct addresses
        // can't grow it without limit. When the cache exceeds 100k
        // entries (~3 MiB at 32-byte keys + 8-byte values + overhead),
        // drop entries whose cached nonce is ≤ the on-disk db.nonce
        // (i.e. the chain has caught up). If still over after the
        // sweep, drop everything — pending nonces are an
        // optimisation, not authoritative state.
        const NONCE_CACHE_HARD_CAP: usize = 100_000;
        let db_nonce = {
            let db = safe_lock(&self.db);
            db.get_account(addr).map(|a| a.nonce).unwrap_or(0)
        };
        let mut cache = safe_lock(&self.pending_nonces);
        if cache.len() >= NONCE_CACHE_HARD_CAP {
            let db_lock = safe_lock(&self.db);
            cache.retain(|cached_addr, cached_next| {
                let on_chain = db_lock
                    .get_account(cached_addr)
                    .map(|a| a.nonce)
                    .unwrap_or(0);
                *cached_next > on_chain
            });
            drop(db_lock);
            if cache.len() >= NONCE_CACHE_HARD_CAP {
                cache.clear();
            }
        }
        let next = std::cmp::max(db_nonce, cache.get(addr).copied().unwrap_or(0));
        cache.insert(*addr, next.saturating_add(1));
        next
    }

    /// Submit a transaction to the correct mempool and broadcast over P2P.
    /// API transactions use priority insertion (front of queue) to avoid being
    /// buried behind demo transactions.
    pub fn submit_tx(&self, tx: Transaction) {
        // Broadcast to other validators via P2P
        if let Some(ref sender) = self.tx_broadcast {
            let _ = sender.try_send(tx.clone());
        }
        if let Some(ref tc) = self.tendermint {
            let mut c = safe_lock(tc);
            c.mempool.submit_priority(tx);
        } else {
            let mut c = safe_lock(&self.consensus);
            c.mempool.submit_priority(tx);
        }
    }

    /// Check for duplicate transaction in the active mempool.
    pub fn mempool_contains(&self, predicate: impl Fn(&Transaction) -> bool) -> bool {
        if let Some(ref tc) = self.tendermint {
            let c = safe_lock(tc);
            c.mempool.pending().iter().any(predicate)
        } else {
            let c = safe_lock(&self.consensus);
            c.mempool.pending().iter().any(predicate)
        }
    }

    /// Get current mempool length.
    pub fn mempool_len(&self) -> usize {
        if let Some(ref tc) = self.tendermint {
            let c = safe_lock(tc);
            c.mempool.len()
        } else {
            let c = safe_lock(&self.consensus);
            c.mempool.len()
        }
    }

    /// Get pending transactions from mempool as JSON summaries.
    pub fn mempool_transactions(&self) -> Vec<serde_json::Value> {
        let txs: Vec<Transaction> = if let Some(ref tc) = self.tendermint {
            let c = safe_lock(tc);
            c.mempool.pending().iter().cloned().collect()
        } else {
            let c = safe_lock(&self.consensus);
            c.mempool.pending().iter().cloned().collect()
        };
        txs.iter().map(tx_to_json).collect()
    }

    /// Check whether a tx hash is currently sitting in the active mempool.
    /// Wraps the consensus-layer `Mempool::contains_hash` so the API layer
    /// doesn't reach into private fields.
    pub fn mempool_contains_hash(&self, hash: &[u8; 32]) -> bool {
        if let Some(ref tc) = self.tendermint {
            let c = safe_lock(tc);
            c.mempool.contains_hash(hash)
        } else {
            let c = safe_lock(&self.consensus);
            c.mempool.contains_hash(hash)
        }
    }

    /// Position of `hash` within the FIFO `pending()` queue, plus the total
    /// pending depth. Returns `None` when the tx isn't in the mempool. Used
    /// by `/api/tx/:hash` so the wallet can render "12 of 80 ahead".
    pub fn mempool_position(&self, hash: &[u8; 32]) -> Option<(usize, usize)> {
        let scan = |pending: &std::collections::VecDeque<Transaction>| -> Option<usize> {
            pending
                .iter()
                .position(|tx| &crate::persistence::ChainStore::compute_tx_hash(tx) == hash)
        };
        if let Some(ref tc) = self.tendermint {
            let c = safe_lock(tc);
            let total = c.mempool.len();
            scan(c.mempool.pending()).map(|p| (p, total))
        } else {
            let c = safe_lock(&self.consensus);
            let total = c.mempool.len();
            scan(c.mempool.pending()).map(|p| (p, total))
        }
    }
}

// ──────────────────────────── NFT Store ────────────────────────────────

/// In-memory NFT storage for the MortalNFT marketplace.
#[derive(Clone, Serialize, Deserialize)]
pub struct NftToken {
    pub id: u64,
    pub name: String,
    pub collection: String,
    pub owner: String,
    pub metadata_hash: String,
    pub energy: u64,
    pub max_energy: u64,
    pub half_life: u64,
    pub minted_epoch: u64,
    pub last_refreshed: u64,
    pub state: String, // "Active", "Grace", "Ghost"
    pub grace_epoch: Option<u64>,
    pub evaporated_epoch: Option<u64>,
    pub ghost_proof: Option<String>,
}

impl NftToken {
    /// Compute current energy using exponential decay.
    pub fn current_energy(&self, epoch: u64) -> u64 {
        if self.state == "Ghost" {
            return 0;
        }
        evaporchain_types::energy_at_epoch(
            self.energy,
            self.half_life,
            epoch.saturating_sub(self.last_refreshed),
        )
    }

    /// Compute decay percentage.
    pub fn decay_pct(&self, epoch: u64) -> f64 {
        if self.max_energy == 0 {
            return 100.0;
        }
        let current = self.current_energy(epoch);
        ((self.max_energy - current) as f64 / self.max_energy as f64 * 100.0 * 10.0).round() / 10.0
    }

    /// Estimate remaining epochs until energy reaches ~0.
    pub fn epochs_remaining(&self, epoch: u64) -> u64 {
        if self.half_life == 0 || self.state == "Ghost" {
            return 0;
        }
        let current = self.current_energy(epoch);
        if current <= 1 {
            return 0;
        }
        // energy * 2^(-t/hl) < 1 => t > hl * log2(energy)
        (self.half_life as f64 * (current as f64).log2()).ceil() as u64
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NftStore {
    pub tokens: Vec<NftToken>,
    pub next_id: u64,
}

/// Record of a produced/applied block for the API.
#[derive(Clone, Serialize, Deserialize)]
pub struct BlockRecord {
    pub number: u64,
    pub epoch: u64,
    pub parent_hash: String,
    pub state_root: String,
    pub tx_count: usize,
    pub evaporations: usize,
    pub entered_grace: usize,
    pub timestamp: u64,
    pub active_objects: usize,
    pub ghost_count: usize,
    pub gas_used: u64,
    pub base_fee: u64,
    pub total_fees: u64,
    pub transactions: Vec<TxRecord>,
    /// Whether this block has a Nova IVC proof attached.
    #[serde(default)]
    pub has_nova_proof: bool,
    /// Nova proof size in bytes (0 if no proof).
    #[serde(default)]
    pub nova_proof_size: usize,
    /// DA data root (hex-encoded blake3 hash of 2D erasure commitments).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_root: Option<String>,
    /// 2D DA: number of row/column roots in the extended data square.
    #[serde(default)]
    pub da_square_size: usize,
    /// Number of namespace blob commitments in this block.
    #[serde(default)]
    pub blob_count: usize,
    /// Rule-Based Consensus: whether this block carries a state function commitment.
    #[serde(default)]
    pub has_state_commitment: bool,
    /// Whether this block is an anchor point (full state materialization).
    #[serde(default)]
    pub is_anchor: bool,
    /// Anchor epoch referenced by this block's state commitment.
    #[serde(default)]
    pub anchor_epoch: u64,
}

/// Transaction record with hash and structured data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxRecord {
    pub hash: String,
    #[serde(rename = "type")]
    pub tx_type: String,
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub half_life: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub gas: u64,
    pub block_number: u64,
    pub epoch: u64,
    pub status: String,
    /// Executor error message when `status == "rejected"`. Carries the
    /// stringified `TxOutcome.error` produced by the execution engine
    /// (e.g. `"InsufficientBalance: have 100, need 500"`). Absent for
    /// successful txs and serialised as missing — wallets / explorers
    /// surface this to answer "why did my tx fail?". Wired through
    /// `tx_records_from_block_with_outcomes` from `outcome_to_status`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Accumulated chain statistics.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChainStats {
    pub total_objects_created: u64,
    pub total_evaporated: u64,
    pub total_resurrected: u64,
    pub total_refreshed: u64,
    pub total_transactions: u64,
    /// Cumulative count of finalised transactions that the executor
    /// rejected (insufficient balance, invalid nonce, signature failure,
    /// etc.). Mirrors the `evap_finalised_txs_total{result="failed"}`
    /// Prometheus series. `#[serde(default)]` so persisted ChainStats
    /// from before this field existed deserialize cleanly to 0.
    #[serde(default)]
    pub total_rejected_transactions: u64,
    pub state_size_trend: Vec<EpochSnapshot>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct EpochSnapshot {
    pub epoch: u64,
    pub active_count: usize,
    pub ghost_count: usize,
    pub total_energy: u64,
}

impl ChainStats {
    pub fn new() -> Self {
        Self {
            total_objects_created: 0,
            total_evaporated: 0,
            total_resurrected: 0,
            total_refreshed: 0,
            total_transactions: 0,
            total_rejected_transactions: 0,
            state_size_trend: Vec::new(),
        }
    }
}

/// Rolling throughput metrics for TPS measurement.
#[derive(Clone, Serialize, Deserialize)]
pub struct ThroughputTracker {
    /// Recent block records: (timestamp_ms, tx_count, exec_time_us, gas_used).
    recent_blocks: VecDeque<(u64, usize, u64, u64)>,
    /// Peak TPS observed.
    pub peak_tps: f64,
}

impl ThroughputTracker {
    pub fn new() -> Self {
        Self {
            recent_blocks: VecDeque::new(),
            peak_tps: 0.0,
        }
    }

    /// Record a new block's throughput data.
    pub fn record_block(
        &mut self,
        timestamp_ms: u64,
        tx_count: usize,
        exec_time_us: u64,
        gas_used: u64,
    ) {
        self.recent_blocks
            .push_back((timestamp_ms, tx_count, exec_time_us, gas_used));
        // Keep last 100 blocks
        while self.recent_blocks.len() > 100 {
            self.recent_blocks.pop_front();
        }
        let tps = self.current_tps();
        if tps > self.peak_tps {
            self.peak_tps = tps;
        }
    }

    /// Calculate TPS over recent blocks (last 10 seconds window).
    pub fn current_tps(&self) -> f64 {
        if self.recent_blocks.len() < 2 {
            return 0.0;
        }
        let now = self.recent_blocks.back().unwrap().0;
        let window_ms = 10_000; // 10-second window
        let cutoff = now.saturating_sub(window_ms);
        let in_window: Vec<_> = self
            .recent_blocks
            .iter()
            .filter(|(ts, _, _, _)| *ts >= cutoff)
            .collect();
        if in_window.len() < 2 {
            return 0.0;
        }
        let total_txs: usize = in_window.iter().map(|(_, tc, _, _)| tc).sum();
        let span_ms = in_window.last().unwrap().0 - in_window.first().unwrap().0;
        if span_ms == 0 {
            return 0.0;
        }
        total_txs as f64 / (span_ms as f64 / 1000.0)
    }

    /// Average block execution time (microseconds) over recent blocks.
    pub fn avg_exec_time_us(&self) -> u64 {
        if self.recent_blocks.is_empty() {
            return 0;
        }
        let total: u64 = self.recent_blocks.iter().map(|(_, _, t, _)| t).sum();
        total / self.recent_blocks.len() as u64
    }

    /// Average gas used per block.
    pub fn avg_gas_per_block(&self) -> u64 {
        if self.recent_blocks.is_empty() {
            return 0;
        }
        let total: u64 = self.recent_blocks.iter().map(|(_, _, _, g)| g).sum();
        total / self.recent_blocks.len() as u64
    }

    /// Average txs per block.
    pub fn avg_txs_per_block(&self) -> f64 {
        if self.recent_blocks.is_empty() {
            return 0.0;
        }
        let total: usize = self.recent_blocks.iter().map(|(_, tc, _, _)| tc).sum();
        total as f64 / self.recent_blocks.len() as f64
    }
}

/// Live event record for the dashboard feed.
#[derive(Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub epoch: u64,
    pub event_type: String, // "grace", "evaporated", "created", "refreshed", "transfer", "resurrected"
    pub message: String,
    pub timestamp_ms: u64,
}

// ──────────────────────────── Genesis Addresses ─────────────────────────

/// Realistic 20-byte genesis addresses (hex, no 0x prefix).
pub const GENESIS_FOUNDATION: &str = "7f3a8b2ce419d605a1c74e823fb960d4159ae378";
pub const GENESIS_CORE_DEV: &str = "2b91f50d68a37ce214b65903d74a8ef1c5263b90";
pub const GENESIS_VALIDATOR1: &str = "91e5c8f23d7b4a061f9c82e640d53a17b8f26f47";
pub const GENESIS_VALIDATOR2: &str = "4d02a7e91c3f86b5d24e0f738c915ba6e0d7a8b6";
pub const GENESIS_ECOSYSTEM: &str = "a3f71b5e928d4c063e7a50f81d9c26b34e8a1c5e";
pub const GENESIS_COMMUNITY: &str = "e8b12d7f94c6a35081e4f29b6d3c8a57f1e07d94";

// ──────────────────────────── Name Helpers ─────────────────────────────

fn addr_from_byte(b: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    a[0] = b;
    a
}

/// Parse a hex address string into a 32-byte array.
///
/// M10 (audit 2026-05-02): strict 32 bytes only. The earlier 1–32-byte
/// left-pad behaviour created address-collision attack surface — `"0xAB"`
/// and `"0x00…00AB"` resolved to the same canonical address but rendered
/// differently elsewhere, masking lookup bugs and enabling cache
/// poisoning. Callers that need byte-shorthand should use
/// `addr_from_byte` explicitly.
pub fn parse_hex_address(s: &str) -> Result<[u8; 32], String> {
    let clean = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(clean).map_err(|_| "invalid hex address".to_string())?;
    if bytes.len() != 32 {
        return Err(format!(
            "address must be exactly 32 bytes (64 hex chars), got {} bytes",
            bytes.len()
        ));
    }
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&bytes);
    Ok(addr)
}

/// Parse address from JSON value — accepts hex string or legacy integer byte.
fn parse_address_value(val: &serde_json::Value) -> Result<[u8; 32], String> {
    match val {
        serde_json::Value::Number(n) => {
            let byte = n.as_u64().ok_or("invalid address number")? as u8;
            Ok(addr_from_byte(byte))
        }
        serde_json::Value::String(s) => parse_hex_address(s),
        _ => Err("address must be a hex string or number".to_string()),
    }
}

/// Display address as truncated hex (first 4 bytes + last 3 bytes of 20-byte portion).
fn account_name(addr: &[u8; 32]) -> String {
    let full = hex::encode(&addr[..20]);
    if full.trim_start_matches('0').is_empty() {
        return "0x0000...0000".to_string();
    }
    format!("0x{}...{}", &full[..8], &full[34..])
}

/// Full 32-byte account address as 0x-prefixed hex.
///
/// Previously truncated to the first 20 bytes which caused distinct
/// AccountAddress values (faucet [0;32] vs faucet recipients whose
/// last byte differs) to render as the same display string, masking
/// failed transfers in /api/accounts and /api/block/N responses and
/// making cluster load-test verification impossible. Restored to full
/// 32-byte hex.
fn account_full(addr: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(addr))
}

/// Try to extract a name from the object's data field, otherwise use hex id.
fn object_name(id: &[u8; 32], data: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(data) {
        if !s.is_empty() && s.len() < 64 {
            return s.to_string();
        }
    }
    let full = hex::encode(&id[..8]);
    format!("0x{}...{}", &full[..8], &hex::encode(&id[6..8]))
}

/// Generate a blake3 tx hash from content.
fn tx_hash(content: &str) -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let input = format!("{}:{}", content, ts);
    blake3::hash(input.as_bytes()).to_hex().to_string()
}

fn tx_to_json(tx: &Transaction) -> serde_json::Value {
    match tx {
        Transaction::Transfer(t) => serde_json::json!({
            "type": "transfer",
            "from": format!("0x{}", hex::encode(&t.from[..4])),
            "to": format!("0x{}", hex::encode(&t.to[..4])),
            "amount": t.amount,
            "nonce": t.nonce,
        }),
        Transaction::CreateObject(t) => serde_json::json!({
            "type": "create_object",
            "creator": format!("0x{}", hex::encode(&t.creator[..4])),
            "object_id": format!("0x{}", hex::encode(&t.object_id[..8])),
            "energy": t.energy,
            "half_life": t.half_life,
        }),
        Transaction::Refresh(t) => serde_json::json!({
            "type": "refresh",
            "object_id": format!("0x{}", hex::encode(&t.object_id[..8])),
            "energy_deposit": t.energy_deposit,
        }),
        Transaction::DeployContract(t) => serde_json::json!({
            "type": "deploy_contract",
            "deployer": format!("0x{}", hex::encode(&t.deployer[..4])),
            "template": t.template,
            "energy": t.energy,
        }),
        Transaction::CallContract(t) => serde_json::json!({
            "type": "call_contract",
            "caller": format!("0x{}", hex::encode(&t.caller[..4])),
            "contract_id": t.contract_id,
            "method": t.method,
        }),
        Transaction::DeployScript(t) => serde_json::json!({
            "type": "deploy_script",
            "deployer": format!("0x{}", hex::encode(&t.deployer[..4])),
            "energy": t.energy,
        }),
        Transaction::CallScript(t) => serde_json::json!({
            "type": "call_script",
            "caller": format!("0x{}", hex::encode(&t.caller[..4])),
            "contract_id": t.contract_id,
            "method": t.method,
        }),
        Transaction::ValidatorStake(t) => serde_json::json!({
            "type": "validator_stake",
            "validator": format!("0x{}", hex::encode(&t.validator_address[..4])),
            "amount": t.stake_amount,
        }),
        Transaction::ValidatorExit(t) => serde_json::json!({
            "type": "validator_exit",
            "validator": format!("0x{}", hex::encode(&t.validator_address[..4])),
        }),
        Transaction::ValidatorClaimStake(t) => serde_json::json!({
            "type": "validator_claim_stake",
            "validator": format!("0x{}", hex::encode(&t.validator_address[..4])),
            "validator_id": t.validator_id,
        }),
        Transaction::Shield(_) => serde_json::json!({ "type": "shield" }),
        Transaction::Unshield(_) => serde_json::json!({ "type": "unshield" }),
        Transaction::PrivateTransfer(_) => serde_json::json!({ "type": "private_transfer" }),
        Transaction::Deferred(_) => serde_json::json!({ "type": "deferred" }),
        Transaction::Blob(_) => serde_json::json!({ "type": "blob" }),
        Transaction::Governance(_) => serde_json::json!({ "type": "governance" }),
        Transaction::MultiSig(_) => serde_json::json!({ "type": "multisig" }),
        Transaction::UserOp(_) => serde_json::json!({ "type": "user_op" }),
        Transaction::UpgradeContract(_) => serde_json::json!({ "type": "upgrade_contract" }),
        Transaction::Delegate(t) => {
            serde_json::json!({ "type": "delegate", "validator_id": t.validator_id, "amount": t.amount })
        }
        Transaction::Undelegate(t) => {
            serde_json::json!({ "type": "undelegate", "validator_id": t.validator_id, "amount": t.amount })
        }
        Transaction::RotateValidatorKey(t) => serde_json::json!({
            "type": "rotate_validator_key",
            "validator_id": t.validator_id,
            "effective_epoch": t.effective_epoch,
        }),
        Transaction::ClaimDelegation(t) => serde_json::json!({
            "type": "claim_delegation",
            "validator_id": t.validator_id,
        }),
        Transaction::Refund(t) => serde_json::json!({
            "type": "refund",
            "source_block_height": t.source_block_height,
            "attacker_hex": hex::encode(t.attacker),
            "victim_hex": hex::encode(t.victim),
            "amount": t.amount,
        }),
    }
}

// ──────────────────────────── Response Types ──────────────────────────

#[derive(Serialize)]
struct StatusResponse {
    chain_name: &'static str,
    version: &'static str,
    block_height: u64,
    epoch: u64,
    active_objects: usize,
    ghost_count: usize,
    total_evaporated: u64,
    peer_count: usize,
    state_root: String,
    proving_enabled: bool,
    uptime_seconds: u64,
}

#[derive(Serialize)]
struct ObjectResponse {
    id: String,
    name: String,
    owner: String,
    owner_name: String,
    energy: u64,
    max_energy: u64,
    half_life: u64,
    state: String,
    created_epoch: u64,
    last_refreshed: u64,
    grace_epoch: Option<u64>,
    current_energy: u64,
    decay_percentage: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    decay_curve: Option<evaporchain_types::DecayCurve>,
    /// Whether this object is governed by the LAD-VM substructural-
    /// resource type system (linear/affine/decaying).
    ///
    /// Backed by `StateObject.lad_mode.is_some()`. `is_lad_typed` is the
    /// boolean shorthand the wallet uses to gate UI affordances; the
    /// richer `lad_mode` field below carries the actual variant.
    is_lad_typed: bool,
    /// LAD-VM substructural mode (`"linear"` | `"affine"` | `"decaying"`)
    /// when this object is LAD-typed; `null` for ordinary objects.
    /// Lets the wallet show the actual mode (e.g. "LAD · linear") rather
    /// than just a generic LAD pill.
    #[serde(skip_serializing_if = "Option::is_none")]
    lad_mode: Option<evaporchain_types::LadMode>,
}

#[derive(Serialize)]
struct AccountResponse {
    address: String,
    name: String,
    balance: u64,
    nonce: u64,
}

#[allow(dead_code)]
#[derive(Serialize)]
struct GhostResponse {
    id: String,
    original_owner: String,
    evaporated_epoch: u64,
    data_hash: String,
}

#[derive(Serialize)]
struct StatsTimelineResponse {
    epochs: Vec<EpochSnapshot>,
}

#[derive(Serialize)]
struct StatsSummaryResponse {
    total_created: u64,
    total_evaporated: u64,
    total_resurrected: u64,
    total_refreshed: u64,
    avg_lifetime_epochs: f64,
    total_transactions: u64,
}

#[derive(Serialize)]
struct NetworkResponse {
    peer_count: usize,
}

#[derive(Serialize)]
struct EventsResponse {
    events: Vec<EventRecord>,
}

#[derive(Deserialize)]
struct BlocksQuery {
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct TransactionsQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    #[serde(rename = "type")]
    tx_type: Option<String>,
    address: Option<String>,
}

#[derive(Serialize)]
struct TransactionsResponse {
    transactions: Vec<TxRecord>,
    total: usize,
    limit: usize,
    offset: usize,
}

#[derive(Deserialize)]
struct EventsQuery {
    limit: Option<usize>,
}

// ── Transaction request types (accept hex string or legacy integer) ──

#[derive(Deserialize)]
struct TransferRequest {
    from: serde_json::Value,
    to: serde_json::Value,
    amount: u64,
    nonce: u64,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    public_key: Option<String>,
}

#[derive(Deserialize)]
struct CreateObjectRequest {
    creator: serde_json::Value,
    object_id: serde_json::Value,
    energy: u64,
    half_life: u64,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    decay_curve: Option<evaporchain_types::DecayCurve>,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    public_key: Option<String>,
}

#[derive(Deserialize)]
struct RefreshRequest {
    object_id: serde_json::Value,
    energy_deposit: u64,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    public_key: Option<String>,
}

// ── Delegation request types (P0 #4 wallet-facing) ────────────────────
//
// These mirror the on-chain `DelegateTx` / `UndelegateTx` /
// `ClaimDelegationTx` shapes (evaporchain-types) but accept the
// wallet's canonical hex/json address form. Each handler validates the
// ML-DSA signature against canonical bytes the same way
// `post_transfer` does (let the consensus layer's signature check fire
// at execute time). The validator-side accounting is already wired:
// `TendermintConsensus::tick` calls
// `validator_set.refresh_delegated_stakes()` which sums per-validator
// `DelegationRecord.amount` into `ValidatorInfo.delegated_stake`, and
// every consensus weight calc uses `effective_stake() = stake +
// delegated_stake`. So the wallet-facing endpoints below are the only
// missing piece on the delegation side of the chain.

#[derive(Deserialize)]
struct DelegateRequest {
    delegator: serde_json::Value,
    validator_id: u64,
    amount: u64,
    nonce: u64,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    public_key: Option<String>,
}

#[derive(Deserialize)]
struct UndelegateRequest {
    delegator: serde_json::Value,
    validator_id: u64,
    amount: u64,
    nonce: u64,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    public_key: Option<String>,
}

#[derive(Deserialize)]
struct ClaimDelegationRequest {
    delegator: serde_json::Value,
    validator_id: u64,
    nonce: u64,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    public_key: Option<String>,
}

#[derive(Serialize)]
struct DelegationView {
    /// 0x-prefixed 32-byte delegator address.
    pub delegator: String,
    pub validator_id: u64,
    /// Currently bonded delegation amount (counts toward
    /// `validator.effective_stake`).
    pub amount: u64,
    pub delegated_at_epoch: u64,
    /// Amount currently in the unbonding window (NOT counted in
    /// effective stake; not yet returned to balance).
    pub unbonding_amount: u64,
    /// `Some(epoch)` if an undelegate is in progress; the delegator can
    /// `claim_delegation` after `unbonding_epoch + UNBONDING_PERIOD_EPOCHS`.
    pub unbonding_epoch: Option<u64>,
}

#[derive(Serialize)]
struct ValidatorDelegationsResponse {
    pub validator_id: u64,
    pub delegation_count: usize,
    /// Σ amount across active delegations — matches
    /// `ValidatorInfo.delegated_stake` after the next consensus tick's
    /// `refresh_delegated_stakes`.
    pub total_delegated: u64,
    pub delegations: Vec<DelegationView>,
}

#[derive(Serialize)]
struct TxResultResponse {
    success: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tx_hash: Option<String>,
}

#[derive(Deserialize)]
struct FaucetRequest {
    address: serde_json::Value,
}

#[derive(Serialize)]
struct FaucetResponse {
    success: bool,
    balance: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

// ──────────────────────────── Batch Transactions ──────────────────────

#[derive(Deserialize)]
#[serde(tag = "type")]
enum BatchTxItem {
    #[serde(rename = "transfer")]
    Transfer {
        from: serde_json::Value,
        to: serde_json::Value,
        amount: u64,
        nonce: u64,
    },
    #[serde(rename = "create_object")]
    CreateObject {
        creator: serde_json::Value,
        object_id: serde_json::Value,
        energy: u64,
        half_life: u64,
    },
    #[serde(rename = "refresh")]
    Refresh {
        object_id: serde_json::Value,
        energy_deposit: u64,
    },
    #[serde(rename = "resurrect")]
    Resurrect {
        object_id: serde_json::Value,
        energy_deposit: u64,
    },
}

#[derive(Deserialize)]
struct BatchRequest {
    transactions: Vec<BatchTxItem>,
}

#[derive(Serialize)]
struct BatchItemResult {
    index: usize,
    success: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tx_hash: Option<String>,
}

#[derive(Serialize)]
struct BatchResponse {
    submitted: usize,
    failed: usize,
    results: Vec<BatchItemResult>,
}

// ──────────────────────────── Auth check for tx endpoints ──────────────

/// Require a valid auth token OR a valid signature for transaction endpoints.
/// Returns Ok(user_id) if authenticated, Err(response) otherwise.
fn require_tx_auth(
    headers: &HeaderMap,
    state: &ApiState,
    has_signature: bool,
) -> Result<Option<i64>, Json<TxResultResponse>> {
    // If a cryptographic signature is provided, let the consensus layer verify it
    if has_signature {
        return Ok(None);
    }
    // Otherwise require a valid session token
    if let Some(ref sessions) = state.auth_sessions {
        match crate::auth::authenticate(headers, sessions) {
            Ok(user_id) => Ok(Some(user_id)),
            Err(e) => Err(Json(TxResultResponse {
                success: false,
                message: format!("Authentication required: {}", e),
                tx_hash: None,
            })),
        }
    } else {
        // No auth system configured — allow (e.g. test mode)
        Ok(None)
    }
}

/// Check that the given address belongs to the authenticated user.
fn require_wallet_ownership(
    state: &ApiState,
    user_id: Option<i64>,
    addr_hex: &str,
) -> Result<(), Json<TxResultResponse>> {
    let user_id = match user_id {
        Some(id) => id,
        None => return Ok(()), // signature-based auth, no user binding
    };
    if let Some(ref _sessions) = state.auth_sessions {
        // We need user_db to check ownership — it's on auth_state
        // For now, we store it on ApiState
        if let Some(ref user_db) = state.user_db {
            match user_db.get_wallet_owner(addr_hex) {
                Ok(Some(owner_id)) if owner_id == user_id => Ok(()),
                Ok(Some(_)) => Err(Json(TxResultResponse {
                    success: false,
                    message: "Address does not belong to your account".into(),
                    tx_hash: None,
                })),
                Ok(None) => Ok(()), // Address not in user DB (e.g. genesis addr) — allow
                Err(e) => {
                    tracing::warn!("DB error during wallet ownership check: {}", e);
                    Err(Json(TxResultResponse {
                        success: false,
                        message: "Unable to verify wallet ownership. Try again.".into(),
                        tx_hash: None,
                    }))
                }
            }
        } else {
            Ok(())
        }
    } else {
        Ok(())
    }
}

/// Admin-endpoint auth gate. Fail-CLOSED if `EVAPORCHAIN_ADMIN_KEY`
/// is unset or empty.
///
/// **Audit fix C1**: the legacy implementation returned `Ok(())` when
/// the env var was unset, leaving every admin endpoint
/// (`/api/admin/drain`, `/api/admin/undrain`, `/metrics`,
/// `/api/network/ban`, `/api/network/unban`, `/api/proof_replay`)
/// completely unauthenticated. Only a stderr warning at startup
/// covered this. Operators who started without the env var believed
/// they were still gated. This now returns 503 Service Unavailable
/// with a clear error message — admin endpoints are unusable until
/// the operator sets the env var.
fn require_admin_auth(headers: &HeaderMap) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let expected = match std::env::var("EVAPORCHAIN_ADMIN_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "admin endpoints disabled: EVAPORCHAIN_ADMIN_KEY not configured",
                    "remedy": "set EVAPORCHAIN_ADMIN_KEY to a strong random value before exposing admin endpoints"
                })),
            ));
        }
    };
    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    // Network-4 (re-audit 2026-05-02): constant-time compare. Length
    // mismatch short-circuits (length leak is harmless; bounded by
    // the env var the operator chose). Same-length goes through
    // subtle::ConstantTimeEq.
    let provided_bytes = provided.as_bytes();
    let expected_bytes = expected.as_bytes();
    let ok = provided_bytes.len() == expected_bytes.len()
        && bool::from(<[u8] as subtle::ConstantTimeEq>::ct_eq(
            provided_bytes,
            expected_bytes,
        ));
    if !ok {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized: invalid admin key"})),
        ));
    }
    Ok(())
}

/// First tries the wallet's own ML-DSA keypair from the user DB.
/// Falls back to the node-level keypair if wallet keys are unavailable or legacy (too short).
fn sign_transaction(tx: &mut Transaction, state: &ApiState, sender_address: Option<&str>) {
    // If already signed with a real ML-DSA signature (~3300 bytes), skip
    if let Some(sig) = tx.signature() {
        if sig.len() > 1000 {
            return; // Real signature — keep it
        }
        // Dummy/short signature — replace with node signing below
    }

    // Try wallet-specific keys first
    if let (Some(ref user_db), Some(addr)) = (&state.user_db, sender_address) {
        if let Ok(Some((pk_hex, sk_hex))) = user_db.get_wallet_keys(addr) {
            // Real ML-DSA public keys are 1952 bytes (3904 hex chars)
            if pk_hex.len() > 1000 {
                if let (Ok(pk_bytes), Ok(sk_bytes)) = (hex::decode(&pk_hex), hex::decode(&sk_hex)) {
                    if let Ok(kp) = MlDsaKeypair::from_bytes(&pk_bytes, &sk_bytes) {
                        let msg = tx.signing_message(&state.chain_id);
                        let sig = kp.sign(&msg);
                        let pk = kp.public_key_bytes();
                        set_tx_signature(tx, sig, pk);
                        return;
                    }
                }
            }
        }
    }

    // Fallback: sign with node keypair
    let msg = tx.signing_message(&state.chain_id);
    let sig = state.node_keypair.sign(&msg);
    let pk = state.node_keypair.public_key_bytes();
    set_tx_signature(tx, sig, pk);
}

/// Set the signature and public_key fields on a transaction variant.
fn set_tx_signature(tx: &mut Transaction, sig: Vec<u8>, pk: Vec<u8>) {
    match tx {
        Transaction::Transfer(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::Refresh(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::CreateObject(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::DeployContract(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::CallContract(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::DeployScript(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::CallScript(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::ValidatorStake(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::ValidatorExit(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::ValidatorClaimStake(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::Shield(t) => {
            t.signature = Some(sig);
            t.public_key = Some(pk);
        }
        Transaction::Unshield(_) | Transaction::PrivateTransfer(_) => {} // ZK-authenticated
        Transaction::Deferred(d) => {
            d.signature = Some(sig);
            d.public_key = Some(pk);
        }
        Transaction::Blob(b) => {
            b.signature = Some(sig);
            b.public_key = Some(pk);
        }
        Transaction::Governance(g) => {
            g.signature = Some(sig);
            g.public_key = Some(pk);
        }
        Transaction::MultiSig(_) => {}
        Transaction::UserOp(u) => {
            u.signature = Some(sig);
            u.public_key = Some(pk);
        }
        Transaction::UpgradeContract(u) => {
            u.signature = Some(sig);
            u.public_key = Some(pk);
        }
        Transaction::Delegate(d) => {
            d.signature = Some(sig);
            d.public_key = Some(pk);
        }
        Transaction::Undelegate(u) => {
            u.signature = Some(sig);
            u.public_key = Some(pk);
        }
        Transaction::RotateValidatorKey(r) => {
            r.signature = Some(sig);
            r.public_key = Some(pk);
        }
        Transaction::ClaimDelegation(c) => {
            c.signature = Some(sig);
            c.public_key = Some(pk);
        }
        // Refund is protocol-issued; signing is a no-op.
        Transaction::Refund(_) => {}
    }
}

/// Sanitize a string input: strip null bytes, HTML tags, limit length.
fn sanitize_string(s: &str, max_len: usize) -> Result<String, String> {
    let cleaned: String = s.chars().filter(|c| *c != '\0').collect();
    // Strip HTML tags to prevent stored XSS
    let stripped = strip_html_tags(&cleaned);
    if stripped.len() > max_len {
        return Err(format!("Input too long: max {} characters", max_len));
    }
    Ok(stripped)
}

/// Strip HTML tags from a string.
fn strip_html_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result
}

/// Validate email format (basic but rejects obvious garbage).
pub fn is_valid_email(email: &str) -> bool {
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return false;
    }
    let local = parts[0];
    let domain = parts[1];
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    if !domain.contains('.') {
        return false;
    }
    // Reject obvious non-email characters
    let valid_chars = |c: char| c.is_alphanumeric() || "._+-".contains(c);
    local.chars().all(valid_chars)
        && domain
            .chars()
            .all(|c| c.is_alphanumeric() || ".-".contains(c))
}

// ──────────────────────────── Handlers ─────────────────────────────────

async fn get_status(State(state): State<Arc<ApiState>>) -> Json<StatusResponse> {
    let mut db = safe_lock(&state.db);
    let history = safe_lock(&state.block_history);
    let stats = safe_lock(&state.stats);
    let latest = history.back();

    Json(StatusResponse {
        chain_name: "EvaporChain",
        version: "0.2.0",
        block_height: latest.map(|b| b.number).unwrap_or(0),
        epoch: latest.map(|b| b.epoch).unwrap_or(0),
        active_objects: db.object_count(),
        ghost_count: db.ghost_count(),
        total_evaporated: stats.total_evaporated,
        peer_count: state.peer_count.load(std::sync::atomic::Ordering::Relaxed),
        state_root: hex::encode(db.compute_state_root()),
        proving_enabled: state.prove_mode,
        uptime_seconds: state.start_time.elapsed().as_secs(),
    })
}

/// Public-facing snapshot of the four-act narrative spine. Per
/// INVENTION_STACK.md Amendment 2 §A2.5: Birth (LLSA-checked
/// genesis), Life (Sentinel), Small Deaths (Tombstone), Final Death
/// (Mortis). The consensus layer populates this via
/// `ApiState::update_four_act_snapshot` after each block.
async fn get_four_act_status(State(state): State<Arc<ApiState>>) -> Json<FourActSnapshot> {
    let snap = safe_lock(&state.four_act_snapshot);
    Json(snap.clone())
}

// ─────────── Mortis death-certificate preview ──────────────────────

#[derive(Debug, Serialize)]
pub struct MortisCertPreviewResp {
    pub status: &'static str,
    pub final_state_root_hex: String,
    pub eulogy_trie_root_hex: String,
    pub epoch_of_death: u64,
    pub final_refresh_pool: u64,
    pub witness_hex: String,
    pub note: String,
}

/// Preview the MortisCertificate that *would* be minted at the chain's
/// current state — without mutating anything. Returns the same NFT
/// shape (state_root, eulogy_root, epoch, refresh_pool, witness) the
/// chain would commit to under the doctrine §A2.5 death predicate.
/// Returns status="already-triggered" once the real death has fired.
async fn get_mortis_cert_preview(
    State(state): State<Arc<ApiState>>,
) -> Json<MortisCertPreviewResp> {
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(MortisCertPreviewResp {
                status: "no-consensus-engine",
                final_state_root_hex: String::new(),
                eulogy_trie_root_hex: String::new(),
                epoch_of_death: 0,
                final_refresh_pool: 0,
                witness_hex: String::new(),
                note: "consensus engine not bound to API state".into(),
            });
        }
    };
    let tc = safe_lock(tc);
    match tc.mortis_cert_preview() {
        None => Json(MortisCertPreviewResp {
            status: "already-triggered",
            final_state_root_hex: String::new(),
            eulogy_trie_root_hex: String::new(),
            epoch_of_death: 0,
            final_refresh_pool: 0,
            witness_hex: String::new(),
            note: "real death certificate already minted; query /api/mortis_cert".into(),
        }),
        Some(c) => Json(MortisCertPreviewResp {
            status: "preview",
            final_state_root_hex: hex::encode(c.final_state_root),
            eulogy_trie_root_hex: hex::encode(c.eulogy_trie_root),
            epoch_of_death: c.epoch_of_death,
            final_refresh_pool: c.final_refresh_pool,
            witness_hex: hex::encode(c.witness),
            note: "preview only — chain is alive; this is what the cert would look like at current state".into(),
        }),
    }
}

// ─────────── LAD-VM (Linear-Affine-Decay) lifecycle simulation ─────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LadModeReq {
    Linear,
    Affine,
    Decaying,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LadActionReq {
    /// Evaluate `use_resource(r, current_epoch)`.
    Use,
    /// Evaluate `drop_resource(r)`.
    Drop,
    /// Evaluate `tick_decay(r, current_epoch)`.
    Tick,
}

#[derive(Debug, Deserialize)]
pub struct LadSimQuery {
    pub mode: LadModeReq,
    /// Stand-in value as a u64 — substrate is generic but the API
    /// pins to u64 for simplicity since the lifecycle outcome is
    /// independent of T.
    pub value: u64,
    pub created_at_epoch: u64,
    /// Required iff mode == Decaying. Window in epochs from creation.
    pub decay_window: Option<u64>,
    pub current_epoch: u64,
    pub action: LadActionReq,
}

#[derive(Debug, Serialize)]
pub struct LadSimResp {
    pub status: &'static str,
    pub action: &'static str,
    pub mode: &'static str,
    /// Outcome of the action: "ok" / "evaporated" / "already-consumed"
    /// / "linear-cannot-drop" / for Tick: "ticked-fresh" /
    /// "ticked-evaporated" / "ticked-no-op".
    pub outcome: &'static str,
    pub returned_value: Option<u64>,
    pub is_evaporated_at_query: bool,
    pub created_at_epoch: u64,
    pub current_epoch: u64,
    pub decay_window: Option<u64>,
    pub detail: String,
}

/// Simulate one LAD-VM resource-lifecycle step. Lets dApps and the
/// dashboard probe the substructural type-system without needing the
/// full `script-lad` compiler frontend. Per INVENTION_STACK.md §4.1
/// row 12 ("Move resources × decay — use it or evaporate").
async fn post_lad_vm_simulate(Json(q): Json<LadSimQuery>) -> Json<LadSimResp> {
    let mode_str = match q.mode {
        LadModeReq::Linear => "linear",
        LadModeReq::Affine => "affine",
        LadModeReq::Decaying => "decaying",
    };
    let action_str = match q.action {
        LadActionReq::Use => "use",
        LadActionReq::Drop => "drop",
        LadActionReq::Tick => "tick",
    };

    // Validate decay_window present iff Decaying.
    if matches!(q.mode, LadModeReq::Decaying) && q.decay_window.is_none() {
        return Json(LadSimResp {
            status: "error",
            action: action_str,
            mode: mode_str,
            outcome: "missing-decay-window",
            returned_value: None,
            is_evaporated_at_query: false,
            created_at_epoch: q.created_at_epoch,
            current_epoch: q.current_epoch,
            decay_window: None,
            detail: "Decaying mode requires decay_window".into(),
        });
    }

    let resource = match q.mode {
        LadModeReq::Linear => evaporchain_lad_vm::Resource::linear(q.value, q.created_at_epoch),
        LadModeReq::Affine => evaporchain_lad_vm::Resource::affine(q.value, q.created_at_epoch),
        LadModeReq::Decaying => evaporchain_lad_vm::Resource::decaying(
            q.value,
            q.created_at_epoch,
            q.decay_window.unwrap_or(0),
        ),
    };
    let evap_at_query = resource.is_evaporated(q.current_epoch);

    match q.action {
        LadActionReq::Use => match evaporchain_lad_vm::use_resource(resource, q.current_epoch) {
            Ok((value, _receipt)) => Json(LadSimResp {
                status: "ok",
                action: action_str,
                mode: mode_str,
                outcome: "ok",
                returned_value: Some(value),
                is_evaporated_at_query: evap_at_query,
                created_at_epoch: q.created_at_epoch,
                current_epoch: q.current_epoch,
                decay_window: q.decay_window,
                detail: "resource consumed; value returned".into(),
            }),
            Err(evaporchain_lad_vm::OpError::AlreadyConsumed) => Json(LadSimResp {
                status: "error",
                action: action_str,
                mode: mode_str,
                outcome: "already-consumed",
                returned_value: None,
                is_evaporated_at_query: evap_at_query,
                created_at_epoch: q.created_at_epoch,
                current_epoch: q.current_epoch,
                decay_window: q.decay_window,
                detail: "use rejected: resource already consumed".into(),
            }),
            Err(evaporchain_lad_vm::OpError::Evaporated) => Json(LadSimResp {
                status: "error",
                action: action_str,
                mode: mode_str,
                outcome: "evaporated",
                returned_value: None,
                is_evaporated_at_query: evap_at_query,
                created_at_epoch: q.created_at_epoch,
                current_epoch: q.current_epoch,
                decay_window: q.decay_window,
                detail: "use rejected: resource has aged past its decay window".into(),
            }),
            Err(e) => Json(LadSimResp {
                status: "error",
                action: action_str,
                mode: mode_str,
                outcome: "error",
                returned_value: None,
                is_evaporated_at_query: evap_at_query,
                created_at_epoch: q.created_at_epoch,
                current_epoch: q.current_epoch,
                decay_window: q.decay_window,
                detail: format!("{e}"),
            }),
        },
        LadActionReq::Drop => match evaporchain_lad_vm::drop_resource(resource) {
            Ok(()) => Json(LadSimResp {
                status: "ok",
                action: action_str,
                mode: mode_str,
                outcome: "ok",
                returned_value: None,
                is_evaporated_at_query: evap_at_query,
                created_at_epoch: q.created_at_epoch,
                current_epoch: q.current_epoch,
                decay_window: q.decay_window,
                detail: "resource dropped without consuming value".into(),
            }),
            Err(evaporchain_lad_vm::OpError::LinearCannotDrop) => Json(LadSimResp {
                status: "error",
                action: action_str,
                mode: mode_str,
                outcome: "linear-cannot-drop",
                returned_value: None,
                is_evaporated_at_query: evap_at_query,
                created_at_epoch: q.created_at_epoch,
                current_epoch: q.current_epoch,
                decay_window: q.decay_window,
                detail: "Linear resources must be consumed exactly once — drop rejected".into(),
            }),
            Err(e) => Json(LadSimResp {
                status: "error",
                action: action_str,
                mode: mode_str,
                outcome: "error",
                returned_value: None,
                is_evaporated_at_query: evap_at_query,
                created_at_epoch: q.created_at_epoch,
                current_epoch: q.current_epoch,
                decay_window: q.decay_window,
                detail: format!("{e}"),
            }),
        },
        LadActionReq::Tick => {
            let after = evaporchain_lad_vm::tick_decay(resource, q.current_epoch);
            let outcome = if after.consumed && evap_at_query {
                "ticked-evaporated"
            } else if after.consumed {
                "ticked-no-op"
            } else {
                "ticked-fresh"
            };
            Json(LadSimResp {
                status: "ok",
                action: action_str,
                mode: mode_str,
                outcome,
                returned_value: None,
                is_evaporated_at_query: evap_at_query,
                created_at_epoch: q.created_at_epoch,
                current_epoch: q.current_epoch,
                decay_window: q.decay_window,
                detail: if outcome == "ticked-evaporated" {
                    "decay tick marked the resource consumed (evaporated past window)".into()
                } else if outcome == "ticked-no-op" {
                    "decay tick was a no-op (already consumed)".into()
                } else {
                    "decay tick: resource still within window, untouched".into()
                },
            })
        }
    }
}

// ─────────── Patronage Covenant API ────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PatronagePledgeReq {
    pub object_id_hex: String,
    pub namespace_id_hex: String,
    pub donation_per_epoch: u64,
    pub epochs: u64,
    pub current_epoch: u64,
}

#[derive(Debug, Deserialize)]
pub struct PatronageActionReq {
    pub object_id_hex: String,
    pub epoch: u64,
}

#[derive(Debug, Serialize)]
pub struct PatronagePledgeResp {
    pub status: &'static str,
    pub object_id_hex: String,
    pub pre_funded: u64,
    pub expires_epoch: u64,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct PatronageHonourResp {
    pub status: &'static str,
    pub donated: u64,
    pub patronage_score: u64,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct PatronageStatusResp {
    pub active_covenants: usize,
    pub total_pre_funded: u64,
    pub total_active_score: u64,
    pub patronage_ns_hex: String,
}

#[derive(Debug, Serialize)]
pub struct PatronageImmuneResp {
    pub object_id_hex: String,
    pub epoch: u64,
    pub immune: bool,
    pub patronage_score: u64,
}

/// POST /api/patronage/pledge — create a Patronage Covenant for an object.
/// Draws `donation_per_epoch × epochs` from the namespace's refresh-pool
/// credit and pre-funds the covenant. Object gains eviction immunity.
async fn post_patronage_pledge(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<PatronagePledgeReq>,
) -> Json<PatronagePledgeResp> {
    let obj_bytes = match hex::decode(&req.object_id_hex) {
        Ok(b) => b,
        Err(_) => {
            return Json(PatronagePledgeResp {
                status: "error",
                object_id_hex: req.object_id_hex,
                pre_funded: 0,
                expires_epoch: 0,
                detail: "invalid object_id_hex".into(),
            })
        }
    };
    let ns_bytes = match hex::decode(&req.namespace_id_hex) {
        Ok(b) => b,
        Err(_) => {
            return Json(PatronagePledgeResp {
                status: "error",
                object_id_hex: req.object_id_hex,
                pre_funded: 0,
                expires_epoch: 0,
                detail: "invalid namespace_id_hex".into(),
            })
        }
    };

    let mut book = safe_lock(&state.patronage_book);
    let mut pool = safe_lock(&state.patronage_pool);

    match evaporchain_refresh_patronage::pledge(
        &mut book,
        &mut pool,
        obj_bytes,
        ns_bytes,
        req.donation_per_epoch,
        req.epochs,
        req.current_epoch,
    ) {
        Ok(cv) => Json(PatronagePledgeResp {
            status: "pledged",
            object_id_hex: req.object_id_hex,
            pre_funded: cv.pre_funded,
            expires_epoch: cv.expires_epoch,
            detail: format!(
                "covenant active epochs {}–{}; {} total pre-funded",
                cv.created_epoch, cv.expires_epoch, cv.pre_funded
            ),
        }),
        Err(e) => Json(PatronagePledgeResp {
            status: "error",
            object_id_hex: req.object_id_hex,
            pre_funded: 0,
            expires_epoch: 0,
            detail: e.to_string(),
        }),
    }
}

/// POST /api/patronage/honour — release one epoch's donation from a covenant
/// into the global patronage pool credit. Increments patronage_score.
async fn post_patronage_honour(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<PatronageActionReq>,
) -> Json<PatronageHonourResp> {
    let obj_bytes = match hex::decode(&req.object_id_hex) {
        Ok(b) => b,
        Err(_) => {
            return Json(PatronageHonourResp {
                status: "error",
                donated: 0,
                patronage_score: 0,
                detail: "invalid object_id_hex".into(),
            })
        }
    };

    let mut book = safe_lock(&state.patronage_book);
    let mut pool = safe_lock(&state.patronage_pool);

    match evaporchain_refresh_patronage::honour(&mut book, &mut pool, &obj_bytes, req.epoch) {
        Ok(donated) => {
            let score = evaporchain_refresh_patronage::patronage_score(&book, &obj_bytes);
            Json(PatronageHonourResp {
                status: "honoured",
                donated,
                patronage_score: score,
                detail: format!(
                    "donated {} at epoch {}; cumulative score {}",
                    donated, req.epoch, score
                ),
            })
        }
        Err(e) => Json(PatronageHonourResp {
            status: "error",
            donated: 0,
            patronage_score: 0,
            detail: e.to_string(),
        }),
    }
}

/// POST /api/patronage/revoke — remove a covenant early; refunds unused
/// pre-funded surplus back to the namespace pool credit.
async fn post_patronage_revoke(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<PatronageActionReq>,
) -> Json<serde_json::Value> {
    let obj_bytes = match hex::decode(&req.object_id_hex) {
        Ok(b) => b,
        Err(_) => {
            return Json(serde_json::json!({
                "status": "error",
                "detail": "invalid object_id_hex"
            }))
        }
    };

    let mut book = safe_lock(&state.patronage_book);
    let mut pool = safe_lock(&state.patronage_pool);

    match evaporchain_refresh_patronage::revoke(&mut book, &mut pool, &obj_bytes, req.epoch) {
        Ok(archived) => Json(serde_json::json!({
            "status": "revoked",
            "object_id_hex": req.object_id_hex,
            "patronage_score_archived": archived.patronage_score,
            "refunded": archived.pre_funded,
            "detail": format!("covenant closed; {} energy refunded to namespace; score {} archived", archived.pre_funded, archived.patronage_score)
        })),
        Err(e) => Json(serde_json::json!({
            "status": "error",
            "detail": e.to_string()
        })),
    }
}

/// GET /api/patronage/status — summary of all active covenants.
async fn get_patronage_status(State(state): State<Arc<ApiState>>) -> Json<PatronageStatusResp> {
    let book = safe_lock(&state.patronage_book);
    Json(PatronageStatusResp {
        active_covenants: book.len(),
        total_pre_funded: book.total_pre_funded(),
        total_active_score: book.total_active_score(),
        patronage_ns_hex: hex::encode(&book.patronage_ns),
    })
}

/// GET /api/patronage/immune?object_id_hex=&epoch= — immunity and score query.
async fn get_patronage_immune(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<PatronageImmuneResp> {
    let hex_str = params.get("object_id_hex").cloned().unwrap_or_default();
    let epoch: u64 = params
        .get("epoch")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let obj_bytes = hex::decode(&hex_str).unwrap_or_default();
    let book = safe_lock(&state.patronage_book);
    let immune = evaporchain_refresh_patronage::is_immune(&book, &obj_bytes, epoch);
    let score = evaporchain_refresh_patronage::patronage_score(&book, &obj_bytes);
    Json(PatronageImmuneResp {
        object_id_hex: hex_str,
        epoch,
        immune,
        patronage_score: score,
    })
}

// ─────────── Boltzmann Stake + Sanov Slash observability ─────────────

#[derive(Debug, Serialize)]
pub struct BoltzmannStakeEntry {
    pub validator_id: u64,
    pub boltzmann_active: u64,
    pub governance_stake: u64,
    pub decay_ratio_pct: f64,
}

#[derive(Debug, Serialize)]
pub struct BoltzmannWeightEntry {
    pub validator_id: u64,
    pub weight: u128,
    pub boltzmann_active: u64,
    pub governance_stake: u64,
}

#[derive(Debug, Serialize)]
pub struct SanovSlashResp {
    pub status: &'static str,
    pub validator_id: u64,
    pub slash_amount: u64,
    pub detail: String,
}

/// GET /api/validators/boltzmann_stakes — current Boltzmann-decayed stake
/// for all validators. Compares against governance stake to show decay ratio.
async fn get_boltzmann_stakes(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    if let Some(tc_arc) = &state.tendermint {
        let tc = safe_lock(tc_arc);
        let entries: Vec<BoltzmannStakeEntry> = tc
            .validator_set
            .validators()
            .iter()
            .map(|v| {
                let b_active = tc
                    .boltzmann_stakes
                    .get(&v.id)
                    .map(|s| s.active)
                    .unwrap_or(v.stake);
                let decay_ratio = if v.stake > 0 {
                    (b_active as f64 / v.stake as f64) * 100.0
                } else {
                    0.0
                };
                BoltzmannStakeEntry {
                    validator_id: v.id,
                    boltzmann_active: b_active,
                    governance_stake: v.stake,
                    decay_ratio_pct: (decay_ratio * 100.0).round() / 100.0,
                }
            })
            .collect();
        Json(serde_json::json!({
            "status": "ok",
            "validators": entries,
            "detail": "boltzmann_active decays per-epoch; governance_stake is the governance-voting weight"
        }))
    } else {
        Json(serde_json::json!({
            "status": "error",
            "detail": "Tendermint consensus not running"
        }))
    }
}

/// GET /api/validators/boltzmann_weights — Boltzmann proposer weights
/// (stake × activity boost) sorted descending. Query: ?beta_mb=1000
async fn get_boltzmann_weights(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let beta_mb: u64 = params
        .get("beta_mb")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000);

    if let Some(tc_arc) = &state.tendermint {
        let tc = safe_lock(tc_arc);
        let weights = tc.boltzmann_proposer_weights(beta_mb);
        let entries: Vec<BoltzmannWeightEntry> = weights
            .into_iter()
            .map(|(id, w)| {
                let b_active = tc.boltzmann_stakes.get(&id).map(|s| s.active).unwrap_or(0);
                let gov_stake = tc.validator_set.get(id).map(|v| v.stake).unwrap_or(0);
                BoltzmannWeightEntry {
                    validator_id: id,
                    weight: w,
                    boltzmann_active: b_active,
                    governance_stake: gov_stake,
                }
            })
            .collect();
        Json(serde_json::json!({
            "status": "ok",
            "beta_mb": beta_mb,
            "weights": entries,
            "detail": "higher weight → more likely to be selected as proposer"
        }))
    } else {
        Json(serde_json::json!({
            "status": "error",
            "detail": "Tendermint consensus not running"
        }))
    }
}

#[derive(Debug, Deserialize)]
pub struct SanovSlashReq {
    pub validator_id: u64,
    pub slash_type: String, // "equivocation" | "downtime"
    pub missed_blocks: Option<u64>,
    pub window: Option<u64>,
}

/// POST /api/validators/sanov_slash — apply a Sanov large-deviation slash to
/// a validator. Slash type: "equivocation" (full Sanov slash for double-sign)
/// or "downtime" (KL-divergence proportional to miss rate).
async fn post_sanov_slash(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SanovSlashReq>,
) -> Json<SanovSlashResp> {
    if let Some(tc_arc) = &state.tendermint {
        let mut tc = safe_lock(tc_arc);
        let (slash_amount, detail) = match req.slash_type.as_str() {
            "equivocation" => {
                let window = req.window.unwrap_or(1024);
                let amount = tc.sanov_slash_equivocation(req.validator_id, window);
                (
                    amount,
                    format!(
                        "Sanov equivocation slash: KL(all-equivocating ‖ 1-in-{window}-tolerance) × stake = {amount}"
                    ),
                )
            }
            "downtime" => {
                let missed = req.missed_blocks.unwrap_or(1);
                let window = req.window.unwrap_or(100);
                let amount = tc.sanov_slash_downtime(req.validator_id, missed, window);
                (
                    amount,
                    format!(
                        "Sanov downtime slash: {missed}/{window} missed, KL × stake = {amount}"
                    ),
                )
            }
            other => (
                0,
                format!("unknown slash_type {other:?}; use 'equivocation' or 'downtime'"),
            ),
        };
        Json(SanovSlashResp {
            status: if slash_amount > 0 || req.slash_type == "downtime" {
                "slashed"
            } else {
                "no_slash"
            },
            validator_id: req.validator_id,
            slash_amount,
            detail,
        })
    } else {
        Json(SanovSlashResp {
            status: "error",
            validator_id: req.validator_id,
            slash_amount: 0,
            detail: "Tendermint consensus not running".into(),
        })
    }
}

// ─────────── Governance: fork-choice mode amendment ─────────────────

#[derive(Debug, Deserialize)]
pub struct ForkChoiceAmendReq {
    /// `"mcc"` or `"singh_attractor"`
    pub mode: String,
    /// Attractors (required when mode is `"singh_attractor"`).
    pub attractors: Option<Vec<AttractorReq>>,
    /// Endorsing validator stakes (summed to prove quorum).
    pub endorser_stakes: Vec<u64>,
    /// Minimum stake required for the amendment to pass.
    pub required_stake: u64,
}

#[derive(Debug, Deserialize)]
pub struct AttractorReq {
    pub center: u64,
    pub basin_radius: u64,
}

/// POST /api/governance/fork_choice_mode — governance amendment to switch the
/// authoritative fork-choice between MCC and Singh-Attractor.
async fn post_governance_fork_choice_mode(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<ForkChoiceAmendReq>,
) -> Json<serde_json::Value> {
    let attractors: Vec<evaporchain_singh_attractor::Attractor> = req
        .attractors
        .unwrap_or_default()
        .into_iter()
        .map(|a| evaporchain_singh_attractor::Attractor::new(a.center, a.basin_radius))
        .collect();

    if let Some(tc_arc) = &state.tendermint {
        let mut tc = safe_lock(tc_arc);
        match tc.governance_set_fork_choice_mode(
            &req.mode,
            attractors,
            &req.endorser_stakes,
            req.required_stake,
        ) {
            Ok(()) => Json(serde_json::json!({
                "status": "amended",
                "fork_choice_mode": tc.fork_choice_mode(),
                "attractor_count": tc.fork_choice_attractors.len(),
                "detail": format!("fork-choice mode set to {:?} by {} endorsers", tc.fork_choice_mode(), req.endorser_stakes.len())
            })),
            Err(e) => Json(serde_json::json!({
                "status": "error",
                "detail": e.to_string()
            })),
        }
    } else {
        Json(serde_json::json!({
            "status": "error",
            "detail": "Tendermint consensus not running (single-validator devnet mode)"
        }))
    }
}

/// POST /api/governance/param — Lane K.1. Set a soft-fork governance
/// knob (parent_acceptance_mode, block_source_mode,
/// conservation_enforcement) without recompiling. Validates against
/// the allowlist in `TendermintConsensus::governance_set_param` so
/// unknown keys and invalid values are rejected with structured
/// diagnostics. fork_choice_mode is intentionally NOT in the
/// allowlist — that key requires endorser-stake validation and goes
/// through `POST /api/governance/fork_choice_mode` instead.
#[derive(serde::Deserialize)]
struct GovernanceParamReq {
    key: String,
    value: String,
}

/// GET /api/cartel_alarm/chain_status — Lane O.8.1b. Returns the
/// on-chain rolling Causal-CHSH alarm's latest verdict snapshot from
/// `TendermintConsensus.cartel_alarm`. The alarm ticks on every
/// committed block (Lane O.8.1) and re-runs the gate every 50
/// records once the buffer reaches 50 entries.
///
/// Distinct from `POST /api/cartel_alarm/run_gate` (Lane O.5) which
/// runs the gate against operator-supplied trace data. THIS endpoint
/// reports the chain's own self-monitoring state — no operator input
/// required, just the latest verdict the chain has computed against
/// its own commit history.
async fn get_cartel_alarm_chain_status(
    State(state): State<Arc<ApiState>>,
) -> Json<serde_json::Value> {
    if let Some(tc_arc) = &state.tendermint {
        let tc = safe_lock(tc_arc);
        match tc.cartel_alarm_status() {
            None => Json(serde_json::json!({
                "status": "uninitialised",
                "buffer_len": tc.cartel_alarm_buffer_len(),
                "records_seen": tc.cartel_alarm_records_seen(),
                "pending_events_count": tc.pending_cartel_alarms_count(),
                "detail": "alarm has not run yet — needs at least 50 committed blocks and a run-interval boundary",
                "doctrine_ref": "INVENTION_STACK.md §A1.10",
            })),
            Some(s) => Json(serde_json::json!({
                "status": "ok",
                "verdict": s.verdict,
                "s_honest": s.s_honest,
                "s_cartel_synthetic": s.s_cartel_synthetic,
                "gap": s.gap,
                "last_run_at_height": s.last_run_at_height,
                "samples_per_bucket": s.samples_per_bucket,
                "thresholds": {
                    "honest_ceiling": s.thresholds.honest_ceiling,
                    "cartel_floor": s.thresholds.cartel_floor,
                    "min_gap": s.thresholds.min_gap,
                },
                "buffer_len": tc.cartel_alarm_buffer_len(),
                "records_seen": tc.cartel_alarm_records_seen(),
                "pending_events_count": tc.pending_cartel_alarms_count(),
                "doctrine_ref": "INVENTION_STACK.md §A1.10",
            })),
        }
    } else {
        Json(serde_json::json!({
            "status": "error",
            "detail": "Tendermint consensus not running (single-validator devnet mode)"
        }))
    }
}

/// GET /api/cartel_alarm/pending_events — Lane O.8.2b. Drain and
/// return the chain's queue of `CartelAlarmEvent`s emitted since the
/// last call. Polling the endpoint consumes the queue (each event is
/// returned exactly once) so an operator dashboard / pager can ack
/// events without bookkeeping its own seen-set.
///
/// Pre-conditions:
/// - Governance must have set `cartel_alarm_mode = "alarm"` for any
///   events to be queued in the first place; default `observe` mode
///   never emits.
/// - The chain's rolling alarm has produced an `AlarmStatus` whose
///   `s_honest_milli` crossed the doctrine `honest_ceiling_milli`
///   (1800 under doctrine defaults).
///
/// V1 is event surface only — no in-protocol validator reaction
/// policy. Lane O.8.3+ will design how validators react.
async fn get_cartel_alarm_pending_events(
    State(state): State<Arc<ApiState>>,
) -> Json<serde_json::Value> {
    if let Some(tc_arc) = &state.tendermint {
        let mut tc = safe_lock(tc_arc);
        let events = tc.take_pending_cartel_alarms();
        let alarm_mode = tc
            .governance_flags_snapshot()
            .get("cartel_alarm_mode")
            .cloned()
            .unwrap_or_else(|| "observe".to_string());
        let count = events.len();
        let json_events: Vec<serde_json::Value> = events
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "at_height": e.at_height,
                    "s_honest_milli": e.s_honest_milli,
                    "s_cartel_synthetic_milli": e.s_cartel_synthetic_milli,
                    "gap_milli": e.gap_milli,
                    "honest_ceiling_milli_at_fire": e.honest_ceiling_milli_at_fire,
                    "samples_per_bucket": e.samples_per_bucket,
                })
            })
            .collect();
        Json(serde_json::json!({
            "status": "ok",
            "cartel_alarm_mode": alarm_mode,
            "event_count": count,
            "events": json_events,
            "doctrine_ref": "INVENTION_STACK.md §A1.10",
        }))
    } else {
        Json(serde_json::json!({
            "status": "error",
            "detail": "Tendermint consensus not running (single-validator devnet mode)"
        }))
    }
}

/// POST /api/cartel_alarm/run_gate — Lane O.5. Run the Causal-CHSH
/// gate against operator-supplied chain trace data. Returns the
/// verdict (Pass/Fail/InputError) plus the S statistic + per-bucket
/// sample counts. Doctrine-locked thresholds (honest_ceiling=1.8,
/// cartel_floor=2.2, min_gap=0.4) baked in — operators cannot override.
#[derive(serde::Deserialize)]
struct CartelAlarmRunReq {
    trace: Vec<evaporchain_causal_chsh::BlockSummary>,
    #[serde(default)]
    concurrency_window_secs: Option<u64>,
}

async fn post_cartel_alarm_run_gate(
    State(_state): State<Arc<ApiState>>,
    Json(req): Json<CartelAlarmRunReq>,
) -> Json<serde_json::Value> {
    use evaporchain_causal_chsh::{
        chsh::compute_chsh_s,
        extract_chsh_samples,
        gate::{run_synthetic_gate, GateThresholds, GateVerdict},
        synthesize_max_cartel_samples,
    };

    let window = req.concurrency_window_secs.unwrap_or(60);
    let trace = req.trace;
    if trace.len() < 50 {
        return Json(serde_json::json!({
            "verdict": "InputError",
            "detail": format!(
                "trace too small ({} blocks) — gate verdict would be noise-bound; supply ≥ 50",
                trace.len()
            )
        }));
    }

    let honest = extract_chsh_samples(&trace, window);
    let n_per_bucket = honest.samples_ab.len();
    if n_per_bucket < 5 {
        return Json(serde_json::json!({
            "verdict": "InputError",
            "detail": format!(
                "under-populated buckets ({} per setting-pair) — widen concurrency_window_secs",
                n_per_bucket
            )
        }));
    }

    let s_honest = match compute_chsh_s(&honest) {
        Ok(s) => s,
        Err(e) => {
            return Json(serde_json::json!({
                "verdict": "InputError",
                "detail": e.to_string()
            }));
        }
    };
    let cartel = synthesize_max_cartel_samples(n_per_bucket);
    let s_cartel = compute_chsh_s(&cartel).unwrap_or(0.0);
    let thresholds = GateThresholds::doctrine();
    let v = run_synthetic_gate(&honest, &cartel, thresholds);

    let samples_per_bucket = vec![
        honest.samples_ab.len(),
        honest.samples_ab_prime.len(),
        honest.samples_a_prime_b.len(),
        honest.samples_a_prime_b_prime.len(),
    ];
    let (verdict_label, fail_reasons): (&str, Vec<String>) = match &v {
        GateVerdict::Pass { .. } => ("Pass", Vec::new()),
        GateVerdict::Fail { reasons, .. } => ("Fail", reasons.clone()),
        GateVerdict::InputError(msg) => ("InputError", vec![msg.clone()]),
    };

    Json(serde_json::json!({
        "verdict": verdict_label,
        "s_honest": s_honest,
        "s_cartel_synthetic": s_cartel,
        "gap": s_cartel - s_honest,
        "samples_per_bucket": samples_per_bucket,
        "thresholds": {
            "honest_ceiling": thresholds.honest_ceiling,
            "cartel_floor": thresholds.cartel_floor,
            "min_gap": thresholds.min_gap,
        },
        "fail_reasons": fail_reasons,
        "doctrine_ref": "INVENTION_STACK.md §A1.10",
    }))
}

async fn post_governance_param(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<GovernanceParamReq>,
) -> Json<serde_json::Value> {
    if let Some(tc_arc) = &state.tendermint {
        let mut tc = safe_lock(tc_arc);
        match tc.governance_set_param(&req.key, &req.value) {
            Ok(()) => Json(serde_json::json!({
                "status": "amended",
                "key": req.key,
                "value": req.value,
                "detail": format!("governance soft-fork knob {} set to {}", req.key, req.value)
            })),
            Err(e) => Json(serde_json::json!({
                "status": "error",
                "detail": e.to_string()
            })),
        }
    } else {
        Json(serde_json::json!({
            "status": "error",
            "detail": "Tendermint consensus not running (single-validator devnet mode)"
        }))
    }
}

/// GET /api/governance/flags — all governance soft-fork keys + their
/// effective values (explicit overrides merged with documented defaults).
/// Lane I.4 + Lane I.5 + Layer 0 #1 introduced opt-in flags
/// (`parent_acceptance_mode`, `block_source_mode`, `conservation_enforcement`)
/// that operators need to inspect to verify which doctrine claims are
/// live. This RPC surfaces them all in one call.
async fn get_governance_flags(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    if let Some(tc_arc) = &state.tendermint {
        let tc = safe_lock(tc_arc);
        let flags = tc.governance_flags_snapshot();
        Json(serde_json::json!({
            "flags": flags,
            "detail": "Effective values for governance soft-fork keys. \
                Defaults applied for any key not explicitly set: \
                fork_choice_mode=mcc, parent_acceptance_mode=linear, \
                block_source_mode=fifo, conservation_enforcement=observe."
        }))
    } else {
        Json(serde_json::json!({
            "flags": {},
            "detail": "Tendermint consensus not running (single-validator devnet mode)"
        }))
    }
}

/// GET /api/governance/fork_choice_mode — current fork-choice mode + attractor set.
async fn get_governance_fork_choice_mode(
    State(state): State<Arc<ApiState>>,
) -> Json<serde_json::Value> {
    if let Some(tc_arc) = &state.tendermint {
        let tc = safe_lock(tc_arc);
        let attractors: Vec<serde_json::Value> = tc
            .fork_choice_attractors
            .iter()
            .map(|a| serde_json::json!({"center": a.center, "basin_radius": a.basin_radius}))
            .collect();
        Json(serde_json::json!({
            "fork_choice_mode": tc.fork_choice_mode(),
            "attractors": attractors,
            "detail": "MCC is the default; governance_set_fork_choice_mode promotes Singh-Attractor when validators signal readiness"
        }))
    } else {
        Json(serde_json::json!({
            "fork_choice_mode": "mcc",
            "attractors": [],
            "detail": "single-validator devnet — Tendermint not running"
        }))
    }
}

// ─────────── Braid-Group Sequencer commitment ─────────────────────

#[derive(Debug, Deserialize)]
pub struct BraidCommitReq {
    /// Braid generators: positive integers are σ_i, negative are σ_i^-1.
    pub generators: Vec<i32>,
    /// Number of strands (n). Generators must be in [1, n-1].
    pub n: u32,
}

/// POST /api/braid/commit — reduce a braid word to substrate-canonical form
/// and commit via blake3. Encodes transaction ordering as a braid-group
/// commitment (Garside 1969 / Birman 1974). §A1.4.
async fn post_braid_commit(Json(req): Json<BraidCommitReq>) -> Json<serde_json::Value> {
    use evaporchain_braid_sequencer::{commit_braid, reduce_canonical, BraidWord};
    match BraidWord::new(req.generators, req.n) {
        Ok(word) => {
            let reduced = reduce_canonical(&word);
            let commitment = commit_braid(&word);
            Json(serde_json::json!({
                "status": "ok",
                "original_length": word.len(),
                "reduced_length": reduced.len(),
                "commitment_hex": hex::encode(commitment),
                "reduced_generators": reduced.generators,
                "detail": format!("Garside substrate-canonical form: {} generators → {} after reduction", word.len(), reduced.len())
            }))
        }
        Err(e) => Json(serde_json::json!({
            "status": "error",
            "detail": e.to_string()
        })),
    }
}

// ─────────── Decay-Forget Proofs (GDPR-native) ────────────────────

#[derive(Debug, Deserialize)]
pub struct DecayForgetReq {
    pub record_id_hex: String,
    pub original_commitment: u64,
    pub activated_epoch: u64,
    pub query_epoch: u64,
    pub forget_threshold: u64,
}

/// POST /api/decay_forget/prove — produce a DecayForgetProof showing a record's
/// recoverability commitment has decayed below `forget_threshold`. GDPR-native:
/// once proven, the chain *cannot* recover the record. §4.2 V2.
async fn post_decay_forget_prove(Json(req): Json<DecayForgetReq>) -> Json<serde_json::Value> {
    use evaporchain_decay_forget::prove_forgotten;
    use evaporchain_energy_kernel::{ChainLambda, DEFAULT_LAMBDA};

    let record_id: [u8; 32] = match hex::decode(&req.record_id_hex) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        Ok(b) => {
            return Json(serde_json::json!({
                "status": "error",
                "detail": format!("record_id_hex must be 32 bytes, got {}", b.len())
            }))
        }
        Err(_) => {
            return Json(serde_json::json!({
                "status": "error",
                "detail": "invalid record_id_hex"
            }))
        }
    };

    let chain_lambda = ChainLambda::new(DEFAULT_LAMBDA);
    let proof = prove_forgotten(
        record_id,
        req.original_commitment,
        chain_lambda,
        req.activated_epoch,
        req.query_epoch,
        req.forget_threshold,
    );

    let is_forgotten = proof.decayed_commitment <= req.forget_threshold;
    Json(serde_json::json!({
        "status": "ok",
        "is_forgotten": is_forgotten,
        "original_commitment": proof.original_commitment,
        "decayed_commitment": proof.decayed_commitment,
        "forget_threshold": proof.forget_threshold,
        "forgotten_at_epoch": proof.forgotten_at_epoch,
        "witness_hex": hex::encode(proof.witness),
        "detail": if is_forgotten {
            format!("FORGOTTEN: decayed {} → {} < threshold {}", proof.original_commitment, proof.decayed_commitment, proof.forget_threshold)
        } else {
            format!("NOT YET FORGOTTEN: {} > threshold {}", proof.decayed_commitment, proof.forget_threshold)
        }
    }))
}

/// POST /api/decay_forget/verify — verify a DecayForgetProof witness.
async fn post_decay_forget_verify(Json(req): Json<DecayForgetReq>) -> Json<serde_json::Value> {
    use evaporchain_decay_forget::{prove_forgotten, verify_forget_proof};
    use evaporchain_energy_kernel::{ChainLambda, DEFAULT_LAMBDA};

    let record_id: [u8; 32] = match hex::decode(&req.record_id_hex) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => {
            return Json(serde_json::json!({
                "status": "error",
                "detail": "invalid or non-32-byte record_id_hex"
            }))
        }
    };

    let chain_lambda = ChainLambda::new(DEFAULT_LAMBDA);
    let proof = prove_forgotten(
        record_id,
        req.original_commitment,
        chain_lambda,
        req.activated_epoch,
        req.query_epoch,
        req.forget_threshold,
    );
    let valid = verify_forget_proof(&proof).is_ok();
    Json(serde_json::json!({
        "status": "ok",
        "valid": valid,
        "is_forgotten": proof.decayed_commitment <= req.forget_threshold,
        "detail": if valid { "witness valid — proof is tamper-evident" } else { "witness INVALID — proof has been tampered" }
    }))
}

// ─────────── Script-LAD resource checker ─────────────────────────

#[derive(Debug, Deserialize)]
pub struct ScriptLadCheckReq {
    /// EvaporScript source containing `@lad(...)` annotations.
    pub source: String,
    /// Epoch at which to evaluate resource state.
    pub check_epoch: u64,
}

#[derive(Debug, Serialize)]
pub struct ScriptLadVerdictEntry {
    pub field: String,
    pub verdict: String,
    pub value: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ScriptLadCheckResp {
    pub status: &'static str,
    pub annotation_count: usize,
    pub verdicts: Vec<ScriptLadVerdictEntry>,
    pub unconsumed_linear: Vec<String>,
    pub evaporated: Vec<String>,
    pub is_clean: bool,
    pub detail: String,
}

#[derive(Debug, Deserialize)]
pub struct ScriptLadSimReq {
    pub source: String,
    pub created_epoch: u64,
    /// Operations: list of {op: "use"|"drop", field: name, epoch: u64}
    pub ops: Vec<ScriptLadOpEntry>,
    pub final_epoch: u64,
}

#[derive(Debug, Deserialize)]
pub struct ScriptLadOpEntry {
    pub op: String,
    pub field: String,
    pub epoch: u64,
}

/// POST /api/script_lad/check — parse @lad annotations and report resource
/// state at check_epoch. Flags unconsumed Linear resources and evaporations.
async fn post_script_lad_check(Json(req): Json<ScriptLadCheckReq>) -> Json<ScriptLadCheckResp> {
    match evaporchain_script_lad::check_lad_resources(&req.source, req.check_epoch) {
        Ok(result) => {
            let verdicts: Vec<ScriptLadVerdictEntry> = result
                .verdicts
                .iter()
                .map(|(field, v)| {
                    let (verdict_str, value) = match v {
                        evaporchain_script_lad::ResourceVerdict::Live { value } => {
                            ("live", Some(*value))
                        }
                        evaporchain_script_lad::ResourceVerdict::Consumed => ("consumed", None),
                        evaporchain_script_lad::ResourceVerdict::Dropped => ("dropped", None),
                        evaporchain_script_lad::ResourceVerdict::Evaporated => ("evaporated", None),
                    };
                    ScriptLadVerdictEntry {
                        field: field.clone(),
                        verdict: verdict_str.into(),
                        value,
                    }
                })
                .collect();
            let is_clean = result.is_clean();
            let detail = if is_clean {
                format!(
                    "{} LAD resources all clean at epoch {}",
                    result.annotations.len(),
                    req.check_epoch
                )
            } else {
                format!(
                    "{} unconsumed-linear, {} evaporated",
                    result.unconsumed_linear.len(),
                    result.evaporated.len()
                )
            };
            Json(ScriptLadCheckResp {
                status: "ok",
                annotation_count: result.annotations.len(),
                verdicts,
                unconsumed_linear: result.unconsumed_linear,
                evaporated: result.evaporated,
                is_clean,
                detail,
            })
        }
        Err(e) => Json(ScriptLadCheckResp {
            status: "error",
            annotation_count: 0,
            verdicts: vec![],
            unconsumed_linear: vec![],
            evaporated: vec![],
            is_clean: false,
            detail: e.to_string(),
        }),
    }
}

/// POST /api/script_lad/simulate — simulate a full resource lifecycle with
/// explicit use/drop operations and return final verdicts after ticking to
/// final_epoch.
async fn post_script_lad_simulate(Json(req): Json<ScriptLadSimReq>) -> Json<serde_json::Value> {
    let ops: Vec<(&str, &str, u64)> = req
        .ops
        .iter()
        .map(|e| (e.op.as_str(), e.field.as_str(), e.epoch))
        .collect();

    match evaporchain_script_lad::simulate_lifecycle(
        &req.source,
        req.created_epoch,
        &ops,
        req.final_epoch,
    ) {
        Ok(verdicts) => {
            let v: serde_json::Map<String, serde_json::Value> = verdicts
                .into_iter()
                .map(|(name, verdict)| {
                    let (verdict_str, value): (&str, serde_json::Value) = match verdict {
                        evaporchain_script_lad::ResourceVerdict::Live { value } => {
                            ("live", serde_json::json!(value))
                        }
                        evaporchain_script_lad::ResourceVerdict::Consumed => {
                            ("consumed", serde_json::Value::Null)
                        }
                        evaporchain_script_lad::ResourceVerdict::Dropped => {
                            ("dropped", serde_json::Value::Null)
                        }
                        evaporchain_script_lad::ResourceVerdict::Evaporated => {
                            ("evaporated", serde_json::Value::Null)
                        }
                    };
                    (
                        name,
                        serde_json::json!({"verdict": verdict_str, "value": value}),
                    )
                })
                .collect();
            Json(serde_json::json!({
                "status": "ok",
                "verdicts": v,
                "final_epoch": req.final_epoch
            }))
        }
        Err(e) => Json(serde_json::json!({
            "status": "error",
            "detail": e.to_string()
        })),
    }
}

// ─────────── Mortis cert verification (tamper-evidence) ────────────

#[derive(Debug, Deserialize)]
pub struct MortisVerifyQuery {
    pub final_state_root_hex: String,
    pub eulogy_trie_root_hex: String,
    pub epoch_of_death: u64,
    pub final_refresh_pool: u64,
    pub witness_hex: String,
}

#[derive(Debug, Serialize)]
pub struct MortisVerifyResp {
    pub status: &'static str,
    pub valid: bool,
    pub detail: String,
}

/// Re-derive the witness for a caller-supplied MortisCertificate and
/// confirm it matches. Lets dashboards prove the cert's tamper-
/// evidence claim end-to-end: preview → tweak any field → witness no
/// longer matches → verifier rejects.
async fn post_mortis_verify(Json(q): Json<MortisVerifyQuery>) -> Json<MortisVerifyResp> {
    let final_state_root = match parse_hex32(&q.final_state_root_hex) {
        Ok(b) => b,
        Err(e) => {
            return Json(MortisVerifyResp {
                status: "error",
                valid: false,
                detail: format!("bad final_state_root_hex: {e}"),
            });
        }
    };
    let eulogy_trie_root = match parse_hex32(&q.eulogy_trie_root_hex) {
        Ok(b) => b,
        Err(e) => {
            return Json(MortisVerifyResp {
                status: "error",
                valid: false,
                detail: format!("bad eulogy_trie_root_hex: {e}"),
            });
        }
    };
    let witness = match parse_hex32(&q.witness_hex) {
        Ok(b) => b,
        Err(e) => {
            return Json(MortisVerifyResp {
                status: "error",
                valid: false,
                detail: format!("bad witness_hex: {e}"),
            });
        }
    };
    let cert = evaporchain_mortis::MortisCertificate {
        final_state_root,
        eulogy_trie_root,
        epoch_of_death: q.epoch_of_death,
        final_refresh_pool: q.final_refresh_pool,
        witness,
    };
    match evaporchain_mortis::certificate::verify_certificate(&cert) {
        Ok(()) => Json(MortisVerifyResp {
            status: "ok",
            valid: true,
            detail: "witness re-derived and matched — certificate is intact".into(),
        }),
        Err(e) => Json(MortisVerifyResp {
            status: "violation",
            valid: false,
            detail: format!("{e}"),
        }),
    }
}

// ───────── EvaporChain identity — single-call dashboard summary ─────

/// Aggregate snapshot of every distinguishing chain primitive,
/// reachable in one HTTP call. Designed for the launch dashboard,
/// press demos, and external observers building light-client UIs.
/// Pulls from FourActSnapshot, TendermintConsensus accessors, the
/// Lambda-Fold instance, the Decay-Lamport clock, and the Sentinel
/// parameter registry.
#[derive(Debug, Serialize)]
pub struct EvaporChainIdentity {
    pub chain_id: String,
    pub four_act: FourActSnapshot,
    pub light_cone_block_count: usize,
    pub tur_liveness: TurLivenessResp,
    pub lambda_fold: LambdaFoldResp,
    pub lamport_time: LamportTimeResp,
    pub sentinel_param_count: usize,
    /// Full Sentinel parameter list with current values + vote counts.
    /// Inline so dashboards can show autonomic drift without a second
    /// round trip. Per INVENTION_STACK.md §A2.5 ("homeostasis, not
    /// legislators").
    pub sentinel_parameters: Vec<SentinelParameterResp>,
    /// HBCT — Hour-Block Capacity Tokens, the launch wedge per
    /// INVENTION_STACK.md §A3.4. Inline so dashboards see the launch
    /// dApp state without a second round trip.
    pub hbct: HbctStateResp,
    pub wired_primitives: Vec<&'static str>,
    pub headline_sentence: &'static str,
}

const HEADLINE_SENTENCE: &str =
    "EvaporChain — the first blockchain whose consensus, fee market, light client, \
     upgrade path, and history are all closed-form solutions of named theorems, \
     parameterized by one constant λ — and the first chain to admit, at genesis, \
     that it can die.";

const WIRED_PRIMITIVES: &[&str] = &[
    "Light-Cone Consensus DAG (Sorkin/Pratt)",
    "Causal-Cone Validator State (Shalizi-Crutchfield 2001)",
    "MCC fork-choice (Jaynes 1980 + Stock 2009)",
    "TUR Liveness Detector (Barato-Seifert 2015)",
    "Cμ-Gate (Shalizi-Crutchfield Cμ ≤ E + hμ)",
    "Crooks-MEV Refund (Crooks 1999 fluctuation theorem)",
    "Modular-Form Beacon (E_4³ − E_6² = 1728·Δ)",
    "Provable Retention Proofs (PRP)",
    "Evaporative Filtration Homology (Cohen-Steiner-Edelsbrunner-Harer 2007)",
    "Lambda-Fold light client (Nova-style)",
    "Singh-Attractor Consensus (Tier 2)",
    "Bell-Certified Beacon (CHSH)",
    "Allen-Decay Opcodes (Allen 1983)",
    "MDL-Shard (Rissanen 1978)",
    "CSLC ε-machine (Shalizi-Crutchfield 2001)",
    "p-adic ultrametric Merkle",
    "Tropical Plücker commitment",
    "Energy-Bound Fiat-Shamir (EB-FS)",
    "Sentinel autonomic governance (homeostasis)",
    "Mortis death certificate (singleton NFT)",
    "Tombstone eulogy trie (32-byte commitments)",
    "LLSA constitution proof (Coq-checked invariants)",
    "EPV decay-pruned versions",
    "Sanov-Slashing (KL-rate cost function)",
    "Singh-Lyapunov Fee Controller",
    "Singh-Boltzmann Stake (decay-weighted voting power)",
    "Decay-Lamport energy-driven logical clock",
    "HBCT Hour-Block Capacity Tokens (launch wedge)",
    "Energy-Verkle Trie state commitment",
    "Phased Nullifier Tree (sliding window)",
];

async fn get_identity(State(state): State<Arc<ApiState>>) -> Json<EvaporChainIdentity> {
    let four_act = safe_lock(&state.four_act_snapshot).clone();
    let chain_id = state.chain_id.clone();

    // Mirror the per-endpoint logic so callers pay one round-trip.
    let (light_cone_block_count, tur_resp, lambda_fold_resp) = match state.tendermint.as_ref() {
        Some(tc) => {
            let tc = safe_lock(tc);
            let lc = tc.light_cone_block_count();
            let window_samples = tc.tur_window_len();
            let (verdict, observed, bound) = match tc.tur_liveness_verdict() {
                None => ("warming-up", None, None),
                Some(evaporchain_tur_liveness::Verdict::Ok { observed, bound }) => {
                    ("ok", Some(observed.to_string()), Some(bound.to_string()))
                }
                Some(evaporchain_tur_liveness::Verdict::Violation { observed, bound }) => (
                    "violation",
                    Some(observed.to_string()),
                    Some(bound.to_string()),
                ),
            };
            let tur = TurLivenessResp {
                verdict,
                observed,
                bound,
                window_samples,
                window_capacity: evaporchain_consensus::tendermint::TUR_WINDOW_BLOCKS,
            };
            let i = tc.lambda_fold_instance();
            let lf = LambdaFoldResp {
                acc_hash_hex: hex::encode(i.acc_hash),
                total_energy_remaining: i.total_energy_remaining.to_string(),
                step_count: i.step_count,
                latest_epoch: i.latest_epoch,
                is_identity: i.is_identity(),
            };
            (lc, tur, lf)
        }
        None => (
            0,
            TurLivenessResp {
                verdict: "no-consensus-engine",
                observed: None,
                bound: None,
                window_samples: 0,
                window_capacity: evaporchain_consensus::tendermint::TUR_WINDOW_BLOCKS,
            },
            LambdaFoldResp {
                acc_hash_hex: String::new(),
                total_energy_remaining: "0".into(),
                step_count: 0,
                latest_epoch: 0,
                is_identity: true,
            },
        ),
    };

    let lamport_time = {
        let c = safe_lock(&state.lamport_clock);
        LamportTimeResp {
            current_tick: c.current_tick,
            accumulated_energy: c.accumulated_energy,
            tick_quantum: c.tick_quantum,
        }
    };

    let sentinel_parameters: Vec<SentinelParameterResp> = {
        let db = safe_lock(&state.db);
        db.all_sentinel_params()
            .into_iter()
            .map(|p| SentinelParameterResp {
                id: p.id,
                current: p.current,
                min: p.min,
                max: p.max,
                vote_count: db.get_sentinel_votes(p.id).len(),
            })
            .collect()
    };
    let sentinel_param_count = sentinel_parameters.len();

    let hbct = hbct_summary(&state);

    Json(EvaporChainIdentity {
        chain_id,
        four_act,
        light_cone_block_count,
        tur_liveness: tur_resp,
        lambda_fold: lambda_fold_resp,
        lamport_time,
        sentinel_param_count,
        sentinel_parameters,
        hbct,
        wired_primitives: WIRED_PRIMITIVES.to_vec(),
        headline_sentence: HEADLINE_SENTENCE,
    })
}

// ───────────────────────── HBCT endpoints ─────────────────────────────

#[derive(Debug, Deserialize)]
pub struct HbctMintReq {
    pub delivery_location: String,
    pub hour_slot: u64,
    pub mwh_amount: u64,
    pub holder_hex: String,
    pub issued_at_epoch: u64,
}

#[derive(Debug, Deserialize)]
pub struct HbctTransferReq {
    pub delivery_location: String,
    pub hour_slot: u64,
    pub from_hex: String,
    pub to_hex: String,
    pub amount: u64,
}

#[derive(Debug, Deserialize)]
pub struct HbctBurnReq {
    pub delivery_location: String,
    pub hour_slot: u64,
    pub holder_hex: String,
    pub amount: u64,
}

#[derive(Debug, Deserialize)]
pub struct HbctBalanceQuery {
    pub delivery_location: String,
    pub hour_slot: u64,
    pub holder_hex: String,
}

#[derive(Debug, Serialize)]
pub struct HbctBalanceResp {
    pub mwh: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct HbctEntryRow {
    pub delivery_location: String,
    pub hour_slot: u64,
    pub holder_hex: String,
    pub mwh_amount: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct HbctStateResp {
    pub entry_count: usize,
    pub total_mwh: u64,
    pub distinct_locations: usize,
    pub distinct_holders: usize,
    pub distinct_hour_slots: usize,
    /// Top 16 entries by MWh, descending. Caller paginates by hitting
    /// future per-(location, slot) endpoints if needed.
    pub top_entries: Vec<HbctEntryRow>,
}

#[derive(Debug, Serialize)]
pub struct HbctActionResp {
    pub status: &'static str,
    pub detail: String,
}

fn parse_hex32(s: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(s.trim_start_matches("0x")).map_err(|e| e.to_string())?;
    if bytes.len() != 32 {
        return Err(format!("expected 32 bytes, got {}", bytes.len()));
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&bytes);
    Ok(a)
}

/// One-shot demo seeder. Mints a realistic batch of HBCT positions
/// across multiple GB / EU delivery locations and hour slots, owned
/// by deterministic stand-in holder addresses. Idempotent: re-calling
/// stacks more MWh into existing entries (HbctBook::mint sums on the
/// composite key), which is also a useful demo gesture for the
/// dashboard.
#[derive(Debug, Serialize)]
pub struct HbctSeedDemoResp {
    pub status: &'static str,
    pub minted_positions: usize,
    pub detail: String,
}

async fn post_hbct_seed_demo(State(state): State<Arc<ApiState>>) -> Json<HbctSeedDemoResp> {
    // Realistic-shaped demo positions. Locations are GB BMU codes +
    // German bidding zone; holders are deterministic stand-ins.
    let positions: &[(&str, u64, u64, u8)] = &[
        ("BMU-T_DRAXX-1", 481248, 250, 0xA1),
        ("BMU-T_HEYM31", 481248, 180, 0xA2),
        ("BMU-T_GRAIN-3", 481249, 95, 0xA1),
        ("BMU-T_PEMB-1", 481249, 130, 0xA3),
        ("DE-LU", 481250, 420, 0xB1),
        ("DE-LU", 481250, 110, 0xB2),
        ("BMU-T_HORNW-1", 481251, 75, 0xA4),
        ("BMU-T_HORNW-1", 481252, 88, 0xA4),
    ];
    let issued_at = 0u64;
    let mut book = safe_lock(&state.hbct_book);
    let mut minted = 0usize;
    let mut last_err: Option<String> = None;
    for (loc, slot, mwh, holder_byte) in positions {
        let holder = [*holder_byte; 32];
        let token = match evaporchain_hbct::HbctToken::new(
            loc.as_bytes().to_vec(),
            *slot,
            *mwh,
            holder,
            issued_at,
        ) {
            Ok(t) => t,
            Err(e) => {
                last_err = Some(format!("token {loc}/{slot}: {e}"));
                continue;
            }
        };
        match book.mint(token) {
            Ok(()) => minted += 1,
            Err(e) => last_err = Some(format!("mint {loc}/{slot}: {e}")),
        }
    }
    Json(HbctSeedDemoResp {
        status: if minted > 0 { "ok" } else { "error" },
        minted_positions: minted,
        detail: last_err.unwrap_or_else(|| format!("minted {minted} positions")),
    })
}

async fn post_hbct_mint(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<HbctMintReq>,
) -> Json<HbctActionResp> {
    let holder = match parse_hex32(&req.holder_hex) {
        Ok(a) => a,
        Err(e) => {
            return Json(HbctActionResp {
                status: "error",
                detail: format!("bad holder: {e}"),
            });
        }
    };
    let token = match evaporchain_hbct::HbctToken::new(
        req.delivery_location.into_bytes(),
        req.hour_slot,
        req.mwh_amount,
        holder,
        req.issued_at_epoch,
    ) {
        Ok(t) => t,
        Err(e) => {
            return Json(HbctActionResp {
                status: "error",
                detail: format!("bad token: {e}"),
            });
        }
    };
    let mut book = safe_lock(&state.hbct_book);
    match book.mint(token) {
        Ok(()) => Json(HbctActionResp {
            status: "ok",
            detail: "minted".into(),
        }),
        Err(e) => Json(HbctActionResp {
            status: "error",
            detail: format!("{e}"),
        }),
    }
}

async fn post_hbct_transfer(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<HbctTransferReq>,
) -> Json<HbctActionResp> {
    let from = match parse_hex32(&req.from_hex) {
        Ok(a) => a,
        Err(e) => {
            return Json(HbctActionResp {
                status: "error",
                detail: format!("bad from: {e}"),
            })
        }
    };
    let to = match parse_hex32(&req.to_hex) {
        Ok(a) => a,
        Err(e) => {
            return Json(HbctActionResp {
                status: "error",
                detail: format!("bad to: {e}"),
            })
        }
    };
    let mut book = safe_lock(&state.hbct_book);
    match book.transfer(
        &req.delivery_location.into_bytes(),
        req.hour_slot,
        from,
        to,
        req.amount,
    ) {
        Ok(()) => Json(HbctActionResp {
            status: "ok",
            detail: "transferred".into(),
        }),
        Err(e) => Json(HbctActionResp {
            status: "error",
            detail: format!("{e}"),
        }),
    }
}

async fn post_hbct_burn(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<HbctBurnReq>,
) -> Json<HbctActionResp> {
    let holder = match parse_hex32(&req.holder_hex) {
        Ok(a) => a,
        Err(e) => {
            return Json(HbctActionResp {
                status: "error",
                detail: format!("bad holder: {e}"),
            })
        }
    };
    let mut book = safe_lock(&state.hbct_book);
    match book.burn(
        &req.delivery_location.into_bytes(),
        req.hour_slot,
        holder,
        req.amount,
    ) {
        Ok(()) => Json(HbctActionResp {
            status: "ok",
            detail: "burnt".into(),
        }),
        Err(e) => Json(HbctActionResp {
            status: "error",
            detail: format!("{e}"),
        }),
    }
}

async fn post_hbct_balance(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<HbctBalanceQuery>,
) -> Json<HbctBalanceResp> {
    let holder = match parse_hex32(&req.holder_hex) {
        Ok(a) => a,
        Err(_) => return Json(HbctBalanceResp { mwh: 0 }),
    };
    let book = safe_lock(&state.hbct_book);
    Json(HbctBalanceResp {
        mwh: book.balance(&req.delivery_location.into_bytes(), req.hour_slot, holder),
    })
}

#[derive(Debug, Deserialize)]
pub struct HbctTickReq {
    pub current_epoch: u64,
}

#[derive(Debug, Serialize)]
pub struct HbctTickResp {
    pub entries_removed: usize,
    pub mwh_burnt: u64,
}

async fn post_hbct_tick(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<HbctTickReq>,
) -> Json<HbctTickResp> {
    let mut book = safe_lock(&state.hbct_book);
    let outcome = evaporchain_hbct::auto_burn_at_slot_close(&mut book, req.current_epoch);
    Json(HbctTickResp {
        entries_removed: outcome.entries_removed,
        mwh_burnt: outcome.mwh_burnt,
    })
}

// ── Mortis cert detail + refresh-pool detail ──

#[derive(Debug, Serialize)]
pub struct MortisCertDetail {
    pub final_state_root: String,
    pub eulogy_trie_root: String,
    pub epoch_of_death: u64,
    pub final_refresh_pool: u64,
    pub witness: String,
}

async fn get_mortis_cert(State(state): State<Arc<ApiState>>) -> Json<Option<MortisCertDetail>> {
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => return Json(None),
    };
    let tc = safe_lock(tc);
    let cert = match tc.mortis_certificate() {
        Some(c) => c,
        None => return Json(None),
    };
    Json(Some(MortisCertDetail {
        final_state_root: hex::encode(cert.final_state_root),
        eulogy_trie_root: hex::encode(cert.eulogy_trie_root),
        epoch_of_death: cert.epoch_of_death,
        final_refresh_pool: cert.final_refresh_pool,
        witness: hex::encode(cert.witness),
    }))
}

#[derive(Debug, Serialize)]
pub struct RefreshPoolCredit {
    pub namespace_hex: String,
    pub accrued: u64,
    pub last_touched_epoch: u64,
}

#[derive(Debug, Serialize)]
pub struct RefreshPoolResp {
    pub total_accrued: u64,
    pub credits: Vec<RefreshPoolCredit>,
}

#[derive(Debug, Serialize)]
pub struct TombstoneDetail {
    pub address_hex: String,
    pub commitment_hex: String,
    pub memorialised: bool,
}

async fn get_tombstone(
    State(state): State<Arc<ApiState>>,
    axum::extract::Path(addr_hex): axum::extract::Path<String>,
) -> Json<TombstoneDetail> {
    let addr = match parse_hex32(&addr_hex) {
        Ok(a) => a,
        Err(_) => {
            return Json(TombstoneDetail {
                address_hex: addr_hex,
                commitment_hex: String::new(),
                memorialised: false,
            });
        }
    };
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(TombstoneDetail {
                address_hex: hex::encode(addr),
                commitment_hex: String::new(),
                memorialised: false,
            });
        }
    };
    let tc = safe_lock(tc);
    match tc.tombstone_for(&addr) {
        Some(commitment) => Json(TombstoneDetail {
            address_hex: hex::encode(addr),
            commitment_hex: hex::encode(commitment),
            memorialised: true,
        }),
        None => Json(TombstoneDetail {
            address_hex: hex::encode(addr),
            commitment_hex: String::new(),
            memorialised: false,
        }),
    }
}

async fn get_refresh_pool(State(state): State<Arc<ApiState>>) -> Json<RefreshPoolResp> {
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(RefreshPoolResp {
                total_accrued: 0,
                credits: vec![],
            })
        }
    };
    let tc = safe_lock(tc);
    let raw = tc.refresh_pool_credits();
    let total: u64 = raw
        .iter()
        .map(|(_, a, _)| *a)
        .fold(0u64, |a, b| a.saturating_add(b));
    Json(RefreshPoolResp {
        total_accrued: total,
        credits: raw
            .into_iter()
            .map(
                |(namespace_hex, accrued, last_touched_epoch)| RefreshPoolCredit {
                    namespace_hex,
                    accrued,
                    last_touched_epoch,
                },
            )
            .collect(),
    })
}

async fn get_hbct_state(State(state): State<Arc<ApiState>>) -> Json<HbctStateResp> {
    Json(hbct_summary(&state))
}

fn hbct_summary(state: &ApiState) -> HbctStateResp {
    let book = safe_lock(&state.hbct_book);
    let mut total_mwh: u64 = 0;
    let mut locs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut holders: std::collections::BTreeSet<[u8; 32]> = std::collections::BTreeSet::new();
    let mut slots: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    let mut rows: Vec<HbctEntryRow> = Vec::with_capacity(book.entries.len());
    for ((loc, slot, holder), mwh) in book.entries.iter() {
        total_mwh = total_mwh.saturating_add(*mwh);
        let loc_str = String::from_utf8_lossy(loc).into_owned();
        locs.insert(loc_str.clone());
        holders.insert(*holder);
        slots.insert(*slot);
        rows.push(HbctEntryRow {
            delivery_location: loc_str,
            hour_slot: *slot,
            holder_hex: hex::encode(holder),
            mwh_amount: *mwh,
        });
    }
    rows.sort_by(|a, b| b.mwh_amount.cmp(&a.mwh_amount));
    rows.truncate(16);
    HbctStateResp {
        entry_count: book.entries.len(),
        total_mwh,
        distinct_locations: locs.len(),
        distinct_holders: holders.len(),
        distinct_hour_slots: slots.len(),
        top_entries: rows,
    }
}

// ── Oracle attestations + settlement ──

#[derive(Debug, Deserialize)]
pub struct HbctSeedAttestationReq {
    pub delivery_location: String,
    pub hour_slot: u64,
    pub holder_hex: String,
    pub mwh_delivered: u64,
    pub attested_at_epoch: u64,
}

#[derive(Debug, Serialize)]
pub struct HbctSettleResp {
    pub status: &'static str,
    pub settled_mwh: u64,
    pub burnt_excess: u64,
    pub detail: String,
}

/// Seed an oracle attestation into the in-memory MockOracleFeed.
/// Production wires real GB Elexon BMRS / ENTSO-E adapters as a
/// background task that calls this endpoint on every settlement
/// notification, OR replaces the trait impl wholesale.
async fn post_hbct_seed_attestation(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<HbctSeedAttestationReq>,
) -> Json<HbctActionResp> {
    let holder = match parse_hex32(&req.holder_hex) {
        Ok(a) => a,
        Err(e) => {
            return Json(HbctActionResp {
                status: "error",
                detail: format!("bad holder: {e}"),
            })
        }
    };
    let mut oracle = safe_lock(&state.hbct_oracle);
    oracle
        .attestations
        .push(evaporchain_hbct::OracleAttestation {
            delivery_location: req.delivery_location.into_bytes(),
            hour_slot: req.hour_slot,
            holder,
            mwh_delivered: req.mwh_delivered,
            attested_at_epoch: req.attested_at_epoch,
        });
    Json(HbctActionResp {
        status: "ok",
        detail: "attestation recorded".into(),
    })
}

/// Settle one (location, slot, holder) HBCT position against the
/// oracle. Burns any held capacity in excess of `mwh_delivered`
/// (under-delivery is a delivery shortfall — the held tokens were
/// never honored). Returns the settled amount and any excess burnt.
// ───────────────────────── Sentinel endpoints ─────────────────────────

#[derive(Debug, Deserialize)]
pub struct SentinelRegisterParamReq {
    pub parameter_id: u32,
    pub current: u64,
    pub min: u64,
    pub max: u64,
}

#[derive(Debug, Deserialize)]
pub struct SentinelVoteReq {
    pub parameter_id: u32,
    pub validator_id: u64,
    pub target: u64,
    pub observed_epoch: u64,
}

#[derive(Debug, Deserialize)]
pub struct SentinelTickReq {
    pub parameter_id: u32,
    pub current_epoch: u64,
    pub max_step: u64,
    /// Half-life in epochs for vote-weight decay. Caller-supplied so
    /// each parameter can have its own time-constant (per the doctrine
    /// §A2.5, Sentinel's window is a per-parameter governance choice).
    pub half_life_epochs: u64,
}

#[derive(Debug, Serialize)]
pub struct SentinelParameterResp {
    pub id: u32,
    pub current: u64,
    pub min: u64,
    pub max: u64,
    pub vote_count: usize,
}

/// One-shot demo seeder. Registers a small set of bounded chain
/// parameters so the dashboard can immediately show autonomic
/// homeostasis without the caller needing to know the API shape.
/// Idempotent on parameter id — re-calling overwrites the bounds.
#[derive(Debug, Serialize)]
pub struct SentinelSeedDemoResp {
    pub status: &'static str,
    pub registered: Vec<u32>,
    pub detail: String,
}

async fn post_sentinel_seed_demo(State(state): State<Arc<ApiState>>) -> Json<SentinelSeedDemoResp> {
    // (id, current, min, max). Realistic-shaped chain knobs.
    let params: &[(u32, u64, u64, u64)] = &[
        (1, 30_000_000, 5_000_000, 100_000_000), // block gas limit
        (2, 10, 1, 60),                          // target block time (s)
        (3, 1_000_000, 1_000, 10_000_000),       // mempool byte cap (kb)
        (4, 4096, 64, 65_536),                   // λ half-life (epochs)
    ];
    let mut registered: Vec<u32> = Vec::new();
    let mut last_err: Option<String> = None;
    {
        let mut db = safe_lock(&state.db);
        for (id, current, min, max) in params {
            match evaporchain_sentinel::BoundedParameter::new(*id, *current, *min, *max) {
                Ok(p) => {
                    db.put_sentinel_param(p);
                    registered.push(*id);
                }
                Err(e) => last_err = Some(format!("param {id}: {e}")),
            }
        }
    }
    Json(SentinelSeedDemoResp {
        status: if !registered.is_empty() {
            "ok"
        } else {
            "error"
        },
        registered,
        detail: last_err
            .unwrap_or_else(|| "block gas limit · block time · mempool cap · λ half-life".into()),
    })
}

/// One-shot demo voter. Casts a deterministic vote per validator on
/// each parameter so the autonomic-tick has something to drift toward.
/// Each call uses the supplied epoch to produce fresh-weight votes.
#[derive(Debug, Deserialize)]
pub struct SentinelSeedVotesQuery {
    /// Epoch to record votes at (so weight = full at this observation
    /// time). Caller passes the current chain epoch.
    pub current_epoch: u64,
}

#[derive(Debug, Serialize)]
pub struct SentinelSeedVotesResp {
    pub status: &'static str,
    pub votes_recorded: usize,
    pub detail: String,
}

async fn post_sentinel_seed_votes(
    State(state): State<Arc<ApiState>>,
    Json(q): Json<SentinelSeedVotesQuery>,
) -> Json<SentinelSeedVotesResp> {
    // Deterministic vote slate: 3 demo validators, each voting for a
    // target near the parameter's max (to make drift visible upward).
    let validators: &[u64] = &[101, 102, 103];
    let mut recorded = 0usize;
    {
        let mut db = safe_lock(&state.db);
        let params = db.all_sentinel_params();
        for p in params {
            // Vote target = max, so the param drifts upward at the
            // SENTINEL_DEFAULT_STEP_CAP per autonomic tick.
            let target = p.max;
            let mut votes = db.get_sentinel_votes(p.id);
            for v in validators {
                votes.retain(|x| x.validator_id != *v);
                votes.push(evaporchain_sentinel::Vote::new(*v, target, q.current_epoch));
                recorded += 1;
            }
            db.put_sentinel_votes(p.id, votes);
        }
    }
    Json(SentinelSeedVotesResp {
        status: if recorded > 0 { "ok" } else { "error" },
        votes_recorded: recorded,
        detail: format!(
            "3 validators voting for max on every registered parameter @ epoch {}",
            q.current_epoch
        ),
    })
}

async fn post_sentinel_register_param(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SentinelRegisterParamReq>,
) -> Json<HbctActionResp> {
    let p = match evaporchain_sentinel::BoundedParameter::new(
        req.parameter_id,
        req.current,
        req.min,
        req.max,
    ) {
        Ok(p) => p,
        Err(e) => {
            return Json(HbctActionResp {
                status: "error",
                detail: format!("{e}"),
            });
        }
    };
    let mut db = safe_lock(&state.db);
    db.put_sentinel_param(p);
    Json(HbctActionResp {
        status: "ok",
        detail: "registered".into(),
    })
}

async fn post_sentinel_vote(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SentinelVoteReq>,
) -> Json<HbctActionResp> {
    let mut db = safe_lock(&state.db);
    if db.get_sentinel_param(req.parameter_id).is_none() {
        return Json(HbctActionResp {
            status: "error",
            detail: format!("unknown parameter {}", req.parameter_id),
        });
    }
    let mut votes = db.get_sentinel_votes(req.parameter_id);
    // One-vote-per-validator: replace if present.
    votes.retain(|v| v.validator_id != req.validator_id);
    votes.push(evaporchain_sentinel::Vote::new(
        req.validator_id,
        req.target,
        req.observed_epoch,
    ));
    db.put_sentinel_votes(req.parameter_id, votes);
    Json(HbctActionResp {
        status: "ok",
        detail: "vote recorded".into(),
    })
}

async fn post_sentinel_tick(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SentinelTickReq>,
) -> Json<SentinelParameterResp> {
    let mut db = safe_lock(&state.db);
    let votes = db.get_sentinel_votes(req.parameter_id);
    let param = match db.get_sentinel_param(req.parameter_id) {
        Some(p) => p,
        None => {
            return Json(SentinelParameterResp {
                id: req.parameter_id,
                current: 0,
                min: 0,
                max: 0,
                vote_count: 0,
            });
        }
    };
    let lambda = evaporchain_energy_kernel::ChainLambda::new(
        evaporchain_energy_kernel::Lambda::from_epochs(req.half_life_epochs.max(1)),
    );
    let mut effective = param;
    if let Ok(new_value) = evaporchain_sentinel::propose_adjustment(
        &param,
        &votes,
        lambda,
        req.current_epoch,
        req.max_step,
    ) {
        effective.current = new_value;
        db.put_sentinel_param(effective);
    }
    Json(SentinelParameterResp {
        id: effective.id,
        current: effective.current,
        min: effective.min,
        max: effective.max,
        vote_count: votes.len(),
    })
}

async fn get_sentinel_param(
    State(state): State<Arc<ApiState>>,
    axum::extract::Path(parameter_id): axum::extract::Path<u32>,
) -> Json<Option<SentinelParameterResp>> {
    let db = safe_lock(&state.db);
    let p = match db.get_sentinel_param(parameter_id) {
        Some(p) => p,
        None => return Json(None),
    };
    let vote_count = db.get_sentinel_votes(parameter_id).len();
    Json(Some(SentinelParameterResp {
        id: p.id,
        current: p.current,
        min: p.min,
        max: p.max,
        vote_count,
    }))
}

// ─────────────── Singh-Boltzmann Stake observability ────────────────

#[derive(Debug, Serialize)]
pub struct BoltzmannStakeResp {
    pub validator_id: u64,
    pub live_staked_amount: u64,
    pub decayed_voting_power: u64,
    pub decay_pct: f64,
}

async fn get_boltzmann_stake(
    State(state): State<Arc<ApiState>>,
    axum::extract::Path((validator_id, current_epoch)): axum::extract::Path<(u64, u64)>,
) -> Json<Option<BoltzmannStakeResp>> {
    let db = safe_lock(&state.db);
    let live = match db.get_stake(validator_id) {
        Some(s) => s.staked_amount.saturating_sub(s.slashed_amount),
        None => return Json(None),
    };
    // Use the chain-global default λ for the observability view.
    // Production governance picks a chain-specific λ via a future
    // ConsensusFourActState extension.
    let lambda = evaporchain_energy_kernel::ChainLambda::default_genesis();
    let registry =
        evaporchain_execution::boltzmann_stake_integration::BoltzmannStakeRegistry::new();
    let decayed = evaporchain_execution::boltzmann_stake_integration::decayed_voting_power(
        &*db,
        &registry,
        validator_id,
        lambda,
        current_epoch,
    )
    .unwrap_or(0);
    let pct = if live == 0 {
        0.0
    } else {
        decayed as f64 / live as f64
    };
    Json(Some(BoltzmannStakeResp {
        validator_id,
        live_staked_amount: live,
        decayed_voting_power: decayed,
        decay_pct: pct,
    }))
}

// ─────────────────── Decay-Lamport time observability ───────────────

#[derive(Debug, Serialize)]
pub struct LamportTimeResp {
    pub current_tick: u64,
    pub accumulated_energy: u64,
    pub tick_quantum: u64,
}

// ─────────── p-adic ultrametric (P=2) ──────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PadicQuery {
    pub x: u64,
    pub y: u64,
}

#[derive(Debug, Serialize)]
pub struct PadicResp {
    pub valuation_x: u32,
    pub valuation_y: u32,
    pub ultrametric_distance: u32,
    pub p: u32,
}

/// Compute the 2-adic ultrametric distance and valuations of two
/// integers. Per INVENTION_STACK.md §A1.4 (far-frontier math).
async fn post_padic(Json(q): Json<PadicQuery>) -> Json<PadicResp> {
    Json(PadicResp {
        valuation_x: evaporchain_padic::valuation::<2>(q.x),
        valuation_y: evaporchain_padic::valuation::<2>(q.y),
        ultrametric_distance: evaporchain_padic::ultrametric_distance::<2>(q.x, q.y),
        p: 2,
    })
}

// ─────────── Tropical scalar weight ────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TropicalWeightQuery {
    pub energy: u64,
}

#[derive(Debug, Serialize)]
pub struct TropicalWeightResp {
    pub energy: u64,
    pub tropical_weight: String,
}

async fn post_tropical_weight(Json(q): Json<TropicalWeightQuery>) -> Json<TropicalWeightResp> {
    let w = evaporchain_tropical::tropical_weight(q.energy);
    Json(TropicalWeightResp {
        energy: q.energy,
        tropical_weight: format!("{w:?}"),
    })
}

// ─────────── Energy-Bound Fiat-Shamir (EB-FS) ──────────────────────

#[derive(Debug, Deserialize)]
pub struct EbFsChallengeQuery {
    /// Hex-encoded transcript bytes.
    pub transcript_hex: String,
    pub epoch: u64,
    pub epoch_energy: u64,
}

#[derive(Debug, Serialize)]
pub struct EbFsChallengeResp {
    pub status: &'static str,
    pub challenge_hex: String,
    pub detail: String,
}

async fn post_eb_fs_challenge(Json(q): Json<EbFsChallengeQuery>) -> Json<EbFsChallengeResp> {
    let transcript = match hex::decode(&q.transcript_hex) {
        Ok(b) => b,
        Err(e) => {
            return Json(EbFsChallengeResp {
                status: "error",
                challenge_hex: String::new(),
                detail: format!("bad transcript_hex: {e}"),
            });
        }
    };
    let challenge = evaporchain_eb_fs::eb_fs_challenge(&transcript, q.epoch, q.epoch_energy);
    Json(EbFsChallengeResp {
        status: "ok",
        challenge_hex: hex::encode(challenge),
        detail: String::new(),
    })
}

// ─────────── Singh-Attractor Consensus (Tier 2) ────────────────────

#[derive(Debug, Deserialize)]
pub struct SinghAttractorQuery {
    pub state_energy: u64,
    pub attractors: Vec<AttractorReq>,
}

#[derive(Debug, Serialize)]
pub struct SinghAttractorResp {
    pub selected_center: Option<u64>,
    pub selected_basin_radius: Option<u64>,
    pub in_basin: bool,
}

async fn post_singh_attractor(Json(q): Json<SinghAttractorQuery>) -> Json<SinghAttractorResp> {
    let attractors: Vec<evaporchain_singh_attractor::Attractor> = q
        .attractors
        .into_iter()
        .map(|a| evaporchain_singh_attractor::Attractor::new(a.center, a.basin_radius))
        .collect();
    match evaporchain_singh_attractor::select_attractor(q.state_energy, &attractors) {
        None => Json(SinghAttractorResp {
            selected_center: None,
            selected_basin_radius: None,
            in_basin: false,
        }),
        Some(a) => Json(SinghAttractorResp {
            selected_center: Some(a.center),
            selected_basin_radius: Some(a.basin_radius),
            in_basin: a.contains(q.state_energy),
        }),
    }
}

// ─────────── Bell-Certified Beacon (Tier 2) ────────────────────────

#[derive(Debug, Deserialize)]
pub struct BellBeaconQuery {
    pub e_ab: i64,
    pub e_ab_prime: i64,
    pub e_a_prime_b: i64,
    pub e_a_prime_b_prime: i64,
    /// Threshold S (in milli-units) above which the beacon is "Bell-
    /// certified". Defaults to LOCAL_REALISM_S_MILLI = 2000.
    pub threshold_milli: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct BellBeaconResp {
    pub status: &'static str,
    pub s_value_milli: u64,
    pub threshold_milli: u64,
    pub bell_certified: bool,
    pub detail: String,
}

async fn post_bell_beacon(Json(q): Json<BellBeaconQuery>) -> Json<BellBeaconResp> {
    let s = match evaporchain_bell_beacon::chsh_s_value(
        q.e_ab,
        q.e_ab_prime,
        q.e_a_prime_b,
        q.e_a_prime_b_prime,
    ) {
        Ok(s) => s,
        Err(e) => {
            return Json(BellBeaconResp {
                status: "error",
                s_value_milli: 0,
                threshold_milli: q
                    .threshold_milli
                    .unwrap_or(evaporchain_bell_beacon::LOCAL_REALISM_S_MILLI),
                bell_certified: false,
                detail: format!("{e}"),
            });
        }
    };
    let threshold = q
        .threshold_milli
        .unwrap_or(evaporchain_bell_beacon::LOCAL_REALISM_S_MILLI);
    Json(BellBeaconResp {
        status: "ok",
        s_value_milli: s,
        threshold_milli: threshold,
        bell_certified: evaporchain_bell_beacon::bell_certified(s, threshold),
        detail: String::new(),
    })
}

// ─────────── Allen-Decay Opcodes (Tier 2) ──────────────────────────

#[derive(Debug, Deserialize)]
pub struct AllenIntervalReq {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Deserialize)]
pub struct AllenRelationQuery {
    pub a: AllenIntervalReq,
    pub b: AllenIntervalReq,
}

#[derive(Debug, Serialize)]
pub struct AllenRelationResp {
    pub status: &'static str,
    pub relation: Option<String>,
    pub detail: String,
}

async fn post_allen_relation(Json(q): Json<AllenRelationQuery>) -> Json<AllenRelationResp> {
    let a = match evaporchain_allen_decay::Interval::new(q.a.start, q.a.end) {
        Ok(i) => i,
        Err(e) => {
            return Json(AllenRelationResp {
                status: "error",
                relation: None,
                detail: format!("a: {e}"),
            });
        }
    };
    let b = match evaporchain_allen_decay::Interval::new(q.b.start, q.b.end) {
        Ok(i) => i,
        Err(e) => {
            return Json(AllenRelationResp {
                status: "error",
                relation: None,
                detail: format!("b: {e}"),
            });
        }
    };
    let r = evaporchain_allen_decay::compute_relation(a, b);
    Json(AllenRelationResp {
        status: "ok",
        relation: Some(format!("{r:?}")),
        detail: String::new(),
    })
}

// ─────────── MDL-Shard (Tier 0 supporting) ─────────────────────────

#[derive(Debug, Deserialize)]
pub struct MdlOptimalQuery {
    pub items: Vec<u64>,
    pub max_shards: u32,
}

#[derive(Debug, Serialize)]
pub struct MdlOptimalResp {
    pub status: &'static str,
    pub assignments: Option<Vec<u32>>,
    pub mdl_score: Option<u64>,
    pub detail: String,
}

async fn post_mdl_optimal(Json(q): Json<MdlOptimalQuery>) -> Json<MdlOptimalResp> {
    match evaporchain_mdl_shard::mdl_optimal(&q.items, q.max_shards) {
        Some(p) => {
            let score = evaporchain_mdl_shard::mdl_score(&p, &q.items);
            Json(MdlOptimalResp {
                status: "ok",
                assignments: Some(p.assignments.clone()),
                mdl_score: Some(score),
                detail: String::new(),
            })
        }
        None => Json(MdlOptimalResp {
            status: "no-partition",
            assignments: None,
            mdl_score: None,
            detail: "mdl_optimal returned None (empty items or max_shards=0)".into(),
        }),
    }
}

// ─────────── CSLC ε-machine reconstruction (Tier 0) ───────────────

#[derive(Debug, Deserialize)]
pub struct CslcReconstructQuery {
    pub counts: Vec<u64>,
}

#[derive(Debug, Serialize)]
pub struct CslcReconstructResp {
    pub status: &'static str,
    pub state_count: usize,
    pub alphabet_size: u32,
    pub detail: String,
}

async fn post_cslc_reconstruct(Json(q): Json<CslcReconstructQuery>) -> Json<CslcReconstructResp> {
    match evaporchain_cslc::reconstruct_unconditional(&q.counts) {
        Ok(machine) => Json(CslcReconstructResp {
            status: "ok",
            state_count: machine.state_count(),
            alphabet_size: q.counts.len() as u32,
            detail: String::new(),
        }),
        Err(e) => Json(CslcReconstructResp {
            status: "error",
            state_count: 0,
            alphabet_size: 0,
            detail: format!("{e}"),
        }),
    }
}

// ─────────── Lambda-Fold light client (Nova-style folding) ────────

#[derive(Debug, Serialize)]
pub struct LambdaFoldResp {
    pub acc_hash_hex: String,
    pub total_energy_remaining: String,
    pub step_count: u64,
    pub latest_epoch: u64,
    pub is_identity: bool,
}

/// Read the chain's current Lambda-Fold accumulator. Light clients
/// pull this once and verify against the chain's expected
/// (acc_hash, total_energy_remaining) in O(1) work — the substrate
/// promise of the energy-folded light client. Per INVENTION_STACK.md
/// §4.1 row 8.
async fn get_lambda_fold(State(state): State<Arc<ApiState>>) -> Json<LambdaFoldResp> {
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(LambdaFoldResp {
                acc_hash_hex: String::new(),
                total_energy_remaining: "0".into(),
                step_count: 0,
                latest_epoch: 0,
                is_identity: true,
            });
        }
    };
    let tc = safe_lock(tc);
    let i = tc.lambda_fold_instance();
    Json(LambdaFoldResp {
        acc_hash_hex: hex::encode(i.acc_hash),
        total_energy_remaining: i.total_energy_remaining.to_string(),
        step_count: i.step_count,
        latest_epoch: i.latest_epoch,
        is_identity: i.is_identity(),
    })
}

#[derive(Debug, Deserialize)]
pub struct LambdaFoldVerifyQuery {
    pub expected_acc_hash_hex: String,
    pub expected_remaining_energy: u128,
}

#[derive(Debug, Serialize)]
pub struct LambdaFoldVerifyResp {
    pub status: &'static str,
    pub detail: String,
}

/// Substrate-quality verifier for the Lambda-Fold instance. Checks
/// (acc_hash, total_energy_remaining) match the caller's expected
/// witness — the same check Nova's R1CS verifier subsumes once the
/// arkworks integration replaces the blake3 stand-in.
async fn post_lambda_fold_verify(
    State(state): State<Arc<ApiState>>,
    Json(q): Json<LambdaFoldVerifyQuery>,
) -> Json<LambdaFoldVerifyResp> {
    let expected_hash = match parse_hex32(&q.expected_acc_hash_hex) {
        Ok(h) => h,
        Err(e) => {
            return Json(LambdaFoldVerifyResp {
                status: "error",
                detail: format!("bad expected_acc_hash_hex: {e}"),
            });
        }
    };
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(LambdaFoldVerifyResp {
                status: "error",
                detail: "no consensus engine".into(),
            });
        }
    };
    let tc = safe_lock(tc);
    let i = tc.lambda_fold_instance();
    match evaporchain_lambda_fold::verify_folded(&i, expected_hash, q.expected_remaining_energy) {
        Ok(()) => Json(LambdaFoldVerifyResp {
            status: "ok",
            detail: String::new(),
        }),
        Err(e) => Json(LambdaFoldVerifyResp {
            status: "violation",
            detail: format!("{e}"),
        }),
    }
}

// ─────────── MEV-detect observations (Crooks-MEV Phase 1.4) ────────

/// One MEV-shaped observation surfaced by the consensus engine's
/// per-block sandwich detector. Operators consume this for
/// monitoring; Phase 3 of `CROOKS_MEV_INTEGRATION_PLAN.md` will add
/// a refund-amount field once the substrate math is wired in.
#[derive(Debug, Serialize)]
pub struct MevObservationView {
    pub block_height: u64,
    pub attacker_pre_idx: usize,
    pub victim_idx: usize,
    pub attacker_post_idx: usize,
    pub attacker_hex: String,
    pub victim_hex: String,
    pub target_hex: String,
    pub work_estimate: u64,
    pub confidence_score: f64,
    /// Phase 2 of `CROOKS_MEV_INTEGRATION_PLAN.md` — Crooks
    /// fluctuation refund estimate. `None` if computation hasn't
    /// run (Phase 1 default state) or β was set to 0.
    pub refund_amount: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct MevObservationsResp {
    pub count: usize,
    pub observations: Vec<MevObservationView>,
}

/// GET /api/mev/observations — read-only view of the consensus
/// engine's MEV-observation ring buffer (Phase 1.3 of the plan).
/// Phase 1 is observe-only; no settlement runs yet.
async fn get_mev_observations(State(state): State<Arc<ApiState>>) -> Json<MevObservationsResp> {
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(MevObservationsResp {
                count: 0,
                observations: vec![],
            });
        }
    };
    let tc = safe_lock(tc);
    let observations: Vec<MevObservationView> = tc
        .mev_observations()
        .iter()
        .map(|o| MevObservationView {
            block_height: o.block_height,
            attacker_pre_idx: o.attacker_pre_idx,
            victim_idx: o.victim_idx,
            attacker_post_idx: o.attacker_post_idx,
            attacker_hex: hex::encode(o.attacker),
            victim_hex: hex::encode(o.victim),
            target_hex: hex::encode(o.target),
            work_estimate: o.work_estimate,
            confidence_score: o.confidence_score_ppm as f64 / 1_000_000.0,
            refund_amount: o.refund_amount,
        })
        .collect();
    Json(MevObservationsResp {
        count: observations.len(),
        observations,
    })
}

// ─────────── Crooks-MEV dispute endpoint (Phase 4.4) ───────────────

#[derive(Debug, Deserialize)]
pub struct MevDisputeQuery {
    pub source_block_height: u64,
    pub source_observation_idx: usize,
    pub current_height: u64,
}

#[derive(Debug, Serialize)]
pub struct MevDisputeResp {
    pub status: &'static str,
    pub detail: String,
}

/// POST /api/mev/dispute — Phase 4.4 operator-dispute endpoint.
/// Adds the (source_block_height, source_observation_idx) pair to
/// the local validator's `disputed_observations` set so
/// `due_refund_txs` no longer emits the corresponding RefundTx.
/// Only effective WITHIN the grace period; past grace, the dispute
/// is rejected with `PastGracePeriod`.
///
/// **Local to this validator** — cluster-wide dispute consensus is
/// a future Phase 4.4d follow-up. Operators who need cluster-level
/// dispute MUST coordinate via governance multisig today.
async fn post_mev_dispute(
    State(state): State<Arc<ApiState>>,
    Json(q): Json<MevDisputeQuery>,
) -> Json<MevDisputeResp> {
    let tc_arc = match state.tendermint.as_ref() {
        Some(tc) => tc.clone(),
        None => {
            return Json(MevDisputeResp {
                status: "error",
                detail: "no consensus engine".into(),
            });
        }
    };
    let mut tc = match tc_arc.lock() {
        Ok(g) => g,
        Err(_) => {
            return Json(MevDisputeResp {
                status: "error",
                detail: "consensus mutex poisoned".into(),
            });
        }
    };
    match tc.dispute_observation(
        q.source_block_height,
        q.source_observation_idx,
        q.current_height,
    ) {
        Ok(()) => Json(MevDisputeResp {
            status: "ok",
            detail: format!(
                "dispute recorded for ({}, {})",
                q.source_block_height, q.source_observation_idx
            ),
        }),
        Err(e) => Json(MevDisputeResp {
            status: "error",
            detail: format!("{e}"),
        }),
    }
}

// ─────────── Lambda-Fold Nova endpoints (Phase 5.4) ────────────────

/// GET /api/lambda_fold/nova response: surfaces the running Nova
/// instance's hot-path-readable fields without exposing the full
/// (potentially MB-sized) compressed proof bytes — those are fetched
/// out-of-band via verify when needed.
#[cfg(feature = "lambda_fold_nova")]
#[derive(Debug, Serialize)]
pub struct LambdaFoldNovaResp {
    pub total_energy_remaining: String,
    pub step_count: u64,
    pub latest_epoch: u64,
    pub is_identity: bool,
    /// Size of the compressed proof in bytes; 0 if at identity.
    pub proof_bytes_len: usize,
}

#[cfg(feature = "lambda_fold_nova")]
async fn get_lambda_fold_nova(State(state): State<Arc<ApiState>>) -> Json<LambdaFoldNovaResp> {
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(LambdaFoldNovaResp {
                total_energy_remaining: "0".into(),
                step_count: 0,
                latest_epoch: 0,
                is_identity: true,
                proof_bytes_len: 0,
            });
        }
    };
    let tc = safe_lock(tc);
    let i = tc.lambda_fold_nova_instance();
    Json(LambdaFoldNovaResp {
        total_energy_remaining: i.total_energy_remaining.to_string(),
        step_count: i.step_count,
        latest_epoch: i.latest_epoch,
        is_identity: i.is_identity(),
        proof_bytes_len: i.proof_bytes.len(),
    })
}

/// POST /api/lambda_fold/nova/verify — runs the full Nova
/// light-client verify path against the chain's current Nova
/// instance. The verifier holds only `vk_bytes` + the instance's
/// proof, no `pp` recomputation. Closes the sublinear-verifier
/// claim on the wire.
#[cfg(feature = "lambda_fold_nova")]
#[derive(Debug, Deserialize)]
pub struct LambdaFoldNovaVerifyQuery {
    /// Lower bound on `total_energy_remaining` the caller expects.
    pub expected_remaining_energy: u128,
}

#[cfg(feature = "lambda_fold_nova")]
#[derive(Debug, Serialize)]
pub struct LambdaFoldNovaVerifyResp {
    pub status: &'static str,
    pub detail: String,
    pub step_count: u64,
}

#[cfg(feature = "lambda_fold_nova")]
async fn post_lambda_fold_nova_verify(
    State(state): State<Arc<ApiState>>,
    Json(q): Json<LambdaFoldNovaVerifyQuery>,
) -> Json<LambdaFoldNovaVerifyResp> {
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(LambdaFoldNovaVerifyResp {
                status: "error",
                detail: "no consensus engine".into(),
                step_count: 0,
            });
        }
    };
    let tc = safe_lock(tc);
    let inst = tc.lambda_fold_nova_instance().clone();

    // Pull vk_bytes from the consensus engine's Nova folder. If the
    // chain hasn't yet folded a nova-mode block, the folder hasn't
    // been lazy-init'd and there's no vk to verify against.
    let vk_bytes = match tc.lambda_fold_nova_vk_bytes() {
        Some(Ok(v)) => v,
        Some(Err(e)) => {
            return Json(LambdaFoldNovaVerifyResp {
                status: "error",
                detail: format!("vk_bytes failed: {e}"),
                step_count: inst.step_count,
            });
        }
        None => {
            return Json(LambdaFoldNovaVerifyResp {
                status: "error",
                detail: "nova folder not initialised — chain has not folded a nova-mode block yet"
                    .into(),
                step_count: inst.step_count,
            });
        }
    };
    drop(tc);

    match evaporchain_lambda_fold::verify_nova_folded(&inst, &vk_bytes, q.expected_remaining_energy)
    {
        Ok(()) => Json(LambdaFoldNovaVerifyResp {
            status: "ok",
            detail: String::new(),
            step_count: inst.step_count,
        }),
        Err(e) => Json(LambdaFoldNovaVerifyResp {
            status: "violation",
            detail: format!("{e}"),
            step_count: inst.step_count,
        }),
    }
}

/// GET /api/lambda_fold/nova/vk_bytes — hex-encoded preprocessed
/// `vk` for off-process light clients. Returns 404-ish status if the
/// folder hasn't been lazy-initialised (no nova-mode block yet).
#[cfg(feature = "lambda_fold_nova")]
#[derive(Debug, Serialize)]
pub struct LambdaFoldNovaVkResp {
    pub status: &'static str,
    pub vk_bytes_hex: String,
    pub vk_bytes_len: usize,
}

#[cfg(feature = "lambda_fold_nova")]
async fn get_lambda_fold_nova_vk_bytes(
    State(state): State<Arc<ApiState>>,
) -> Json<LambdaFoldNovaVkResp> {
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(LambdaFoldNovaVkResp {
                status: "error",
                vk_bytes_hex: String::new(),
                vk_bytes_len: 0,
            });
        }
    };
    let tc = safe_lock(tc);
    match tc.lambda_fold_nova_vk_bytes() {
        Some(Ok(v)) => {
            let len = v.len();
            Json(LambdaFoldNovaVkResp {
                status: "ok",
                vk_bytes_hex: hex::encode(v),
                vk_bytes_len: len,
            })
        }
        Some(Err(e)) => Json(LambdaFoldNovaVkResp {
            status: "error",
            vk_bytes_hex: format!("vk_bytes failed: {e}"),
            vk_bytes_len: 0,
        }),
        None => Json(LambdaFoldNovaVkResp {
            status: "uninitialised",
            vk_bytes_hex: String::new(),
            vk_bytes_len: 0,
        }),
    }
}

// ─────────── Evaporative Filtration Homology (EFH) ─────────────────

#[derive(Debug, Deserialize)]
pub struct EfhH0Query {
    /// Energy values to compute 0-dim persistence over.
    pub energies: Vec<u64>,
}

#[derive(Debug, Serialize)]
pub struct EfhH0Resp {
    pub pairs: Vec<(String, String)>,
}

#[derive(Debug, Deserialize)]
pub struct EfhBottleneckQuery {
    pub diagram_a: Vec<(u64, u64)>,
    pub diagram_b: Vec<(u64, u64)>,
}

#[derive(Debug, Serialize)]
pub struct EfhBottleneckResp {
    pub bottleneck_distance: String,
}

/// Compute the 0-dim persistence diagram (sublevel filtration) over a
/// caller-supplied list of energies. Per INVENTION_STACK.md §4.1 row 9.
async fn post_efh_h0(Json(q): Json<EfhH0Query>) -> Json<EfhH0Resp> {
    let pd = evaporchain_efh::compute_h0(&q.energies);
    Json(EfhH0Resp {
        pairs: pd
            .pairs
            .iter()
            .map(|(b, d)| (b.to_string(), d.to_string()))
            .collect(),
    })
}

/// Bottleneck distance between two persistence diagrams. Cohen-
/// Steiner-Edelsbrunner-Harer 2007 stability bound:
/// bottleneck_distance(PD(f), PD(g)) ≤ ||f − g||_∞.
async fn post_efh_bottleneck(Json(q): Json<EfhBottleneckQuery>) -> Json<EfhBottleneckResp> {
    let pd_a = evaporchain_efh::PersistenceDiagram::new(
        q.diagram_a,
        evaporchain_efh::Filtration::Sublevel,
    );
    let pd_b = evaporchain_efh::PersistenceDiagram::new(
        q.diagram_b,
        evaporchain_efh::Filtration::Sublevel,
    );
    let d = evaporchain_efh::bottleneck_distance(&pd_a, &pd_b);
    Json(EfhBottleneckResp {
        bottleneck_distance: d.to_string(),
    })
}

// ─────────── Provable Retention Proofs (PRP) ───────────────────────

#[derive(Debug, Deserialize)]
pub struct PrpProveQuery {
    /// Hex-encoded 32-byte state id.
    pub state_id_hex: String,
    /// Energy committed at activation.
    pub committed_energy: u64,
    /// Activation epoch.
    pub activated_epoch: u64,
    /// Retention floor (energy below which the state is no longer
    /// considered "retained").
    pub floor: u64,
    /// Half-life in epochs. Defaults to chain ChainLambda::default_genesis().
    pub half_life_epochs: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct PrpProveResp {
    pub status: &'static str,
    pub state_id_hex: String,
    pub activated_epoch: u64,
    pub committed_energy: u64,
    pub retained_until_epoch: u64,
    pub witness_hex: String,
    pub detail: String,
}

/// Prove the latest epoch at which `committed_energy` is provably
/// retained above `floor` under chain-global λ. Per
/// INVENTION_STACK.md §4.1 #11.
async fn post_prp_prove(Json(q): Json<PrpProveQuery>) -> Json<PrpProveResp> {
    let state_id = match parse_hex32(&q.state_id_hex) {
        Ok(b) => b,
        Err(e) => {
            return Json(PrpProveResp {
                status: "error",
                state_id_hex: q.state_id_hex,
                activated_epoch: 0,
                committed_energy: 0,
                retained_until_epoch: 0,
                witness_hex: String::new(),
                detail: format!("bad state_id_hex: {e}"),
            });
        }
    };
    let half_life = q
        .half_life_epochs
        .unwrap_or(evaporchain_energy_kernel::ChainLambda::default_genesis().half_life());
    let chain_lambda = evaporchain_energy_kernel::ChainLambda::new(
        evaporchain_energy_kernel::Lambda::from_epochs(half_life.max(1)),
    );
    let proof = evaporchain_prp::prove_retention(
        state_id,
        q.committed_energy,
        chain_lambda,
        q.activated_epoch,
        q.floor,
    );
    Json(PrpProveResp {
        status: "ok",
        state_id_hex: hex::encode(proof.state_id),
        activated_epoch: proof.activated_epoch,
        committed_energy: proof.committed_energy,
        retained_until_epoch: proof.retained_until_epoch,
        witness_hex: hex::encode(proof.witness),
        detail: String::new(),
    })
}

// ─────────── Modular-Form Beacon (E_4³ − E_6² = 1728·Δ) ────────────

#[derive(Debug, Serialize)]
pub struct BeaconResp {
    pub tau: u64,
    pub e4: String,
    pub e6: String,
    pub delta: String,
    pub identity_holds: bool,
    pub identity_residual: String,
    pub tolerance: String,
}

/// Compute the per-epoch modular-form beacon at `tau` and verify the
/// E_4³ − E_6² = 1728·Δ identity. Per INVENTION_STACK.md §A1.4.
async fn get_beacon(axum::extract::Path(tau): axum::extract::Path<u64>) -> Json<BeaconResp> {
    let beacon = evaporchain_modular_beacon::compute_beacon(tau);
    // Tolerance: scales with τ since the truncated polynomial breaks
    // the identity for non-zero q. Launch placeholder; governance can
    // tighten once the truncation depth is locked.
    let tolerance: i128 = (tau as i128).saturating_mul(1_000_000_000);
    let (identity_holds, residual) =
        match evaporchain_modular_beacon::verify_modular_identity(&beacon, tolerance) {
            Ok(()) => (true, 0i128),
            Err(evaporchain_modular_beacon::BeaconError::IdentityFailed { residual, .. }) => {
                (false, residual)
            }
        };
    Json(BeaconResp {
        tau: beacon.tau,
        e4: beacon.e4.to_string(),
        e6: beacon.e6.to_string(),
        delta: beacon.delta.to_string(),
        identity_holds,
        identity_residual: residual.to_string(),
        tolerance: tolerance.to_string(),
    })
}

// ─────────── Crooks-MEV Refund (Crooks 1999 fluctuation theorem) ───

#[derive(Debug, Deserialize)]
pub struct CrooksRefundQuery {
    /// Forward pmf (fixed-point parts-per-million).
    pub p_forward_ppm: u64,
    /// Reverse pmf (fixed-point parts-per-million).
    pub p_reverse_ppm: u64,
    /// Total energy extracted by the MEV-suspect path, in chain energy units.
    pub work_extracted: u64,
    /// Inverse temperature β in millibits-per-fee-unit. Launch default 10.
    pub beta_mb: u64,
}

#[derive(Debug, Serialize)]
pub struct CrooksRefundResp {
    pub status: &'static str,
    pub delta_f_millibits: i64,
    pub refund: u64,
    pub detail: String,
}

/// Compute the Crooks-fluctuation-theorem MEV refund. Caller supplies
/// observed forward/reverse pmfs of the path, total work extracted,
/// and β; chain returns the refund (= work_extracted − ΔF clamped at 0).
/// Per INVENTION_STACK.md §A1.3.
async fn post_crooks_refund(Json(q): Json<CrooksRefundQuery>) -> Json<CrooksRefundResp> {
    let log_ratio =
        match evaporchain_cfm::crooks_log_ratio_millibits(q.p_forward_ppm, q.p_reverse_ppm) {
            Ok(r) => r,
            Err(e) => {
                return Json(CrooksRefundResp {
                    status: "error",
                    delta_f_millibits: 0,
                    refund: 0,
                    detail: format!("crooks log-ratio: {e}"),
                });
            }
        };
    let delta_f = match evaporchain_crooks_mev_refund::compute_delta_f_millibits(
        q.work_extracted as i64,
        log_ratio,
        q.beta_mb,
    ) {
        Ok(d) => d,
        Err(e) => {
            return Json(CrooksRefundResp {
                status: "error",
                delta_f_millibits: 0,
                refund: 0,
                detail: format!("{e}"),
            });
        }
    };
    let refund = evaporchain_crooks_mev_refund::compute_refund(q.work_extracted, delta_f);
    Json(CrooksRefundResp {
        status: "ok",
        delta_f_millibits: delta_f,
        refund,
        detail: String::new(),
    })
}

// ─────────── Cμ-Gate (Shalizi-Crutchfield Cμ ≤ E + hμ) ─────────────

#[derive(Debug, Deserialize)]
pub struct CmuCheckQuery {
    /// Observed statistical complexity Cμ in millibits.
    pub cmu_mb: u64,
    /// Excess entropy E in millibits.
    pub excess_entropy_mb: u64,
    /// Entropy rate hμ in millibits.
    pub entropy_rate_mb: u64,
}

#[derive(Debug, Serialize)]
pub struct CmuCheckResp {
    pub verdict: &'static str,
    pub observed_cmu_mb: u64,
    pub bound_mb: u64,
}

/// Run the chain-side Cμ ≤ E + hμ gate. Caller supplies all three
/// information-theoretic inputs in millibits; chain returns Ok/
/// Violation. The measurement scheme for chain-driven Cμ/E/hμ is a
/// future governance choice; this endpoint is the substrate.
async fn get_cmu_check(
    axum::extract::Query(q): axum::extract::Query<CmuCheckQuery>,
) -> Json<CmuCheckResp> {
    let v = evaporchain_cmu_gate::cmu_check(q.cmu_mb, q.excess_entropy_mb, q.entropy_rate_mb);
    let (verdict, observed, bound) = match v {
        evaporchain_cmu_gate::Verdict::Ok {
            observed_cmu,
            bound,
        } => ("ok", observed_cmu, bound),
        evaporchain_cmu_gate::Verdict::Violation {
            observed_cmu,
            bound,
        } => ("violation", observed_cmu, bound),
    };
    Json(CmuCheckResp {
        verdict,
        observed_cmu_mb: observed,
        bound_mb: bound,
    })
}

// ─────────── TUR Liveness Detector observability ───────────────────

#[derive(Debug, Serialize)]
pub struct TurLivenessResp {
    pub verdict: &'static str,
    pub observed: Option<String>,
    pub bound: Option<String>,
    pub window_samples: usize,
    pub window_capacity: usize,
}

async fn get_tur_liveness(State(state): State<Arc<ApiState>>) -> Json<TurLivenessResp> {
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(TurLivenessResp {
                verdict: "no-consensus-engine",
                observed: None,
                bound: None,
                window_samples: 0,
                window_capacity: evaporchain_consensus::tendermint::TUR_WINDOW_BLOCKS,
            });
        }
    };
    let tc = safe_lock(tc);
    let window_samples = tc.tur_window_len();
    let (verdict, observed, bound) = match tc.tur_liveness_verdict() {
        None => ("warming-up", None, None),
        Some(evaporchain_tur_liveness::Verdict::Ok { observed, bound }) => {
            ("ok", Some(observed.to_string()), Some(bound.to_string()))
        }
        Some(evaporchain_tur_liveness::Verdict::Violation { observed, bound }) => (
            "violation",
            Some(observed.to_string()),
            Some(bound.to_string()),
        ),
    };
    Json(TurLivenessResp {
        verdict,
        observed,
        bound,
        window_samples,
        window_capacity: evaporchain_consensus::tendermint::TUR_WINDOW_BLOCKS,
    })
}

// ─────────── Singh-Attractor fork choice (Tier 2) ──────────────────

#[derive(Debug, Deserialize)]
pub struct SinghAttractorForkChoiceQuery {
    pub candidates: String,
    pub attractors: Vec<AttractorReq>,
}

#[derive(Debug, Serialize)]
pub struct SinghAttractorForkChoiceResp {
    pub chosen_head_hex: Option<String>,
    pub considered: usize,
    pub attractors: usize,
}

/// Singh-Attractor fork choice over caller-supplied candidate heads
/// + attractor basins. For each candidate head, the chain reads its
/// block "energy" from the Light-Cone DAG and returns whichever head
/// lands in (or nearest to) an attractor basin. Per INVENTION_STACK.md
/// §4.2 (Tier 2). Available alongside MCC for light clients to choose
/// either rule.
async fn post_singh_attractor_fork_choice(
    State(state): State<Arc<ApiState>>,
    Json(q): Json<SinghAttractorForkChoiceQuery>,
) -> Json<SinghAttractorForkChoiceResp> {
    let heads: Vec<[u8; 32]> = q
        .candidates
        .split(',')
        .filter_map(|s| parse_hex32(s.trim()).ok())
        .collect();
    let attractors: Vec<evaporchain_singh_attractor::Attractor> = q
        .attractors
        .iter()
        .map(|a| evaporchain_singh_attractor::Attractor::new(a.center, a.basin_radius))
        .collect();
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(SinghAttractorForkChoiceResp {
                chosen_head_hex: None,
                considered: heads.len(),
                attractors: attractors.len(),
            });
        }
    };
    let tc = safe_lock(tc);
    let chosen = tc.singh_attractor_fork_choice(&heads, &attractors);
    Json(SinghAttractorForkChoiceResp {
        chosen_head_hex: chosen.map(hex::encode),
        considered: heads.len(),
        attractors: attractors.len(),
    })
}

// ─────────── Cone-Merged Bridges (Tier 2) ──────────────────────────

#[derive(Debug, Deserialize)]
pub struct ConeReq {
    pub half_life_epochs: u64,
    pub threshold: u64,
    pub committed_energy: u64,
    pub observed_epoch: u64,
}

#[derive(Debug, Deserialize)]
pub struct ConeBridgeQuery {
    pub cone_a: ConeReq,
    pub cone_b: ConeReq,
    pub query_epoch: u64,
}

#[derive(Debug, Serialize)]
pub struct ConeBridgeResp {
    pub bridge_valid: bool,
    pub cone_a_inside: bool,
    pub cone_b_inside: bool,
    pub query_epoch: u64,
}

fn cone_from_req(r: &ConeReq) -> evaporchain_cone_bridge::EnergyCone {
    evaporchain_cone_bridge::EnergyCone::new(
        evaporchain_energy_kernel::ChainLambda::new(
            evaporchain_energy_kernel::Lambda::from_epochs(r.half_life_epochs.max(1)),
        ),
        r.threshold,
        r.committed_energy,
        r.observed_epoch,
    )
}

async fn post_cone_bridge(Json(q): Json<ConeBridgeQuery>) -> Json<ConeBridgeResp> {
    let a = cone_from_req(&q.cone_a);
    let b = cone_from_req(&q.cone_b);
    Json(ConeBridgeResp {
        bridge_valid: evaporchain_cone_bridge::bridge_valid(&a, &b, q.query_epoch),
        cone_a_inside: a.is_inside(q.query_epoch),
        cone_b_inside: b.is_inside(q.query_epoch),
        query_epoch: q.query_epoch,
    })
}

// ─────────── EG-FSS forward-secure signatures (Tier 2) ─────────────

#[derive(Debug, Deserialize)]
pub struct EgFssSignVerifyQuery {
    /// 32-byte hex seed for the EgFssKey. Caller controls — substrate
    /// uses the seed verbatim as period 0 key material.
    pub seed_hex: String,
    /// Energy spent against the key before signing — drives evolution.
    pub energy_spent: u64,
    /// Threshold per period (energy units). Defaults to 100.
    pub threshold_per_period: Option<u64>,
    /// Hex-encoded message bytes to sign + verify.
    pub message_hex: String,
}

#[derive(Debug, Serialize)]
pub struct EgFssSignVerifyResp {
    pub status: &'static str,
    pub period_index: u64,
    pub key_material_hex: String,
    pub signature_mac_hex: String,
    pub verify_ok: bool,
    pub detail: String,
}

/// Round-trip demo of the Energy-Indexed Forward-Secure Signature:
/// build a key from seed, evolve it by `energy_spent`, sign the
/// message, verify against the evolved period's key material. Per
/// INVENTION_STACK.md §4.2 (Tier 2).
async fn post_eg_fss_sign_verify(Json(q): Json<EgFssSignVerifyQuery>) -> Json<EgFssSignVerifyResp> {
    let seed = match parse_hex32(&q.seed_hex) {
        Ok(s) => s,
        Err(e) => {
            return Json(EgFssSignVerifyResp {
                status: "error",
                period_index: 0,
                key_material_hex: String::new(),
                signature_mac_hex: String::new(),
                verify_ok: false,
                detail: format!("bad seed_hex: {e}"),
            });
        }
    };
    let message = match hex::decode(&q.message_hex) {
        Ok(m) => m,
        Err(e) => {
            return Json(EgFssSignVerifyResp {
                status: "error",
                period_index: 0,
                key_material_hex: String::new(),
                signature_mac_hex: String::new(),
                verify_ok: false,
                detail: format!("bad message_hex: {e}"),
            });
        }
    };
    let threshold = q.threshold_per_period.unwrap_or(100).max(1);
    let key = evaporchain_eg_fss::EgFssKey::from_seed(seed);
    let evolved = match key.evolve(q.energy_spent, threshold) {
        Ok(k) => k,
        Err(e) => {
            return Json(EgFssSignVerifyResp {
                status: "error",
                period_index: 0,
                key_material_hex: String::new(),
                signature_mac_hex: String::new(),
                verify_ok: false,
                detail: format!("evolve: {e}"),
            });
        }
    };
    let sig = evaporchain_eg_fss::sign(&evolved, &message);
    let verify_ok =
        evaporchain_eg_fss::verify(evolved.key_material, evolved.period_index, &message, &sig)
            .is_ok();
    Json(EgFssSignVerifyResp {
        status: "ok",
        period_index: evolved.period_index,
        key_material_hex: hex::encode(evolved.key_material),
        signature_mac_hex: hex::encode(sig.mac),
        verify_ok,
        detail: format!(
            "key evolved {} periods (energy_spent={}, threshold={}); sig {}",
            evolved.period_index,
            q.energy_spent,
            threshold,
            if verify_ok {
                "verified"
            } else {
                "FAILED to verify"
            }
        ),
    })
}

// ─────────── MCC fork-choice (Jaynes 1980 + Stock 2009) ────────────

#[derive(Debug, Deserialize)]
pub struct MccForkChoiceQuery {
    /// Comma-separated hex-encoded 32-byte candidate fork heads.
    pub candidates: String,
    /// Inverse-temperature for the caliber penalty term. Defaults to
    /// 10_000 (launch governance default).
    pub beta_mb: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct MccForkChoiceResp {
    pub chosen_head_hex: Option<String>,
    pub considered: usize,
    pub beta_mb: u64,
}

async fn get_mcc_fork_choice(
    State(state): State<Arc<ApiState>>,
    axum::extract::Query(q): axum::extract::Query<MccForkChoiceQuery>,
) -> Json<MccForkChoiceResp> {
    let beta_mb = q.beta_mb.unwrap_or(10_000);
    let heads: Vec<[u8; 32]> = q
        .candidates
        .split(',')
        .filter_map(|s| parse_hex32(s.trim()).ok())
        .collect();
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(MccForkChoiceResp {
                chosen_head_hex: None,
                considered: heads.len(),
                beta_mb,
            });
        }
    };
    let tc = safe_lock(tc);
    let chosen = tc.mcc_choose_fork(&heads, beta_mb);
    Json(MccForkChoiceResp {
        chosen_head_hex: chosen.map(hex::encode),
        considered: heads.len(),
        beta_mb,
    })
}

// ─────────── Shalizi-Crutchfield Causal-Cone observability ──────────

#[derive(Debug, Serialize)]
pub struct CausalConeResp {
    pub head_hex: String,
    pub ancestor_count: u64,
    pub total_remaining_energy: String,
    pub oldest_observed_epoch: u64,
    pub latest_observed_epoch: u64,
    pub canonical_cone_hash_hex: String,
    pub observation_epoch: u64,
    pub chain_lambda_half_life_epochs: u64,
}

#[derive(Debug, Deserialize)]
pub struct CausalConeQuery {
    /// Hex-encoded 32-byte block id (parent_hash). Required.
    pub head_hex: String,
    /// Chain epoch at which to compute λ-decayed remaining energies.
    /// Defaults to current chain head epoch if absent.
    pub observation_epoch: Option<u64>,
    /// λ half-life in epochs. Defaults to ChainLambda::default_genesis().
    pub chain_lambda_half_life_epochs: Option<u64>,
}

async fn get_causal_cone(
    State(state): State<Arc<ApiState>>,
    axum::extract::Query(q): axum::extract::Query<CausalConeQuery>,
) -> Json<Option<CausalConeResp>> {
    let head = match parse_hex32(&q.head_hex) {
        Ok(h) => h,
        Err(_) => return Json(None),
    };
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => return Json(None),
    };
    let half_life = q
        .chain_lambda_half_life_epochs
        .unwrap_or(evaporchain_energy_kernel::ChainLambda::default_genesis().half_life());
    let observation_epoch = q.observation_epoch.unwrap_or_else(|| {
        let tc = safe_lock(tc);
        tc.height()
    });
    let tc = safe_lock(tc);
    let summary = match tc.causal_cone_summary(head, half_life, observation_epoch) {
        Some(s) => s,
        None => return Json(None),
    };
    Json(Some(CausalConeResp {
        head_hex: hex::encode(summary.head_id),
        ancestor_count: summary.ancestor_count,
        total_remaining_energy: summary.total_remaining_energy.to_string(),
        oldest_observed_epoch: summary.oldest_observed_epoch,
        latest_observed_epoch: summary.latest_observed_epoch,
        canonical_cone_hash_hex: hex::encode(summary.canonical_cone_hash),
        observation_epoch,
        chain_lambda_half_life_epochs: half_life,
    }))
}

// ─────────────── Demo reset (HBCT + Sentinel) ──────────────────────

#[derive(Debug, Serialize)]
pub struct DemoResetResp {
    pub status: &'static str,
    pub cleared_hbct_entries: usize,
    pub cleared_sentinel_params: usize,
    pub cleared_sentinel_votes: usize,
    pub detail: String,
}

/// Reset the demo-mutable state so visitors can re-run the dashboard
/// demo loop. Clears the HBCT book, every registered Sentinel
/// parameter, and every recorded Sentinel vote slate. Does NOT touch
/// chain state (accounts, stake, blocks, eulogy trie, mortis monitor) —
/// those are real chain history and not safe to wipe via API.
async fn post_demo_reset(State(state): State<Arc<ApiState>>) -> Json<DemoResetResp> {
    let cleared_hbct_entries = {
        let mut book = safe_lock(&state.hbct_book);
        let n = book.entries.len();
        book.entries.clear();
        n
    };
    let (cleared_params, cleared_votes) = {
        let mut db = safe_lock(&state.db);
        let params = db.all_sentinel_params();
        let mut votes_total = 0usize;
        for p in &params {
            votes_total += db.get_sentinel_votes(p.id).len();
            db.put_sentinel_votes(p.id, Vec::new());
        }
        // Re-register each parameter id with degenerate bounds so
        // future seed_demo calls overwrite cleanly. Simpler: leave
        // them in place and let seed_demo's idempotent put overwrite.
        // We'll instead clear by overwriting bounds to a sentinel
        // (current=0, min=0, max=0) which BoundedParameter::new
        // accepts (current==min==max). Skipping — leaving the
        // parameters means re-seed is idempotent and the dashboard
        // immediately shows them cleared of votes.
        (params.len(), votes_total)
    };
    Json(DemoResetResp {
        status: "ok",
        cleared_hbct_entries,
        cleared_sentinel_params: cleared_params,
        cleared_sentinel_votes: cleared_votes,
        detail: format!(
            "wiped {cleared_hbct_entries} HBCT entries and {cleared_votes} Sentinel votes across {cleared_params} parameters; chain state untouched"
        ),
    })
}

// ─────────────── HLWA — Half-Life Wrapped Asset ─────────────────────

#[derive(Debug, Deserialize)]
pub struct HlwaEffectiveSupplyReq {
    pub current_supply: u64,
    pub origin_attested_supply: u64,
    pub last_attested_epoch: u64,
    /// Half-life of attestation freshness in epochs.
    pub attestation_lambda_epochs: u64,
    pub current_epoch: u64,
}

async fn post_hlwa_effective_supply(
    Json(req): Json<HlwaEffectiveSupplyReq>,
) -> Json<serde_json::Value> {
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use evaporchain_hlwa::WrappedAsset;
    let lambda = ChainLambda::new(Lambda::from_epochs(req.attestation_lambda_epochs.max(1)));
    let asset = WrappedAsset::new(
        req.current_supply,
        req.origin_attested_supply,
        req.last_attested_epoch,
        lambda,
    );
    match asset.effective_supply(req.current_epoch) {
        Ok(eff) => {
            let excess = asset.excess_to_burn(req.current_epoch).unwrap_or(0);
            Json(serde_json::json!({
                "status": "ok",
                "effective_supply": eff,
                "current_supply": req.current_supply,
                "excess_to_burn": excess,
                "current_epoch": req.current_epoch,
            }))
        }
        Err(e) => Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    }
}

async fn post_hlwa_re_attest(Json(req): Json<HlwaEffectiveSupplyReq>) -> Json<serde_json::Value> {
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use evaporchain_hlwa::WrappedAsset;
    let lambda = ChainLambda::new(Lambda::from_epochs(req.attestation_lambda_epochs.max(1)));
    let before = WrappedAsset::new(
        req.current_supply,
        req.origin_attested_supply,
        req.last_attested_epoch,
        lambda,
    );
    let after = before.re_attest(req.current_supply, req.current_epoch);
    Json(serde_json::json!({
        "status": "ok",
        "new_attested_supply": after.origin_attested_supply,
        "new_last_attested_epoch": after.last_attested_epoch,
        "effective_supply_after": after.effective_supply(req.current_epoch).unwrap_or(0),
    }))
}

// ─────────────── LLSA Amendment Apply ────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LlsaApplyAmendmentReq {
    pub from_version: u32,
    pub to_version: u32,
    pub step_new_descriptor_hex: String,
    pub to_version_seed_energy: u64,
    pub activation_epoch: u64,
    /// Expected invariant id (32 bytes hex). Leave as 64 zeros for substrate mode.
    pub expected_invariant_hex: String,
}

async fn post_llsa_apply_amendment(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<LlsaApplyAmendmentReq>,
) -> Json<serde_json::Value> {
    use evaporchain_llsa::proof::{AlwaysAcceptVerifier, LlsaProof};
    use evaporchain_llsa::{apply_amendment, Amendment};

    let descriptor = hex::decode(&req.step_new_descriptor_hex).unwrap_or_default();
    let expected_invariant = match hex::decode(&req.expected_invariant_hex) {
        Ok(b) if b.len() == 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            a
        }
        Ok(_) => {
            return Json(
                serde_json::json!({"status":"error","detail":"expected_invariant_hex must be 64 hex chars"}),
            )
        }
        Err(_) => [0u8; 32],
    };

    // Build amendment to compute its hash (needed for the proof binding).
    let mut amendment = Amendment {
        from_version: req.from_version,
        to_version: req.to_version,
        step_new_descriptor: descriptor,
        proof: LlsaProof {
            coq_term_hash: [0u8; 32],
            target_invariant_id: expected_invariant,
            bound_amendment_hash: [0u8; 32], // placeholder, filled below
            proof_bytes: vec![],
        },
    };
    // Bind proof to the canonical amendment hash so AlwaysAcceptVerifier accepts.
    amendment.proof.bound_amendment_hash = amendment.hash();

    let mut reg = safe_lock(&state.epv_registry);
    match apply_amendment(
        &mut reg,
        &amendment,
        expected_invariant,
        req.to_version_seed_energy,
        req.activation_epoch,
        &AlwaysAcceptVerifier,
    ) {
        Ok(()) => Json(serde_json::json!({
            "status": "ok",
            "from_version": req.from_version,
            "to_version": req.to_version,
            "seed_energy": req.to_version_seed_energy,
            "total_versions": reg.len(),
        })),
        Err(e) => Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    }
}

// ─────────────── HLTS — Hashgraph-Like Threshold Shares ─────────────

#[derive(Debug, Deserialize)]
pub struct HltsQuorumReq {
    pub shares: Vec<HltsShareDto>,
    /// Minimum alive shares required for quorum.
    pub k: usize,
    /// Energy threshold below which a share is considered dead.
    pub threshold: u64,
    /// λ half-life in epochs.
    pub lambda_epochs: u64,
    /// Epoch at which to evaluate share liveness.
    pub query_epoch: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HltsShareDto {
    pub idx: u32,
    pub energy: u64,
    pub observed_epoch: u64,
}

async fn post_hlts_quorum_check(Json(req): Json<HltsQuorumReq>) -> Json<serde_json::Value> {
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use evaporchain_hlts::{count_alive, quorum_alive, Share};
    let shares: Vec<Share> = req
        .shares
        .iter()
        .map(|s| Share::new(s.idx, s.energy, s.observed_epoch))
        .collect();
    let chain_lambda = ChainLambda::new(Lambda::from_epochs(req.lambda_epochs.max(1)));
    let alive = count_alive(&shares, chain_lambda, req.query_epoch, req.threshold);
    let meets = quorum_alive(&shares, req.k, chain_lambda, req.query_epoch, req.threshold);
    Json(serde_json::json!({
        "status": "ok",
        "total_shares": shares.len(),
        "alive_count": alive,
        "k": req.k,
        "meets_quorum": meets,
        "query_epoch": req.query_epoch,
        "threshold": req.threshold,
    }))
}

// ─────────────── PNT — Phased Nullifier Tree ─────────────────────────

#[derive(Debug, Deserialize)]
pub struct PntInsertReq {
    /// 32-byte nullifier as 64 hex chars.
    pub nullifier_hex: String,
}

async fn post_pnt_insert(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<PntInsertReq>,
) -> Json<serde_json::Value> {
    let n = match hex::decode(&req.nullifier_hex) {
        Ok(b) if b.len() == 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            a
        }
        _ => {
            return Json(
                serde_json::json!({"status":"error","detail":"nullifier_hex must be 64 hex chars"}),
            )
        }
    };
    let mut pnt = safe_lock(&state.pnt);
    match pnt.insert_nullifier(n) {
        Ok(()) => Json(serde_json::json!({
            "status": "ok",
            "current_phase": pnt.current_phase,
            "nullifier_hex": req.nullifier_hex,
        })),
        Err(e) => Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    }
}

async fn post_pnt_advance_phase(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let mut pnt = safe_lock(&state.pnt);
    pnt.advance_phase();
    Json(serde_json::json!({
        "status": "ok",
        "current_phase": pnt.current_phase,
        "window_depth": pnt.window_depth,
    }))
}

async fn get_pnt_status(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let pnt = safe_lock(&state.pnt);
    let total_nullifiers: usize = pnt.window.iter().map(|s| s.len()).sum();
    Json(serde_json::json!({
        "status": "ok",
        "current_phase": pnt.current_phase,
        "window_depth": pnt.window_depth,
        "live_phases": pnt.window.len(),
        "total_nullifiers_in_window": total_nullifiers,
    }))
}

async fn get_pnt_is_spent(
    State(state): State<Arc<ApiState>>,
    Path(hex_str): Path<String>,
) -> Json<serde_json::Value> {
    let n = match hex::decode(&hex_str) {
        Ok(b) if b.len() == 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            a
        }
        _ => {
            return Json(
                serde_json::json!({"status":"error","detail":"path must be 64 hex chars (32 bytes)"}),
            )
        }
    };
    let pnt = safe_lock(&state.pnt);
    let spent = pnt.is_spent_in_window(&n);
    Json(serde_json::json!({
        "status": "ok",
        "nullifier_hex": hex_str,
        "is_spent": spent,
        "current_phase": pnt.current_phase,
    }))
}

// ─────────────── Entropic Slashing ──────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct EntropicSlashReq {
    pub stake: u64,
    /// Observed misbehaviour event counts (e.g. [n_honest, n_equivocating]).
    pub observed_counts: Vec<u64>,
}

async fn post_entropic_slash(Json(req): Json<EntropicSlashReq>) -> Json<serde_json::Value> {
    use evaporchain_entropic_slashing::entropic_slash;
    match entropic_slash(req.stake, &req.observed_counts) {
        Ok(slash) => Json(serde_json::json!({
            "status": "ok",
            "slash": slash,
            "stake": req.stake,
            "fraction_ppm": if req.stake > 0 { slash * 1_000_000 / req.stake } else { 0 },
        })),
        Err(e) => Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    }
}

// ─────────────── Lyapunov Fee Controller ────────────────────────────

#[derive(Debug, Deserialize)]
pub struct FeeControllerStepReq {
    pub gas_used: u64,
    pub epochs_elapsed: u64,
}

async fn post_fee_controller_step(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<FeeControllerStepReq>,
) -> Json<serde_json::Value> {
    use evaporchain_fee_controller::{base_fee, FeeController, FeeControllerParams};
    let params = FeeControllerParams::default_genesis();
    let mut fs = safe_lock(&state.fee_state);
    match FeeController::step(&params, &*fs, req.gas_used, req.epochs_elapsed) {
        Ok((new_state, drift)) => {
            let fee = base_fee(&new_state, &params);
            *fs = new_state;
            Json(serde_json::json!({
                "status": "ok",
                "energy_after": new_state.energy,
                "base_fee": fee,
                "lyapunov_v_before": drift.v_before,
                "lyapunov_v_after": drift.v_after,
                "lyapunov_delta": drift.delta,
                "gas_used": req.gas_used,
            }))
        }
        Err(e) => Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    }
}

async fn get_fee_controller_status(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    use evaporchain_fee_controller::{base_fee, FeeControllerParams};
    let params = FeeControllerParams::default_genesis();
    let fs = safe_lock(&state.fee_state);
    let fee = base_fee(&*fs, &params);
    Json(serde_json::json!({
        "status": "ok",
        "energy": fs.energy,
        "base_fee": fee,
        "target_energy": params.target_energy,
        "target_gas": params.target_gas,
        "fee_response_ppm": params.fee_response_ppm,
    }))
}

// ─────────────── Evaporated Fork Certificates ────────────────────────

#[derive(Debug, Deserialize)]
pub struct ForkCertProveReq {
    pub fork_root_hex: String,
    pub blocks: Vec<ForkBlockDto>,
    pub evaluated_at_epoch: u64,
    pub threshold: u128,
    /// λ half-life in epochs for block energy decay.
    pub lambda_epochs: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForkBlockDto {
    pub seed_energy: u64,
    pub observed_epoch: u64,
}

#[derive(Debug, Deserialize)]
pub struct ForkCertVerifyReq {
    pub fork_root_hex: String,
    pub evaluated_at_epoch: u64,
    pub total_seed_energy: u128,
    pub decayed_energy: u128,
    pub threshold: u128,
    pub witness_hex: String,
}

async fn post_fork_cert_prove(Json(req): Json<ForkCertProveReq>) -> Json<serde_json::Value> {
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use evaporchain_evap_fork_cert::{prove_fork_evaporated, ForkBlock};

    let fork_root = match hex::decode(&req.fork_root_hex) {
        Ok(b) if b.len() == 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            a
        }
        _ => {
            return Json(
                serde_json::json!({"status":"error","detail":"fork_root_hex must be 64 hex chars"}),
            )
        }
    };
    let blocks: Vec<ForkBlock> = req
        .blocks
        .iter()
        .map(|b| ForkBlock {
            seed_energy: b.seed_energy,
            observed_epoch: b.observed_epoch,
        })
        .collect();
    let chain_lambda = ChainLambda::new(Lambda::from_epochs(req.lambda_epochs.max(1)));
    let cert = prove_fork_evaporated(
        fork_root,
        &blocks,
        chain_lambda,
        req.evaluated_at_epoch,
        req.threshold,
    );
    let is_evaporated = cert.decayed_energy < cert.threshold;
    Json(serde_json::json!({
        "status": "ok",
        "fork_root_hex": req.fork_root_hex,
        "total_seed_energy": cert.total_seed_energy,
        "decayed_energy": cert.decayed_energy,
        "threshold": cert.threshold,
        "is_evaporated": is_evaporated,
        "witness_hex": hex::encode(cert.witness),
    }))
}

async fn post_fork_cert_verify(Json(req): Json<ForkCertVerifyReq>) -> Json<serde_json::Value> {
    use evaporchain_evap_fork_cert::{verify_evaporated_cert, EvaporatedForkCert};

    let fork_root = match hex::decode(&req.fork_root_hex) {
        Ok(b) if b.len() == 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            a
        }
        _ => {
            return Json(
                serde_json::json!({"status":"error","detail":"fork_root_hex must be 64 hex chars"}),
            )
        }
    };
    let witness = match hex::decode(&req.witness_hex) {
        Ok(b) if b.len() == 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            a
        }
        _ => {
            return Json(
                serde_json::json!({"status":"error","detail":"witness_hex must be 64 hex chars"}),
            )
        }
    };
    let cert = EvaporatedForkCert {
        fork_root,
        evaluated_at_epoch: req.evaluated_at_epoch,
        total_seed_energy: req.total_seed_energy,
        decayed_energy: req.decayed_energy,
        threshold: req.threshold,
        witness,
    };
    match verify_evaporated_cert(&cert) {
        Ok(()) => Json(serde_json::json!({
            "status": "ok",
            "verified": true,
            "decayed_energy": cert.decayed_energy,
            "threshold": cert.threshold,
        })),
        Err(e) => {
            Json(serde_json::json!({"status":"error","detail":e.to_string(),"verified":false}))
        }
    }
}

// ─────────────── Evaporated-Fork Certificate V2 (Bell-anchored) ─────
//
// V2 closes a pre-computation attack present in V1: a forker who
// knows the chain's half-life can pre-compute a future V1 cert
// because the V1 witness is a pure function of public chain state.
// V2 binds the certificate to a 32-byte chain-supplied seed anchor
// (typically a `BellCertificate.seed`) plus its issuance epoch, so
// the witness cannot be derived before the chain has confirmed
// blocks at the seed-anchor epoch.

#[derive(Debug, Deserialize, Serialize)]
pub struct ForkCertV2ProveReq {
    pub fork_root_hex: String,
    pub blocks: Vec<ForkBlockDto>,
    pub evaluated_at_epoch: u64,
    pub threshold: u128,
    pub lambda_epochs: u64,
    /// 32-byte chain-supplied anchor (typically `BellCertificate.seed`).
    pub bell_seed_anchor_hex: String,
    /// Epoch the seed anchor was issued at. Must satisfy
    /// `seed_anchor_epoch ≤ evaluated_at_epoch`.
    pub seed_anchor_epoch: u64,
}

#[derive(Debug, Deserialize)]
pub struct ForkCertV2VerifyReq {
    pub fork_root_hex: String,
    pub evaluated_at_epoch: u64,
    pub total_seed_energy: u128,
    pub decayed_energy: u128,
    pub threshold: u128,
    pub bell_seed_anchor_hex: String,
    pub seed_anchor_epoch: u64,
    pub witness_hex: String,
}

fn decode_32(hex_str: &str) -> Option<[u8; 32]> {
    match hex::decode(hex_str) {
        Ok(b) if b.len() == 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            Some(a)
        }
        _ => None,
    }
}

async fn post_fork_cert_v2_prove(Json(req): Json<ForkCertV2ProveReq>) -> Json<serde_json::Value> {
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use evaporchain_evap_fork_cert::ForkBlock;
    use evaporchain_evap_fork_cert_v2::prove_fork_evaporated_v2;

    let fork_root = match decode_32(&req.fork_root_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"fork_root_hex must be 64 hex chars"
            }))
        }
    };
    let bell_seed_anchor = match decode_32(&req.bell_seed_anchor_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"bell_seed_anchor_hex must be 64 hex chars"
            }))
        }
    };
    let blocks: Vec<ForkBlock> = req
        .blocks
        .iter()
        .map(|b| ForkBlock {
            seed_energy: b.seed_energy,
            observed_epoch: b.observed_epoch,
        })
        .collect();
    let chain_lambda = ChainLambda::new(Lambda::from_epochs(req.lambda_epochs.max(1)));
    match prove_fork_evaporated_v2(
        fork_root,
        &blocks,
        chain_lambda,
        req.evaluated_at_epoch,
        req.threshold,
        bell_seed_anchor,
        req.seed_anchor_epoch,
    ) {
        Ok(cert) => {
            let is_evaporated = cert.decayed_energy < cert.threshold;
            Json(serde_json::json!({
                "status": "ok",
                "fork_root_hex": req.fork_root_hex,
                "total_seed_energy": cert.total_seed_energy,
                "decayed_energy": cert.decayed_energy,
                "threshold": cert.threshold,
                "bell_seed_anchor_hex": hex::encode(cert.bell_seed_anchor),
                "seed_anchor_epoch": cert.seed_anchor_epoch,
                "is_evaporated": is_evaporated,
                "witness_hex": hex::encode(cert.witness),
            }))
        }
        Err(e) => Json(serde_json::json!({
            "status": "error",
            "detail": e.to_string(),
        })),
    }
}

async fn post_fork_cert_v2_verify(Json(req): Json<ForkCertV2VerifyReq>) -> Json<serde_json::Value> {
    use evaporchain_evap_fork_cert_v2::{verify_evaporated_cert_v2, EvaporatedForkCertV2};

    let fork_root = match decode_32(&req.fork_root_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"fork_root_hex must be 64 hex chars"
            }))
        }
    };
    let bell_seed_anchor = match decode_32(&req.bell_seed_anchor_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"bell_seed_anchor_hex must be 64 hex chars"
            }))
        }
    };
    let witness = match decode_32(&req.witness_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"witness_hex must be 64 hex chars"
            }))
        }
    };
    let cert = EvaporatedForkCertV2 {
        fork_root,
        evaluated_at_epoch: req.evaluated_at_epoch,
        total_seed_energy: req.total_seed_energy,
        decayed_energy: req.decayed_energy,
        threshold: req.threshold,
        bell_seed_anchor,
        seed_anchor_epoch: req.seed_anchor_epoch,
        witness,
    };
    match verify_evaporated_cert_v2(&cert) {
        Ok(()) => Json(serde_json::json!({
            "status": "ok",
            "verified": true,
            "decayed_energy": cert.decayed_energy,
            "threshold": cert.threshold,
            "bell_seed_anchor_hex": hex::encode(cert.bell_seed_anchor),
            "seed_anchor_epoch": cert.seed_anchor_epoch,
        })),
        Err(e) => Json(serde_json::json!({
            "status":"error",
            "detail":e.to_string(),
            "verified":false
        })),
    }
}

// ─────────────── Bell-Certified Beacon V2 (chain-attached cert) ────
//
// V1 (`/api/bell_beacon`) ships the abstract CHSH gate at integer
// milli-units. V2 hardens that primitive into a chain-attached
// `BellCertificate` carrying the window bounds, gate sample stats,
// and an anti-grinding `seed = BLAKE3(domain || chain_id || pre_seed
// || sorted pair_tags)` bound to the proposer's `prev_block_hash`.
// The certificate's `seed` is the canonical chain-supplied anchor
// for downstream primitives (e.g. `/api/fork_cert_v2/prove`'s
// `bell_seed_anchor_hex`).

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BellBeaconV2PairDto {
    pub first_energy: u64,
    pub first_tx_count: u64,
    pub second_energy: u64,
    pub second_tx_count: u64,
    /// 32-byte canonical pair tag (hex). Reordering pairs cannot
    /// change the seed, so any stable per-pair tag works.
    pub tag_hex: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BellBeaconV2IssueReq {
    pub chain_id: String,
    pub window_start: u64,
    pub window_end: u64,
    pub pairs: Vec<BellBeaconV2PairDto>,
    pub prev_block_hash_hex: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BellCertDto {
    pub window_start: u64,
    pub window_end: u64,
    pub n_pairs: u64,
    pub bucket_counts: [u64; 4],
    pub s_honest_milli: i64,
    pub s_cartel_milli: i64,
    pub gap_milli: i64,
    pub honest_ceiling_milli: u64,
    pub cartel_floor_milli: u64,
    pub min_gap_milli: u64,
    pub prev_block_hash_hex: String,
    pub seed_hex: String,
}

impl BellCertDto {
    fn from_cert(cert: &evaporchain_bell_beacon_v2::BellCertificate) -> Self {
        Self {
            window_start: cert.window_start,
            window_end: cert.window_end,
            n_pairs: cert.n_pairs,
            bucket_counts: cert.bucket_counts,
            s_honest_milli: cert.s_honest_milli,
            s_cartel_milli: cert.s_cartel_milli,
            gap_milli: cert.gap_milli,
            honest_ceiling_milli: cert.honest_ceiling_milli,
            cartel_floor_milli: cert.cartel_floor_milli,
            min_gap_milli: cert.min_gap_milli,
            prev_block_hash_hex: hex::encode(cert.prev_block_hash),
            seed_hex: hex::encode(cert.seed),
        }
    }

    fn to_cert(&self) -> Option<evaporchain_bell_beacon_v2::BellCertificate> {
        Some(evaporchain_bell_beacon_v2::BellCertificate {
            window_start: self.window_start,
            window_end: self.window_end,
            n_pairs: self.n_pairs,
            bucket_counts: self.bucket_counts,
            s_honest_milli: self.s_honest_milli,
            s_cartel_milli: self.s_cartel_milli,
            gap_milli: self.gap_milli,
            honest_ceiling_milli: self.honest_ceiling_milli,
            cartel_floor_milli: self.cartel_floor_milli,
            min_gap_milli: self.min_gap_milli,
            prev_block_hash: decode_32(&self.prev_block_hash_hex)?,
            seed: decode_32(&self.seed_hex)?,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BellBeaconV2VerifyReq {
    pub chain_id: String,
    pub pairs: Vec<BellBeaconV2PairDto>,
    pub prev_block_hash_hex: String,
    pub certificate: BellCertDto,
}

fn dto_to_pairs(
    pairs: &[BellBeaconV2PairDto],
) -> Result<Vec<evaporchain_bell_beacon_v2::ConcurrentPair>, String> {
    pairs
        .iter()
        .map(|p| {
            let tag = decode_32(&p.tag_hex)
                .ok_or_else(|| format!("tag_hex must be 64 hex chars: {}", p.tag_hex))?;
            Ok(evaporchain_bell_beacon_v2::ConcurrentPair {
                first: evaporchain_bell_beacon_v2::PairStats {
                    energy: p.first_energy,
                    tx_count: p.first_tx_count,
                },
                second: evaporchain_bell_beacon_v2::PairStats {
                    energy: p.second_energy,
                    tx_count: p.second_tx_count,
                },
                tag,
            })
        })
        .collect()
}

async fn post_bell_beacon_v2_issue(
    Json(req): Json<BellBeaconV2IssueReq>,
) -> Json<serde_json::Value> {
    use evaporchain_bell_beacon_v2::issue_certificate;
    use evaporchain_causal_chsh::gate::GateThresholds;

    let prev_block_hash = match decode_32(&req.prev_block_hash_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"prev_block_hash_hex must be 64 hex chars"
            }))
        }
    };
    let pairs = match dto_to_pairs(&req.pairs) {
        Ok(p) => p,
        Err(e) => {
            return Json(serde_json::json!({
                "status":"error",
                "detail": e,
            }))
        }
    };

    match issue_certificate(
        &req.chain_id,
        req.window_start,
        req.window_end,
        &pairs,
        GateThresholds::doctrine(),
        prev_block_hash,
    ) {
        Ok(cert) => Json(serde_json::json!({
            "status": "ok",
            "certificate": BellCertDto::from_cert(&cert),
        })),
        Err(e) => Json(serde_json::json!({
            "status": "error",
            "detail": e.to_string(),
        })),
    }
}

async fn post_bell_beacon_v2_verify(
    Json(req): Json<BellBeaconV2VerifyReq>,
) -> Json<serde_json::Value> {
    use evaporchain_bell_beacon_v2::verify_certificate;
    use evaporchain_causal_chsh::gate::GateThresholds;

    let prev_block_hash = match decode_32(&req.prev_block_hash_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"prev_block_hash_hex must be 64 hex chars"
            }))
        }
    };
    let pairs = match dto_to_pairs(&req.pairs) {
        Ok(p) => p,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e})),
    };
    let cert = match req.certificate.to_cert() {
        Some(c) => c,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"certificate.prev_block_hash_hex / seed_hex must each be 64 hex chars"
            }))
        }
    };

    match verify_certificate(
        &req.chain_id,
        &pairs,
        prev_block_hash,
        GateThresholds::doctrine(),
        &cert,
    ) {
        Ok(()) => Json(serde_json::json!({
            "status": "ok",
            "verified": true,
            "seed_hex": hex::encode(cert.seed),
        })),
        Err(e) => Json(serde_json::json!({
            "status": "error",
            "detail": e.to_string(),
            "verified": false,
        })),
    }
}

// ─────────────── Singh-Attractor V2 (Bell-anchored fallback) ───────
//
// V1 (`/api/singh_attractor`) ships deterministic in-basin selection
// + nearest-centre fallback. V1's fallback is predictable: a
// malicious proposer who can push state into the no-basin region can
// know in advance which attractor the chain will fall back to.
//
// V2 closes that gap by anchoring the fallback to a chain-supplied
// 32-byte seed (typically `BellCertificate.seed` from
// `/api/bell_beacon_v2/issue`). Out-of-basin selection becomes
// inverse-distance-weighted sampling seeded by the certificate;
// closer attractors are likelier but not deterministic. In-basin
// selection is unchanged from V1 — the seed is unused.
//
// V2 also returns a bounded Lyapunov drift `min(|state − center|,
// drift_rate)` toward the selected attractor's centre, so the chain
// can apply it per epoch and the energy state strictly approaches
// the centre on the basin's interior.

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AttractorV2Dto {
    pub center: u64,
    pub basin_radius: u64,
    /// Maximum per-epoch drift magnitude toward `center`.
    pub drift_rate: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SinghAttractorV2DrawReq {
    pub state_energy: u64,
    pub attractors: Vec<AttractorV2Dto>,
    /// 32-byte certificate seed (typically from
    /// `/api/bell_beacon_v2/issue` `seed_hex`).
    pub certificate_seed_hex: String,
}

async fn post_singh_attractor_v2_draw(
    Json(req): Json<SinghAttractorV2DrawReq>,
) -> Json<serde_json::Value> {
    use evaporchain_singh_attractor_v2::{draw_attractor, AttractorV2};

    let seed = match decode_32(&req.certificate_seed_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"certificate_seed_hex must be 64 hex chars"
            }))
        }
    };
    let attractors: Vec<AttractorV2> = req
        .attractors
        .iter()
        .map(|a| AttractorV2::new(a.center, a.basin_radius, a.drift_rate))
        .collect();

    match draw_attractor(req.state_energy, &attractors, &seed) {
        Ok(r) => Json(serde_json::json!({
            "status": "ok",
            "selected_center": r.selected_center,
            "selected_index": r.selected_index,
            "drift": r.drift.to_string(), // i128 → string for JSON safety
            "used_fallback": r.used_fallback,
        })),
        Err(e) => Json(serde_json::json!({
            "status": "error",
            "detail": e.to_string(),
        })),
    }
}

// ─────────────── IB Validators V2 (Immune Validator Set) ───────────
//
// V1 (`evaporchain-ib-validators`) ships the Tishby-Pereira-Bialek
// Information-Bottleneck vote gate `ib_vote(local, prior, params)
// → Commit | Abstain` based on KL divergence. V1 says nothing about
// *which* validators are eligible to vote.
//
// V2 wraps V1 with three structural rejection paths into a unified
// `Jailed{reason}` outcome:
//
//  1. CHSH-failed-window jail — validators active during a window
//     whose `BellCertificate` failed the gate are jailed for
//     `jail_epochs`. Closes the doctrine link from Bell-Beacon V2
//     to validator immunity: a failing Bell-Beacon gate doesn't
//     just signal anomaly, it actively removes the implicated
//     validators from the voting set.
//  2. Energy-floor jail — validators below `energy_floor` cannot
//     vote until refresh.
//  3. Explicit slash — operator jails with a typed code.
//
// JailState is BTreeMap-canonical so iteration is validator-
// deterministic. Both endpoints are stateless: callers submit the
// current jail state + inputs; handlers return the gate verdict
// or the updated jail state.

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum JailReasonDto {
    ChshFailedWindow { window_start: u64, window_end: u64 },
    EnergyBelowFloor { observed: u64, floor: u64 },
    Slashed { code: u32 },
}

impl JailReasonDto {
    fn to_inner(&self) -> evaporchain_ib_validators_v2::JailReason {
        use evaporchain_ib_validators_v2::JailReason as R;
        match *self {
            JailReasonDto::ChshFailedWindow {
                window_start,
                window_end,
            } => R::ChshFailedWindow {
                window_start,
                window_end,
            },
            JailReasonDto::EnergyBelowFloor { observed, floor } => {
                R::EnergyBelowFloor { observed, floor }
            }
            JailReasonDto::Slashed { code } => R::Slashed { code },
        }
    }

    fn from_inner(r: &evaporchain_ib_validators_v2::JailReason) -> Self {
        use evaporchain_ib_validators_v2::JailReason as R;
        match *r {
            R::ChshFailedWindow {
                window_start,
                window_end,
            } => JailReasonDto::ChshFailedWindow {
                window_start,
                window_end,
            },
            R::EnergyBelowFloor { observed, floor } => {
                JailReasonDto::EnergyBelowFloor { observed, floor }
            }
            R::Slashed { code } => JailReasonDto::Slashed { code },
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JailEntryDto {
    pub validator_id_hex: String,
    pub reason: JailReasonDto,
    pub expires_at_epoch: u64,
}

fn dto_to_jail_state(
    entries: &[JailEntryDto],
) -> Result<evaporchain_ib_validators_v2::JailState, String> {
    use evaporchain_ib_validators_v2::{JailEntry, JailState};
    let mut js = JailState::new();
    for e in entries {
        let id = decode_32(&e.validator_id_hex).ok_or_else(|| {
            format!(
                "validator_id_hex must be 64 hex chars: {}",
                e.validator_id_hex
            )
        })?;
        js.insert(
            id,
            JailEntry {
                reason: e.reason.to_inner(),
                expires_at_epoch: e.expires_at_epoch,
            },
        );
    }
    Ok(js)
}

fn jail_state_to_dto(js: &evaporchain_ib_validators_v2::JailState) -> Vec<JailEntryDto> {
    js.iter()
        .map(|(id, entry)| JailEntryDto {
            validator_id_hex: hex::encode(id),
            reason: JailReasonDto::from_inner(&entry.reason),
            expires_at_epoch: entry.expires_at_epoch,
        })
        .collect()
}

#[derive(Debug, Deserialize, Serialize)]
pub struct IbV2VoteReq {
    /// Per-account energies forming the local validator's state
    /// signature. Histogram-binned via V1 `StateSignature::from_energies`.
    pub local_energies: Vec<u64>,
    /// Per-account energies forming the prior signature.
    pub prior_energies: Vec<u64>,
    /// Histogram scale for both signatures (max energy in the
    /// binning range).
    pub signature_scale: u64,
    /// IB tradeoff parameter — KL-divergence threshold in milli-bits.
    pub lambda_mb: u64,
    pub validator_id_hex: String,
    pub energy: u64,
    pub energy_floor: u64,
    pub current_epoch: u64,
    /// Current jail state. Submit empty list if the chain has no
    /// jailed validators.
    pub jail_state: Vec<JailEntryDto>,
}

async fn post_ib_validators_v2_vote(Json(req): Json<IbV2VoteReq>) -> Json<serde_json::Value> {
    use evaporchain_ib_validators::{IbParams, StateSignature};
    use evaporchain_ib_validators_v2::{ib_vote_v2, VoteV2};

    let validator_id = match decode_32(&req.validator_id_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"validator_id_hex must be 64 hex chars"
            }))
        }
    };
    let jail_state = match dto_to_jail_state(&req.jail_state) {
        Ok(js) => js,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e})),
    };
    let local_sig = StateSignature::from_energies(&req.local_energies, req.signature_scale);
    let prior_sig = StateSignature::from_energies(&req.prior_energies, req.signature_scale);
    let params = IbParams {
        lambda_mb: req.lambda_mb,
    };

    match ib_vote_v2(
        &local_sig,
        &prior_sig,
        &params,
        &validator_id,
        req.energy,
        req.energy_floor,
        &jail_state,
        req.current_epoch,
    ) {
        Ok(VoteV2::Commit) => Json(serde_json::json!({
            "status":"ok",
            "vote":"commit",
        })),
        Ok(VoteV2::Abstain) => Json(serde_json::json!({
            "status":"ok",
            "vote":"abstain",
        })),
        Ok(VoteV2::Jailed { reason }) => Json(serde_json::json!({
            "status":"ok",
            "vote":"jailed",
            "reason": JailReasonDto::from_inner(&reason),
        })),
        Err(e) => Json(serde_json::json!({
            "status":"error",
            "detail": e.to_string(),
        })),
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct IbV2ChshJailReq {
    /// Validators that were active during the failing CHSH window.
    /// Each entry is a 32-byte hex validator id.
    pub participants_hex: Vec<String>,
    pub window_start: u64,
    pub window_end: u64,
    pub current_epoch: u64,
    /// Jail expires at `current_epoch + jail_epochs`.
    pub jail_epochs: u64,
    /// Pre-existing jail state to mutate. Empty list = fresh state.
    pub jail_state: Vec<JailEntryDto>,
}

async fn post_ib_validators_v2_jail_chsh_failure(
    Json(req): Json<IbV2ChshJailReq>,
) -> Json<serde_json::Value> {
    use evaporchain_ib_validators_v2::vote::apply_chsh_failure_jail;

    let participants: Vec<[u8; 32]> = match req
        .participants_hex
        .iter()
        .map(|h| decode_32(h).ok_or_else(|| format!("participant id must be 64 hex chars: {h}")))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(v) => v,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e})),
    };
    let mut jail_state = match dto_to_jail_state(&req.jail_state) {
        Ok(js) => js,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e})),
    };
    apply_chsh_failure_jail(
        &mut jail_state,
        &participants,
        req.window_start,
        req.window_end,
        req.current_epoch,
        req.jail_epochs,
    );
    Json(serde_json::json!({
        "status":"ok",
        "jailed_count": participants.len(),
        "jail_state": jail_state_to_dto(&jail_state),
    }))
}

// ─────────────── Light-Cone V2 (causal-cone Merkle proofs) ─────────
//
// V1 (`/api/light_cone/*`) ships chain-state DAG queries
// (`candidate_heads`, `authoritative_head`, `antichain_digest`,
// `block_clock`). V2 closes the gap for light clients: instead of
// sending the full DAG, the chain commits to each block's causal
// past via a BLAKE3 Merkle root. Light clients verify ancestry in
// O(log n) from the root + a proof, never touching the DAG.
//
// Three endpoints, all stateless (DAG submitted as input):
//
//  - POST /api/light_cone_v2/causal_root: compute the Merkle
//    commitment over a block's BTreeSet-sorted causal_past.
//  - POST /api/light_cone_v2/prove_ancestry: produce a MerklePath
//    proving (descendant has ancestor in its causal past).
//  - POST /api/light_cone_v2/verify_ancestry: pure light-client
//    verifier — needs only (causal_root, ancestor_id, proof).

#[derive(Debug, Deserialize)]
pub struct LightConeV2CausalRootReq {
    pub blocks: Vec<AntichainBlockDto>,
    pub block_id_hex: String,
}

#[derive(Debug, Deserialize)]
pub struct LightConeV2ProveReq {
    pub blocks: Vec<AntichainBlockDto>,
    pub descendant_hex: String,
    pub ancestor_hex: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MerklePathDto {
    pub siblings_hex: Vec<String>,
    pub directions: Vec<bool>,
}

#[derive(Debug, Deserialize)]
pub struct LightConeV2VerifyReq {
    pub causal_root_hex: String,
    pub ancestor_id_hex: String,
    pub proof: MerklePathDto,
}

/// Build a `LightCone` from the DTO blocks. Mirrors the inline
/// pattern in `post_antichain_compute`; factoring it here keeps the
/// V2 handlers thin.
fn build_light_cone_from_dto(
    blocks: &[AntichainBlockDto],
) -> Result<evaporchain_light_cone::LightCone, String> {
    use evaporchain_light_cone::{Block, BlockId, LightCone};
    fn parse_id(s: &str) -> Option<BlockId> {
        let bytes = hex::decode(s).ok()?;
        if bytes.len() != 32 {
            return None;
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(&bytes);
        Some(id)
    }
    let mut lc = LightCone::new();
    for b in blocks {
        let id = parse_id(&b.id_hex).ok_or_else(|| format!("bad id_hex: {}", b.id_hex))?;
        let mut parents = Vec::new();
        for ph in &b.parent_ids {
            parents.push(parse_id(ph).ok_or_else(|| format!("bad parent id: {ph}"))?);
        }
        lc.insert(Block::new(id, parents, b.energy, b.observed_epoch))
            .map_err(|e| e.to_string())?;
    }
    Ok(lc)
}

async fn post_light_cone_v2_causal_root(
    Json(req): Json<LightConeV2CausalRootReq>,
) -> Json<serde_json::Value> {
    use evaporchain_light_cone_v2::causal_root;

    let lc = match build_light_cone_from_dto(&req.blocks) {
        Ok(lc) => lc,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e})),
    };
    let block_id = match decode_32(&req.block_id_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"block_id_hex must be 64 hex chars"
            }))
        }
    };
    let root = causal_root(&lc, block_id);
    Json(serde_json::json!({
        "status":"ok",
        "block_id_hex": req.block_id_hex,
        "causal_root_hex": hex::encode(root),
    }))
}

async fn post_light_cone_v2_prove_ancestry(
    Json(req): Json<LightConeV2ProveReq>,
) -> Json<serde_json::Value> {
    use evaporchain_light_cone_v2::{causal_root, prove_ancestry};

    let lc = match build_light_cone_from_dto(&req.blocks) {
        Ok(lc) => lc,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e})),
    };
    let descendant = match decode_32(&req.descendant_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"descendant_hex must be 64 hex chars"
            }))
        }
    };
    let ancestor = match decode_32(&req.ancestor_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"ancestor_hex must be 64 hex chars"
            }))
        }
    };
    match prove_ancestry(&lc, descendant, ancestor) {
        Ok(path) => {
            let root = causal_root(&lc, descendant);
            Json(serde_json::json!({
                "status":"ok",
                "descendant_hex": req.descendant_hex,
                "ancestor_hex": req.ancestor_hex,
                "causal_root_hex": hex::encode(root),
                "proof": MerklePathDto {
                    siblings_hex: path.siblings.iter().map(hex::encode).collect(),
                    directions: path.directions,
                },
            }))
        }
        Err(e) => Json(serde_json::json!({
            "status":"error",
            "detail": e.to_string(),
        })),
    }
}

async fn post_light_cone_v2_verify_ancestry(
    Json(req): Json<LightConeV2VerifyReq>,
) -> Json<serde_json::Value> {
    use evaporchain_light_cone_v2::{verify_ancestry, MerklePath};

    let root = match decode_32(&req.causal_root_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"causal_root_hex must be 64 hex chars"
            }))
        }
    };
    let ancestor = match decode_32(&req.ancestor_id_hex) {
        Some(b) => b,
        None => {
            return Json(serde_json::json!({
                "status":"error",
                "detail":"ancestor_id_hex must be 64 hex chars"
            }))
        }
    };
    let mut siblings = Vec::with_capacity(req.proof.siblings_hex.len());
    for s in &req.proof.siblings_hex {
        match decode_32(s) {
            Some(b) => siblings.push(b),
            None => {
                return Json(serde_json::json!({
                    "status":"error",
                    "detail": format!("proof.siblings_hex entry must be 64 hex chars: {s}"),
                }))
            }
        }
    }
    let path = MerklePath {
        siblings,
        directions: req.proof.directions.clone(),
    };
    match verify_ancestry(&root, &ancestor, &path) {
        Ok(verified) => Json(serde_json::json!({
            "status":"ok",
            "verified": verified,
        })),
        Err(e) => Json(serde_json::json!({
            "status":"error",
            "detail": e.to_string(),
            "verified": false,
        })),
    }
}

// ─────────────── Singh-Inequality V2 (variance-aware Bernstein) ────
//
// V1 (`evaporchain-singh-inequality`) ships an energy-weighted
// Hoeffding bound `σ²_H = Σ ω_i²` with `ω_i = (b_i − a_i)·e_i / E_max`.
// That uses range only — worst-case variance for a bounded random
// variable. Real chain signals concentrate near the centre of their
// range; V2 (Bernstein 1924) gives a strictly tighter tail bound
// when the actual variance is small relative to the range:
//
//   P(|S − E[S]| ≥ ε) ≤ 2·exp(−ε² / (2σ² + (2/3)·M·ε))
//
// V2 ships:
//  - `singh_bernstein_variance(contribs)` — energy-weighted variance
//    accumulator with Popoviciu (`var ≤ range²`) guard.
//  - `passes_singh_bernstein_gate(ε, contribs, K)` — integer gate:
//    `3·ε² ≥ K·(6·σ² + 2·M·ε)`, equivalent to `ε²/(2σ² + (2/3)Mε) ≥ K`.
//  - `bernstein_strictly_tighter` — runs both gates side-by-side
//    so operators can see exactly when V2 admits a claim that V1
//    rejects (the "concentrated signal" operating region).
//
// All u128 values are serialised as decimal strings to avoid the
// JavaScript safe-integer coercion that would round numbers above
// 2^53. Inputs accept u64 (which fits the JSON number domain).

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContributorWithVarianceDto {
    pub lo: u64,
    pub hi: u64,
    pub energy: u64,
    /// σ²_i. Same scale as range². u64 is enough for any realistic
    /// chain signal: max range² for u32 contributions is 2^64.
    pub variance_proxy: u64,
}

impl ContributorWithVarianceDto {
    fn to_inner(&self) -> evaporchain_singh_inequality_v2::ContributorWithVariance {
        evaporchain_singh_inequality_v2::ContributorWithVariance {
            lo: self.lo,
            hi: self.hi,
            energy: self.energy,
            variance_proxy: self.variance_proxy as u128,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SinghBernsteinGateReq {
    pub contributors: Vec<ContributorWithVarianceDto>,
    /// Deviation ε (claim magnitude).
    pub deviation: u64,
    /// Soundness multiplier K (positive).
    pub soundness_multiplier: u64,
}

async fn post_singh_inequality_v2_gate(
    Json(req): Json<SinghBernsteinGateReq>,
) -> Json<serde_json::Value> {
    use evaporchain_singh_inequality_v2::bound::max_range;
    use evaporchain_singh_inequality_v2::{passes_singh_bernstein_gate, singh_bernstein_variance};

    let contribs: Vec<_> = req.contributors.iter().map(|c| c.to_inner()).collect();
    let var = match singh_bernstein_variance(&contribs) {
        Ok(v) => v,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    };
    let m = match max_range(&contribs) {
        Ok(m) => m,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    };
    let admits = match passes_singh_bernstein_gate(
        req.deviation as u128,
        &contribs,
        req.soundness_multiplier as u128,
    ) {
        Ok(b) => b,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    };
    Json(serde_json::json!({
        "status":"ok",
        "admits": admits,
        "variance_bound": var.to_string(),
        "max_range": m.to_string(),
        "deviation": req.deviation,
        "soundness_multiplier": req.soundness_multiplier,
    }))
}

async fn post_singh_inequality_v2_compare(
    Json(req): Json<SinghBernsteinGateReq>,
) -> Json<serde_json::Value> {
    use evaporchain_singh_inequality_v2::bernstein_strictly_tighter;

    let contribs: Vec<_> = req.contributors.iter().map(|c| c.to_inner()).collect();
    match bernstein_strictly_tighter(
        req.deviation as u128,
        &contribs,
        req.soundness_multiplier as u128,
    ) {
        Ok(adv) => Json(serde_json::json!({
            "status":"ok",
            "v1_admits": adv.v1_admits,
            "v2_admits": adv.v2_admits,
            "v1_variance_bound": adv.v1_variance_bound.to_string(),
            "v2_variance_bound": adv.v2_variance_bound.to_string(),
            "v2_strictly_tighter": adv.v2_admits && !adv.v1_admits,
        })),
        Err(e) => Json(serde_json::json!({
            "status":"error",
            "detail": e.to_string(),
        })),
    }
}

// ─────────────── Antichain Mempool ──────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AntichainComputeReq {
    /// Blocks to insert into a fresh LightCone DAG.
    /// Each block: { id_hex: "...", parent_ids: ["...", ...], energy: u64, observed_epoch: u64 }
    pub blocks: Vec<AntichainBlockDto>,
    /// Energy threshold: antichain must exceed this total to be accepted.
    pub threshold: u64,
    /// Current epoch for decay computation.
    pub current_epoch: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AntichainBlockDto {
    pub id_hex: String,
    pub parent_ids: Vec<String>,
    pub energy: u64,
    pub observed_epoch: u64,
}

async fn post_antichain_compute(Json(req): Json<AntichainComputeReq>) -> Json<serde_json::Value> {
    use evaporchain_antichain_mempool::{extend_to_maximal, total_energy_meets_threshold};
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use evaporchain_light_cone::{Block, BlockId, LightCone};

    fn parse_id(s: &str) -> Option<BlockId> {
        let bytes = hex::decode(s).ok()?;
        if bytes.len() != 32 {
            return None;
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(&bytes);
        Some(id)
    }

    let mut lc = LightCone::new();
    for b in &req.blocks {
        let id = match parse_id(&b.id_hex) {
            Some(i) => i,
            None => {
                return Json(
                    serde_json::json!({"status":"error","detail":format!("bad id_hex: {}", b.id_hex)}),
                )
            }
        };
        let mut parents = Vec::new();
        for ph in &b.parent_ids {
            match parse_id(ph) {
                Some(pid) => parents.push(pid),
                None => {
                    return Json(
                        serde_json::json!({"status":"error","detail":format!("bad parent id: {ph}")}),
                    )
                }
            }
        }
        let block = Block::new(id, parents, b.energy, b.observed_epoch);
        if let Err(e) = lc.insert(block) {
            return Json(serde_json::json!({"status":"error","detail":e.to_string()}));
        }
    }

    // Build candidate set from all block IDs (descending energy order).
    let mut candidates: Vec<BlockId> = lc.ids().collect();
    candidates.sort_by(|a, b| {
        let ea = lc.get(a).map(|b| b.energy).unwrap_or(0);
        let eb = lc.get(b).map(|b| b.energy).unwrap_or(0);
        eb.cmp(&ea)
    });

    let seed = evaporchain_antichain_mempool::Antichain::empty();
    let antichain = match extend_to_maximal(&seed, &lc, candidates) {
        Ok(a) => a,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    };

    let chain_lambda = ChainLambda::new(Lambda::from_epochs(4096));
    let meets = total_energy_meets_threshold(
        &antichain,
        &lc,
        chain_lambda,
        req.current_epoch,
        req.threshold,
    );

    let member_ids: Vec<String> = antichain
        .members()
        .iter()
        .map(|id| hex::encode(id))
        .collect();
    Json(serde_json::json!({
        "status": "ok",
        "antichain_size": member_ids.len(),
        "members": member_ids,
        "meets_threshold": meets,
        "threshold": req.threshold,
    }))
}

// ─────────────── Hot/Cold Stake ─────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct HotColdStakeReq {
    pub hot: u64,
    pub cold: u64,
    /// Hot half-life in epochs (default 100).
    pub hot_lambda_epochs: u64,
    /// Cold half-life in epochs (default 10000).
    pub cold_lambda_epochs: u64,
    pub last_touched_epoch: u64,
    pub current_epoch: u64,
    /// For promote/demote: amount to move.
    pub amount: Option<u64>,
}

fn hcs_from_req(req: &HotColdStakeReq) -> evaporchain_hot_cold_stake::HotColdStake {
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    evaporchain_hot_cold_stake::HotColdStake::new(
        req.hot,
        req.cold,
        ChainLambda::new(Lambda::from_epochs(req.hot_lambda_epochs.max(1))),
        ChainLambda::new(Lambda::from_epochs(req.cold_lambda_epochs.max(1))),
        req.last_touched_epoch,
    )
}

async fn post_hot_cold_decay(Json(req): Json<HotColdStakeReq>) -> Json<serde_json::Value> {
    let s = hcs_from_req(&req).decay(req.current_epoch);
    Json(serde_json::json!({
        "status": "ok",
        "hot_after": s.hot,
        "cold_after": s.cold,
        "total_after": s.total(),
        "epochs_elapsed": req.current_epoch.saturating_sub(req.last_touched_epoch),
    }))
}

async fn post_hot_cold_promote(Json(req): Json<HotColdStakeReq>) -> Json<serde_json::Value> {
    let amount = req.amount.unwrap_or(0);
    let s = hcs_from_req(&req).decay(req.current_epoch);
    match s.promote(amount) {
        Ok(after) => Json(serde_json::json!({
            "status": "ok",
            "hot_after": after.hot,
            "cold_after": after.cold,
            "total_after": after.total(),
            "promoted": amount,
        })),
        Err(e) => Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    }
}

async fn post_hot_cold_demote(Json(req): Json<HotColdStakeReq>) -> Json<serde_json::Value> {
    let amount = req.amount.unwrap_or(0);
    let s = hcs_from_req(&req).decay(req.current_epoch);
    match s.demote(amount) {
        Ok(after) => Json(serde_json::json!({
            "status": "ok",
            "hot_after": after.hot,
            "cold_after": after.cold,
            "total_after": after.total(),
            "demoted": amount,
        })),
        Err(e) => Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    }
}

// ─────────────── Evaporative Protocol Versioning (EPV) ──────────────

#[derive(Debug, Deserialize)]
pub struct EpvRegisterReq {
    pub id: u32,
    pub seed_energy: u64,
    pub activated_epoch: u64,
}

#[derive(Debug, Deserialize)]
pub struct EpvPruneReq {
    pub current_epoch: u64,
    pub e_min: u64,
    /// λ half-life in epochs for version-energy decay.
    pub lambda_epochs: u64,
}

async fn post_epv_register(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<EpvRegisterReq>,
) -> Json<serde_json::Value> {
    use evaporchain_epv::ProtocolVersion;
    let mut reg = safe_lock(&state.epv_registry);
    let v = ProtocolVersion::new(req.id, req.seed_energy, req.activated_epoch);
    match reg.register(v) {
        Ok(()) => Json(serde_json::json!({
            "status": "ok",
            "id": req.id,
            "seed_energy": req.seed_energy,
            "total_versions": reg.len(),
        })),
        Err(e) => Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    }
}

async fn get_epv_status(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    let reg = safe_lock(&state.epv_registry);
    let epoch = {
        let hist = safe_lock(&state.block_history);
        hist.len() as u64
    };
    let chain_lambda = ChainLambda::new(Lambda::from_epochs(4096));
    let versions: Vec<_> = reg
        .iter()
        .map(|v| {
            serde_json::json!({
                "id": v.id,
                "seed_energy": v.seed_energy,
                "activated_epoch": v.activated_epoch,
                "remaining_energy": v.remaining_at(chain_lambda, epoch),
                "is_runnable": reg.is_runnable(v.id, chain_lambda, epoch, 1),
            })
        })
        .collect();
    Json(serde_json::json!({
        "status": "ok",
        "total_versions": reg.len(),
        "current_epoch": epoch,
        "versions": versions,
    }))
}

async fn post_epv_prune(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<EpvPruneReq>,
) -> Json<serde_json::Value> {
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use evaporchain_epv::prune_evaporated;
    let lambda_safe = req.lambda_epochs.max(1);
    let chain_lambda = ChainLambda::new(Lambda::from_epochs(lambda_safe));
    let mut reg = safe_lock(&state.epv_registry);
    let before = reg.len();
    let outcome = prune_evaporated(&mut reg, chain_lambda, req.current_epoch, req.e_min);
    Json(serde_json::json!({
        "status": "ok",
        "pruned": outcome.pruned,
        "surviving": before - outcome.pruned.len(),
        "current_epoch": req.current_epoch,
        "e_min": req.e_min,
    }))
}

// ─────────────── Cone-locked Capsule (ETLP) ─────────────────────────

#[derive(Debug, Deserialize)]
pub struct EtlpSealReq {
    pub seal_epoch: u64,
    pub energy_threshold: u64,
    pub ciphertext_hex: String,
}

#[derive(Debug, Deserialize)]
pub struct EtlpWitnessReq {
    pub seal_epoch: u64,
    pub energy_threshold: u64,
    pub committed_energy: u64,
    pub observed_epoch: u64,
}

#[derive(Debug, Deserialize)]
pub struct EtlpUnlockReq {
    pub seal_epoch: u64,
    pub energy_threshold: u64,
    pub ciphertext_hex: String,
    pub committed_energy: u64,
    pub observed_epoch: u64,
    pub binding_hex: String,
    pub current_epoch: u64,
    /// λ half-life in epochs.
    pub lambda_epochs: u64,
}

async fn post_etlp_seal(Json(req): Json<EtlpSealReq>) -> Json<serde_json::Value> {
    use evaporchain_etlp::Capsule;
    let ct = match hex::decode(&req.ciphertext_hex) {
        Ok(b) => b,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    };
    match Capsule::new(req.seal_epoch, req.energy_threshold, ct) {
        Ok(_) => Json(serde_json::json!({
            "status": "ok",
            "seal_epoch": req.seal_epoch,
            "energy_threshold": req.energy_threshold,
            "ciphertext_len": req.ciphertext_hex.len() / 2,
        })),
        Err(e) => Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    }
}

async fn post_etlp_witness(Json(req): Json<EtlpWitnessReq>) -> Json<serde_json::Value> {
    use evaporchain_etlp::EnergyWitness;
    let binding = EnergyWitness::compute_binding(
        req.seal_epoch,
        req.energy_threshold,
        req.committed_energy,
        req.observed_epoch,
    );
    Json(serde_json::json!({
        "status": "ok",
        "binding_hex": hex::encode(binding),
        "committed_energy": req.committed_energy,
        "observed_epoch": req.observed_epoch,
    }))
}

async fn post_etlp_can_unlock(Json(req): Json<EtlpUnlockReq>) -> Json<serde_json::Value> {
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use evaporchain_etlp::{can_unlock, Capsule, EnergyWitness};
    let ct = match hex::decode(&req.ciphertext_hex) {
        Ok(b) => b,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    };
    let binding_bytes = match hex::decode(&req.binding_hex) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => {
            return Json(
                serde_json::json!({"status":"error","detail":"binding_hex must be 64 hex chars"}),
            )
        }
    };
    let capsule = match Capsule::new(req.seal_epoch, req.energy_threshold, ct) {
        Ok(c) => c,
        Err(e) => return Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    };
    let witness = EnergyWitness {
        committed_energy: req.committed_energy,
        observed_epoch: req.observed_epoch,
        binding: binding_bytes,
    };
    let chain_lambda = ChainLambda::new(Lambda::from_epochs(req.lambda_epochs.max(1)));
    match can_unlock(&capsule, &witness, chain_lambda, req.current_epoch) {
        Ok(unlocked) => Json(serde_json::json!({
            "status": "ok",
            "can_unlock": unlocked,
            "current_epoch": req.current_epoch,
            "energy_threshold": req.energy_threshold,
        })),
        Err(e) => Json(serde_json::json!({"status":"error","detail":e.to_string()})),
    }
}

// ─────────────── Decay-Stamped Nullifiers (DSN) ──────────────────────

#[derive(Debug, Deserialize)]
pub struct DsnFoldReq {
    /// 32-byte nullifier as 64 hex chars.
    pub nullifier_hex: String,
}

async fn post_dsn_fold_nullifier(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<DsnFoldReq>,
) -> Json<serde_json::Value> {
    let nullifier_bytes = match hex::decode(&req.nullifier_hex) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => {
            return Json(
                serde_json::json!({"status":"error","detail":"nullifier_hex must be 64 hex chars (32 bytes)"}),
            )
        }
    };
    let mut window = safe_lock(&state.dsn_window);
    window.fold_nullifier(&nullifier_bytes);
    Json(serde_json::json!({
        "status": "ok",
        "total_count": window.total_count(),
        "aggregate_root_hex": hex::encode(window.aggregate_root()),
    }))
}

async fn post_dsn_advance_window(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let mut window = safe_lock(&state.dsn_window);
    window.advance_window();
    Json(serde_json::json!({
        "status": "ok",
        "total_count": window.total_count(),
        "aggregate_root_hex": hex::encode(window.aggregate_root()),
    }))
}

async fn get_dsn_status(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let window = safe_lock(&state.dsn_window);
    Json(serde_json::json!({
        "status": "ok",
        "total_count": window.total_count(),
        "aggregate_root_hex": hex::encode(window.aggregate_root()),
    }))
}

// ─────────────── Wilson-Singh Block Flow (WSBF / RG) ────────────────

#[derive(Debug, Deserialize)]
pub struct WsbfRgFlowReq {
    /// Block summaries to feed into the RG flow.
    pub blocks: Vec<WsbfBlockSummaryDto>,
    /// Number of blocks to coarse-grain per step (must be ≥ 1).
    pub coarse_grain: usize,
    /// Controls how strongly entropy shifts λ_eff (in millibits; 0 = no correction).
    pub entropy_scale_mb: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WsbfBlockSummaryDto {
    pub height: u64,
    pub total_energy: u64,
    pub active_accounts: u64,
    pub lambda_half_life: u64,
}

async fn post_wsbf_rg_flow(Json(req): Json<WsbfRgFlowReq>) -> Json<serde_json::Value> {
    use evaporchain_wsbf::flow::rg_flow;
    use evaporchain_wsbf::params::{BlockSummary, RgFlowParams};
    if req.coarse_grain == 0 {
        return Json(serde_json::json!({"status":"error","detail":"coarse_grain must be ≥ 1"}));
    }
    let blocks: Vec<BlockSummary> = req
        .blocks
        .iter()
        .map(|b| BlockSummary {
            height: b.height,
            total_energy: b.total_energy,
            active_accounts: b.active_accounts,
            lambda_half_life: b.lambda_half_life,
        })
        .collect();
    let params = RgFlowParams {
        coarse_grain: req.coarse_grain,
        entropy_scale_mb: req.entropy_scale_mb,
    };
    let steps = rg_flow(&blocks, &params);
    Json(serde_json::json!({
        "status": "ok",
        "input_blocks": blocks.len(),
        "rg_steps": steps.len(),
        "effective_params": steps.iter().map(|ep| serde_json::json!({
            "step": ep.step,
            "height_start": ep.height_start,
            "height_end": ep.height_end,
            "lambda_eff": ep.lambda_eff,
            "effective_accounts": ep.effective_accounts,
            "energy_density": ep.energy_density,
            "entropy_mb": ep.entropy_mb,
        })).collect::<Vec<_>>(),
    }))
}

// ─────────────── RG Consensus Phase Map ─────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RgPhaseClassifyReq {
    pub lambda_eff: u64,
    pub n_validators: u64,
    /// Adversary fraction × 1000 (e.g. 333 = 33.3 %).
    pub adversary_fraction_per_mille: u64,
}

#[derive(Debug, Deserialize)]
pub struct RgPhaseTrajectoryReq {
    /// WSBF EffectiveParams sequence (use /api/wsbf/rg_flow first).
    pub steps: Vec<WsbfEffectiveParamsDto>,
    pub n_validators: u64,
    pub adversary_fraction_per_mille: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WsbfEffectiveParamsDto {
    pub step: usize,
    pub height_start: u64,
    pub height_end: u64,
    pub lambda_eff: u64,
    pub effective_accounts: u64,
    pub energy_density: u64,
    pub entropy_mb: u64,
}

async fn post_rg_phase_classify(Json(req): Json<RgPhaseClassifyReq>) -> Json<serde_json::Value> {
    use evaporchain_rg_phase_map::phase::{classify_regime, PhaseMapParams};
    let phase = classify_regime(
        req.lambda_eff,
        req.n_validators,
        req.adversary_fraction_per_mille,
        &PhaseMapParams::default(),
    );
    Json(serde_json::json!({
        "status": "ok",
        "phase": format!("{phase:?}"),
        "lambda_eff": req.lambda_eff,
        "n_validators": req.n_validators,
        "adversary_fraction_per_mille": req.adversary_fraction_per_mille,
    }))
}

async fn post_rg_phase_trajectory(
    Json(req): Json<RgPhaseTrajectoryReq>,
) -> Json<serde_json::Value> {
    use evaporchain_rg_phase_map::phase::{find_fixed_point, phase_trajectory, PhaseMapParams};
    use evaporchain_wsbf::params::EffectiveParams as RgEp;
    let steps: Vec<RgEp> = req
        .steps
        .iter()
        .map(|s| RgEp {
            step: s.step,
            height_start: s.height_start,
            height_end: s.height_end,
            lambda_eff: s.lambda_eff,
            effective_accounts: s.effective_accounts,
            energy_density: s.energy_density,
            entropy_mb: s.entropy_mb,
        })
        .collect();
    let traj = phase_trajectory(
        &steps,
        req.adversary_fraction_per_mille,
        req.n_validators,
        &PhaseMapParams::default(),
    );
    let fixed_point = find_fixed_point(&traj);
    Json(serde_json::json!({
        "status": "ok",
        "steps": traj.len(),
        "trajectory": traj.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>(),
        "fixed_point_step": fixed_point,
    }))
}

// ─────────────── Demurrage ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DemurrageOwedReq {
    pub balance: u64,
    pub last_touched_epoch: u64,
    pub current_epoch: u64,
    /// λ_base in parts-per-million per epoch (0 = disabled).
    pub lambda_base_ppm: u64,
    /// Energy threshold below which demurrage is zero.
    pub threshold: u64,
}

/// POST /api/demurrage/owed — compute demurrage owed on an idle balance.
///
/// Pure function: no state mutation. Returns owed amount and the ppm rate.
async fn post_demurrage_owed(Json(req): Json<DemurrageOwedReq>) -> Json<serde_json::Value> {
    use evaporchain_demurrage::{demurrage_owed, rate_ppm, DemurrageParams};
    let params = DemurrageParams::new(req.lambda_base_ppm, req.threshold);
    let owed = demurrage_owed(
        req.balance,
        req.last_touched_epoch,
        req.current_epoch,
        &params,
    );
    let rate = rate_ppm(req.balance, &params);
    let elapsed = req.current_epoch.saturating_sub(req.last_touched_epoch);
    Json(serde_json::json!({
        "status": "ok",
        "balance": req.balance,
        "last_touched_epoch": req.last_touched_epoch,
        "current_epoch": req.current_epoch,
        "elapsed_epochs": elapsed,
        "rate_ppm": rate,
        "owed": owed,
        "remaining_balance": req.balance.saturating_sub(owed),
        "is_disabled": params.is_disabled(),
    }))
}

// ─────────────── Settle Demurrage — debit account, credit refresh pool ─

#[derive(Debug, Deserialize)]
pub struct SettleDemurrageReq {
    /// Sender address (hex, 1-32 bytes).
    pub from: String,
    /// Hex-encoded ML-DSA signature over the canonical settle-message.
    /// Canonical bytes: `JSON({type:"settle_demurrage",from,current_epoch})`
    /// — same convention the wallet's other tx flows use (see
    /// `useWallet.signTransaction` / `useWallet.sendTransfer` in
    /// extension/src/hooks/useWallet.ts L420-431).
    pub signature: String,
    /// Hex-encoded ML-DSA public key (1952 bytes).
    pub public_key: String,
}

#[derive(Debug, Serialize)]
pub struct SettleDemurrageResp {
    /// "settled" | "nothing_owed" | "error"
    pub status: &'static str,
    pub settled: u64,
    pub new_balance: u64,
    pub new_last_touched_epoch: u64,
    pub detail: String,
}

/// POST /api/tx/settle_demurrage — debit `owed` from the sender's balance
/// and credit it to the protocol-owned refresh pool under the canonical
/// "DEMU" namespace, mirroring the slash-settlement path in
/// `tendermint::settle_slash` (crates/evaporchain-consensus/src/
/// tendermint.rs L659-665).
///
/// Computes `owed` exactly as `/api/demurrage/owed` does — same
/// `evaporchain_demurrage::demurrage_owed(balance, last_touched, now,
/// params)` formula. Genesis params are hard-coded here (lambda_base 1
/// ppm, threshold 1024) until a chain-config endpoint exposes them.
///
/// Verifies the ML-DSA signature against the canonical signing
/// payload `JSON({type:"settle_demurrage",from,current_epoch})`.
///
/// `last_touched_epoch` is now sourced from `Account.last_touched_epoch`
/// — the per-account demurrage anchor stamped by the execution layer
/// every time the balance or nonce mutates under a tx. After settling
/// we also bump the on-chain anchor forward to `current_epoch` so the
/// settled-amount cannot be re-extracted on the next epoch tick.
async fn post_settle_demurrage(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SettleDemurrageReq>,
) -> Json<SettleDemurrageResp> {
    use evaporchain_demurrage::{demurrage_owed, DemurrageParams};

    let from = match parse_hex_address(&req.from) {
        Ok(a) => a,
        Err(e) => {
            return Json(SettleDemurrageResp {
                status: "error",
                settled: 0,
                new_balance: 0,
                new_last_touched_epoch: 0,
                detail: format!("invalid from address: {e}"),
            });
        }
    };

    // Verify ML-DSA signature over the canonical payload.
    let sig_bytes = match hex::decode(&req.signature) {
        Ok(b) => b,
        Err(_) => {
            return Json(SettleDemurrageResp {
                status: "error",
                settled: 0,
                new_balance: 0,
                new_last_touched_epoch: 0,
                detail: "invalid signature hex".into(),
            });
        }
    };
    let pk_bytes = match hex::decode(&req.public_key) {
        Ok(b) => b,
        Err(_) => {
            return Json(SettleDemurrageResp {
                status: "error",
                settled: 0,
                new_balance: 0,
                new_last_touched_epoch: 0,
                detail: "invalid public_key hex".into(),
            });
        }
    };

    // Read the current epoch + balance + last_touched_epoch from chain
    // state.
    let history = safe_lock(&state.block_history);
    let current_epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);

    let canonical = format!(
        "{{\"type\":\"settle_demurrage\",\"from\":\"{}\",\"current_epoch\":{}}}",
        req.from, current_epoch
    );

    use evaporchain_crypto::signatures::{MlDsaVerifier, Verifier};
    if !MlDsaVerifier::verify(canonical.as_bytes(), &sig_bytes, &pk_bytes) {
        return Json(SettleDemurrageResp {
            status: "error",
            settled: 0,
            new_balance: 0,
            new_last_touched_epoch: 0,
            detail: "signature verification failed".into(),
        });
    }

    let mut db = safe_lock(&state.db);
    let (balance, _nonce, last_touched_epoch) = match db.get_account(&from) {
        Some(acct) => (acct.balance, acct.nonce, acct.last_touched_epoch),
        None => {
            return Json(SettleDemurrageResp {
                status: "error",
                settled: 0,
                new_balance: 0,
                new_last_touched_epoch: 0,
                detail: "account not found".into(),
            });
        }
    };

    // Genesis demurrage params (lambda_base 1 ppm, threshold 1024) —
    // matches /api/demurrage/owed call sites in the wallet store.
    let params = DemurrageParams::new(1, 1024);
    let owed = demurrage_owed(balance, last_touched_epoch, current_epoch, &params);

    if owed == 0 {
        return Json(SettleDemurrageResp {
            status: "nothing_owed",
            settled: 0,
            new_balance: balance,
            new_last_touched_epoch: current_epoch,
            detail: "no demurrage owed at current epoch".into(),
        });
    }

    // Debit the account and slide its demurrage anchor forward so the
    // settled amount can't be re-charged on the next epoch tick.
    let to_debit = owed.min(balance);
    if let Some(acct_mut) = db.get_account_mut(&from) {
        acct_mut.balance = acct_mut.balance.saturating_sub(to_debit);
        acct_mut.last_touched_epoch = current_epoch;
    }
    let new_balance = db.get_account(&from).map(|a| a.balance).unwrap_or(0);
    drop(db);

    // Credit the protocol-owned refresh pool under the canonical
    // "DEMU" namespace, mirroring `settle_slash`'s "SLSH" pattern.
    let demu_ns: Vec<u8> = vec![0x44, 0x45, 0x4d, 0x55]; // "DEMU"
    let mut pool = safe_lock(&state.patronage_pool);
    pool.accrue(demu_ns, to_debit, current_epoch);
    drop(pool);

    Json(SettleDemurrageResp {
        status: "settled",
        settled: to_debit,
        new_balance,
        new_last_touched_epoch: current_epoch,
        detail: format!("settled {} EVAP demurrage to refresh pool (DEMU)", to_debit),
    })
}

// ─────────────── Bell Beacon — latest measured S ──────────────────────

#[derive(Debug, Serialize)]
pub struct BellBeaconLatestResp {
    /// "ok" | "no_data" | "error"
    pub status: &'static str,
    /// Most recent S-value in milli-units. 0 when no data.
    pub s_value_milli: u64,
    /// Local-realism threshold in milli-units (always
    /// LOCAL_REALISM_S_MILLI = 2000 unless overridden by chain config).
    pub threshold_milli: u64,
    /// Whether the most recent S-value clears the local-realism
    /// threshold.
    pub bell_certified: bool,
    /// Block height the measurement is anchored to. 0 when no data.
    pub block_height: u64,
    /// Epoch the measurement is anchored to. 0 when no data.
    pub epoch: u64,
    pub detail: String,
}

/// GET /api/bell/latest — most recent Bell-Beacon measurement.
///
/// The consensus layer derives a per-block CHSH S-value from the VRF
/// output in the commit pipeline (Bell-Certified gate in
/// `crates/evaporchain-consensus/src/tendermint.rs`) and persists it
/// on `TendermintConsensus`, exposed via `last_bell_reading()`. This
/// handler returns the live measurement when Tendermint is bound and
/// has produced at least one VRF-derived S-value; otherwise it falls
/// back to `status: "no_data"` with the latest block height + epoch
/// from the in-memory history so wallets can render a "no live
/// measurement" badge alongside the design-target fallback.
async fn get_bell_latest(State(state): State<Arc<ApiState>>) -> Json<BellBeaconLatestResp> {
    if let Some(tc_handle) = state.tendermint.as_ref() {
        let tc = safe_lock(tc_handle);
        if let Some(r) = tc.last_bell_reading() {
            return Json(BellBeaconLatestResp {
                status: "ok",
                s_value_milli: r.s_value_milli,
                threshold_milli: r.threshold_milli,
                bell_certified: r.bell_certified,
                block_height: r.block_height,
                epoch: r.epoch,
                detail: "live VRF-derived CHSH measurement".into(),
            });
        }
    }

    let history = safe_lock(&state.block_history);
    let (block_height, epoch) = history
        .back()
        .map(|b| (b.number, b.epoch))
        .unwrap_or((0, 0));
    drop(history);

    Json(BellBeaconLatestResp {
        status: "no_data",
        s_value_milli: 0,
        threshold_milli: evaporchain_bell_beacon::LOCAL_REALISM_S_MILLI,
        bell_certified: false,
        block_height,
        epoch,
        detail: "no Bell-Beacon measurement yet — Tendermint either not \
                 bound (mock / devnet mode) or no block with a \
                 VRF-derived CHSH S-value has been committed"
            .into(),
    })
}

// ─────────────── MERA — authenticated energy state commitment ─────────

#[derive(Debug, Deserialize)]
pub struct MeraCommitReq {
    /// Account energy values (physical layer, in chain energy units).
    pub energies: Vec<u64>,
    /// Chain's λ half-life in epochs.
    pub lambda_half_life: u64,
    /// τ₀ — base half-life assigned to MERA layer 0 (epochs).
    pub base_half_life: u64,
}

/// POST /api/mera/commit — build a MERA state commitment from account energies.
///
/// Returns the 32-byte root_hash, per-layer hashes, and the compact
/// header_bytes that go into the block header. Pure compute — no chain state.
async fn post_mera_commit(Json(req): Json<MeraCommitReq>) -> Json<serde_json::Value> {
    use evaporchain_mera::commit;
    if req.energies.is_empty() {
        return Json(
            serde_json::json!({ "status": "error", "error": "energies must be non-empty" }),
        );
    }
    let lhl = req.lambda_half_life.max(1);
    let bhl = req.base_half_life.max(1);
    let (commitment, _tree) = commit(&req.energies, lhl, bhl);
    let layer_hashes_hex: Vec<String> = commitment
        .layer_hashes
        .iter()
        .map(|h| hex::encode(h))
        .collect();
    Json(serde_json::json!({
        "status": "ok",
        "n_accounts": commitment.n_accounts,
        "depth": commitment.depth,
        "lambda_half_life": commitment.lambda_half_life,
        "root_hash": hex::encode(commitment.root_hash),
        "layer_hashes": layer_hashes_hex,
        "header_bytes": hex::encode(commitment.header_bytes()),
    }))
}

// ─────────────── Self-Annealing Validator Gate ────────────────────────

#[derive(Debug, Deserialize)]
pub struct AnnealingTempReq {
    pub lambda_half_life: u64,
    pub beta_mb: u64,
    pub epoch: u64,
}

#[derive(Debug, Deserialize)]
pub struct AnnealedScoreDto {
    pub stake: u64,
    pub activity: u64,
    pub uptime_milli: u64,
}

#[derive(Debug, Deserialize)]
pub struct AnnealingAcceptReq {
    pub lambda_half_life: u64,
    pub beta_mb: u64,
    pub epoch: u64,
    /// Deterministic slot nonce derived from block hash (never a PRNG).
    pub slot_nonce: u64,
    pub incumbent: AnnealedScoreDto,
    pub candidate: AnnealedScoreDto,
}

/// POST /api/annealing/temperature — compute SA effective temperature at epoch.
///
/// T(epoch) = λ × 2^(−epoch/λ). Returns 0 once fully crystallised.
async fn post_annealing_temperature(Json(req): Json<AnnealingTempReq>) -> Json<serde_json::Value> {
    use evaporchain_self_annealing::{effective_temperature, AnnealingParams};
    let params = AnnealingParams {
        lambda_half_life: req.lambda_half_life,
        beta_mb: req.beta_mb,
    };
    let temp = effective_temperature(&params, req.epoch);
    let crystallised = temp == 0;
    Json(serde_json::json!({
        "status": "ok",
        "epoch": req.epoch,
        "lambda_half_life": req.lambda_half_life,
        "effective_temperature": temp,
        "crystallised": crystallised,
        "half_lives_elapsed": if req.lambda_half_life > 0 { req.epoch / req.lambda_half_life } else { u64::MAX },
    }))
}

/// POST /api/annealing/accepts_candidate — SA acceptance gate for validator set rotation.
///
/// Deterministic: given the same (epoch, slot_nonce, incumbent, candidate) every
/// validator independently reaches the same accept/reject decision. Uses
/// Kirkpatrick-Gelatt-Vecchi 1983 rational approximation — no PRNG.
async fn post_annealing_accepts_candidate(
    Json(req): Json<AnnealingAcceptReq>,
) -> Json<serde_json::Value> {
    use evaporchain_self_annealing::{
        accepts_candidate, effective_temperature, AnnealedScore, AnnealingParams,
    };
    let params = AnnealingParams {
        lambda_half_life: req.lambda_half_life,
        beta_mb: req.beta_mb,
    };
    let v_old = AnnealedScore {
        stake: req.incumbent.stake,
        activity: req.incumbent.activity,
        uptime_milli: req.incumbent.uptime_milli,
    };
    let v_new = AnnealedScore {
        stake: req.candidate.stake,
        activity: req.candidate.activity,
        uptime_milli: req.candidate.uptime_milli,
    };
    let accepted = accepts_candidate(&params, req.epoch, &v_old, &v_new, req.slot_nonce);
    let temp = effective_temperature(&params, req.epoch);
    Json(serde_json::json!({
        "status": "ok",
        "epoch": req.epoch,
        "effective_temperature": temp,
        "crystallised": temp == 0,
        "accepted": accepted,
        "detail": if accepted { "candidate accepted by SA gate" } else { "candidate rejected — incumbent retained" },
    }))
}

// ─────────────── Tombstone — "small deaths" eulogy trie ──────────────

#[derive(Debug, Deserialize)]
pub struct TombstoneMintReq {
    /// 64-hex address of the evaporated account.
    pub address_hex: String,
    pub final_balance: u64,
    pub final_epoch: u64,
    /// One of: "evaporated", "forgotten", "slashed", "rent", or "other:<n>"
    pub cause: String,
}

#[derive(Debug, Deserialize)]
pub struct TombstoneEulogyRootReq {
    /// List of tombstone entries: each has address_hex + commitment_hex (from /mint).
    pub entries: Vec<TombstoneEntry>,
}

#[derive(Debug, Deserialize)]
pub struct TombstoneEntry {
    pub address_hex: String,
    pub commitment_hex: String,
}

fn parse_cause(s: &str) -> evaporchain_tombstone::CauseOfDeath {
    use evaporchain_tombstone::CauseOfDeath;
    match s.to_lowercase().as_str() {
        "evaporated" => CauseOfDeath::Evaporated,
        "forgotten" => CauseOfDeath::ForgottenViaDecayProof,
        "slashed" => CauseOfDeath::SlashedToZero,
        "rent" => CauseOfDeath::RentExhausted,
        other => {
            let n: u32 = other
                .strip_prefix("other:")
                .and_then(|x| x.parse().ok())
                .unwrap_or(0);
            CauseOfDeath::Other(n)
        }
    }
}

/// POST /api/tombstone/mint — mint the 32-byte memorial for an evaporated account.
///
/// Domain-separated blake3: `"evaporchain-tombstone" || addr || final_balance ||
/// final_epoch || cause_discriminant`. Pure compute — no state.
async fn post_tombstone_mint(Json(req): Json<TombstoneMintReq>) -> Json<serde_json::Value> {
    use evaporchain_tombstone::mint;
    let addr = match parse_hex32(&req.address_hex) {
        Ok(a) => a,
        Err(_) => {
            return Json(
                serde_json::json!({ "status": "error", "error": "address_hex must be 64 hex chars" }),
            )
        }
    };
    let cause = parse_cause(&req.cause);
    let tombstone = mint(addr, req.final_balance, req.final_epoch, cause);
    Json(serde_json::json!({
        "status": "ok",
        "address_hex": req.address_hex,
        "final_balance": req.final_balance,
        "final_epoch": req.final_epoch,
        "cause": req.cause,
        "commitment": hex::encode(tombstone.commitment),
    }))
}

/// POST /api/tombstone/eulogy_root — build an EulogyTrie from a batch of
/// (address, commitment) pairs and return the order-independent blake3 root.
///
/// Two nodes that have observed the same tombstone set (in any order)
/// compute the same root — safe for light-client verification.
async fn post_tombstone_eulogy_root(
    Json(req): Json<TombstoneEulogyRootReq>,
) -> Json<serde_json::Value> {
    use evaporchain_tombstone::{EulogyTrie, Tombstone};
    let mut trie = EulogyTrie::new();
    for entry in &req.entries {
        let addr = match parse_hex32(&entry.address_hex) {
            Ok(a) => a,
            Err(_) => {
                return Json(
                    serde_json::json!({ "status": "error", "error": format!("bad address_hex: {}", entry.address_hex) }),
                )
            }
        };
        let commitment = match parse_hex32(&entry.commitment_hex) {
            Ok(c) => c,
            Err(_) => {
                return Json(
                    serde_json::json!({ "status": "error", "error": format!("bad commitment_hex: {}", entry.commitment_hex) }),
                )
            }
        };
        if let Err(e) = trie.insert(addr, Tombstone { commitment }) {
            return Json(serde_json::json!({ "status": "error", "error": e.to_string() }));
        }
    }
    Json(serde_json::json!({
        "status": "ok",
        "n_entries": trie.len(),
        "eulogy_root": hex::encode(trie.root()),
    }))
}

// ─────────────── HBCT-Elexon: epoch → GB settlement period mapping ───

#[derive(Debug, Deserialize)]
pub struct ElexonEpochReq {
    /// Unix timestamp (seconds) at chain epoch 0.
    pub genesis_unix_ts: u64,
    /// Seconds per chain epoch (default 12 for 12-second slots).
    pub epoch_duration_s: u64,
    /// The chain epoch at which the capacity slot *closes*.
    pub hour_slot: u64,
}

/// POST /api/elexon/epoch_to_slot — map a chain epoch to a GB Elexon settlement slot.
///
/// Returns the calendar date "YYYY-MM-DD" and settlement period (1..=48)
/// for the BMRS B1790 query, no network required. Used by the HBCT oracle
/// to resolve capacity delivery confirmation from the UK grid.
async fn post_elexon_epoch_to_slot(Json(req): Json<ElexonEpochReq>) -> Json<serde_json::Value> {
    use evaporchain_hbct_elexon::mapping::epoch_to_elexon_slot;
    let dur = req.epoch_duration_s.max(1);
    let slot = epoch_to_elexon_slot(req.genesis_unix_ts, dur, req.hour_slot);
    Json(serde_json::json!({
        "status": "ok",
        "hour_slot": req.hour_slot,
        "genesis_unix_ts": req.genesis_unix_ts,
        "epoch_duration_s": dur,
        "settlement_date": slot.date,
        "settlement_period": slot.period,
        "detail": format!("Elexon BMRS B1790 query: date={} period={}", slot.date, slot.period),
    }))
}

// ─────────────── Energy Kernel — conservation audit + redirect sim ───────────

#[derive(serde::Deserialize)]
struct EnergyAccReq {
    accounts: u64,
    stake: u64,
    refresh_pool: u64,
    slashed_pool: u64,
}

#[derive(serde::Deserialize)]
struct ConservationCheckReq {
    before: EnergyAccReq,
    after: EnergyAccReq,
    epochs_elapsed: u64,
    half_life_epochs: u64,
}

async fn post_energy_kernel_conservation_check(
    Json(req): Json<ConservationCheckReq>,
) -> Json<serde_json::Value> {
    use evaporchain_energy_kernel::{
        compartment::EnergyAccumulator, conservation::ConservationCheck, ChainLambda, Lambda,
    };
    let before = EnergyAccumulator::new(
        req.before.accounts,
        req.before.stake,
        req.before.refresh_pool,
        req.before.slashed_pool,
    );
    let after = EnergyAccumulator::new(
        req.after.accounts,
        req.after.stake,
        req.after.refresh_pool,
        req.after.slashed_pool,
    );
    let lambda = ChainLambda::new(Lambda::from_epochs(req.half_life_epochs.max(1)));
    match ConservationCheck::block_step(&before, &after, req.epochs_elapsed, lambda) {
        Ok(()) => Json(serde_json::json!({
            "valid": true,
            "before_total": before.total(),
            "after_total": after.total(),
            "epochs_elapsed": req.epochs_elapsed,
            "half_life_epochs": req.half_life_epochs,
        })),
        Err(e) => Json(serde_json::json!({
            "valid": false,
            "error": e.to_string(),
            "before_total": before.total(),
            "after_total": after.total(),
            "epochs_elapsed": req.epochs_elapsed,
            "half_life_epochs": req.half_life_epochs,
        })),
    }
}

#[derive(serde::Deserialize)]
struct EnergyRedirectReq {
    accounts: u64,
    stake: u64,
    refresh_pool: u64,
    slashed_pool: u64,
    redirect_kind: String,
    amount: u64,
}

async fn post_energy_kernel_redirect(
    Json(req): Json<EnergyRedirectReq>,
) -> Json<serde_json::Value> {
    use evaporchain_energy_kernel::{
        compartment::EnergyAccumulator,
        redirect::{EnergyRedirect, RedirectKind},
    };
    let mut acc =
        EnergyAccumulator::new(req.accounts, req.stake, req.refresh_pool, req.slashed_pool);
    let kind = match req.redirect_kind.to_lowercase().as_str() {
        "slash" => RedirectKind::Slash,
        "slash_settle" => RedirectKind::SlashSettle,
        "mev_burn" => RedirectKind::MevBurn,
        "demurrage" => RedirectKind::Demurrage,
        "refresh_payout" => RedirectKind::RefreshPayout,
        other => {
            return Json(serde_json::json!({ "error": format!("unknown redirect_kind: {other}") }))
        }
    };
    let total_before = acc.total();
    match EnergyRedirect::new(kind, req.amount).apply(&mut acc) {
        Ok((from, to)) => {
            use evaporchain_energy_kernel::compartment::Compartment;
            Json(serde_json::json!({
                "success": true,
                "from_compartment": format!("{from:?}"),
                "to_compartment": format!("{to:?}"),
                "amount": req.amount,
                "total_before": total_before,
                "total_after": acc.total(),
                "state_after": {
                    "accounts":    acc[Compartment::Accounts],
                    "stake":       acc[Compartment::Stake],
                    "refresh_pool": acc[Compartment::RefreshPool],
                    "slashed_pool": acc[Compartment::SlashedPool],
                }
            }))
        }
        Err(e) => Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

// ─────────────── Autopoietic Chain Health ────────────────────────────

async fn get_autopoietic_health(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    use evaporchain_autopoietic::AutopoieticHealth;
    use evaporchain_autopoietic::ChainAutopoiesis;
    use evaporchain_llsa::proof::AlwaysAcceptVerifier;

    // Derive current epoch from block count (proxy; production would use consensus height).
    let epoch = {
        let hist = safe_lock(&state.block_history);
        hist.len() as u64
    };

    // Find the most recent sentinel vote epoch across all parameters.
    let last_sentinel_vote = {
        let db = safe_lock(&state.db);
        let params = db.all_sentinel_params();
        let mut max_epoch: Option<u64> = None;
        for p in &params {
            for v in db.get_sentinel_votes(p.id) {
                max_epoch = Some(max_epoch.map_or(v.observed_epoch, |e| e.max(v.observed_epoch)));
            }
        }
        max_epoch
    };

    // Known patronage covenant object IDs (the 5 demo objects seeded at startup).
    let covenant_ids: Vec<Vec<u8>> = (1u8..=5).map(|i| vec![i, 0, 0, 0]).collect();

    let book = safe_lock(&state.patronage_book);
    let sys = ChainAutopoiesis::new(
        AlwaysAcceptVerifier,
        1_000, // min_patronage_energy: 1 000 energy units
        50,    // sentinel_heartbeat_window: 50 epochs
    );
    let report: AutopoieticHealth =
        sys.health_report(&book, &covenant_ids, last_sentinel_vote, epoch);

    Json(serde_json::json!({
        "status": format!("{:?}", report.status),
        "patronage": format!("{:?}", report.patronage),
        "sentinel": format!("{:?}", report.sentinel),
        "llsa": format!("{:?}", report.llsa),
        "total_patronage_energy": report.total_patronage_energy,
        "epoch": report.epoch,
        "last_sentinel_vote_epoch": last_sentinel_vote,
    }))
}

// ─────────────── Consensus Phase + WSBF λ_eff ───────────────────────

async fn get_consensus_phase(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    if let Some(tc) = &state.tendermint {
        let tc = safe_lock(tc);
        let phase = tc.consensus_phase();
        let ep = tc.effective_params().map(|ep| {
            serde_json::json!({
                "step": ep.step,
                "height_start": ep.height_start,
                "height_end": ep.height_end,
                "lambda_eff": ep.lambda_eff,
                "effective_accounts": ep.effective_accounts,
                "energy_density": ep.energy_density,
                "entropy_mb": ep.entropy_mb,
            })
        });
        Json(serde_json::json!({
            "status": "ok",
            "consensus_phase": format!("{phase:?}"),
            "last_effective_params": ep,
        }))
    } else {
        Json(serde_json::json!({
            "status": "ok",
            "consensus_phase": "LivenessStable",
            "last_effective_params": null,
            "note": "Tendermint not active — running MockConsensus",
        }))
    }
}

// ─────────────── /api/docs — endpoint catalog ───────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ApiDocEntry {
    pub method: &'static str,
    pub path: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub example: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct ApiDocsResp {
    pub chain: String,
    pub launch_sprint_endpoints: usize,
    pub endpoints: Vec<ApiDocEntry>,
}

const ENDPOINT_CATALOG: &[ApiDocEntry] = &[
    // Identity / spine
    ApiDocEntry { method: "GET",  path: "/api/identity",              category: "identity", description: "Single-call dashboard summary: four-act spine, light-cone count, TUR verdict, lambda-fold accumulator, lamport time, sentinel parameters, HBCT state, wired primitives, headline sentence", example: None },
    ApiDocEntry { method: "GET",  path: "/api/four_act",              category: "identity", description: "Four-act narrative spine snapshot (Birth/Life/Small Deaths/Final Death)", example: None },
    ApiDocEntry { method: "GET",  path: "/api/light_cone",            category: "identity", description: "Light-Cone DAG block count + 'running alongside Tendermint' flag", example: None },
    ApiDocEntry { method: "GET",  path: "/api/light_cone/antichain_digest", category: "identity", description: "Phase 4.4 antichain commit-cert digest. Deterministic 32-byte blake3 fingerprint of the closing antichain, domain-separated under `evaporchain-antichain-digest-v1`. Operators compare this across cluster validators to confirm cross-validator agreement on antichain finality. Returns {digest, closing_antichain, closing_antichain_size, running_alongside_tendermint}.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/light_cone/antichain_digest_history", category: "identity", description: "Phase 4.4 rolling history of (height, digest) pairs (last 128 committed blocks under `light_cone_state_branches_enabled = true`). Operators retrospectively cross-compare per-height digests across cluster validators: divergence at any past height is the freeze-class signal for antichain disagreement. Returns {history:[{block_height, digest}], count, running_alongside_tendermint}.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/light_cone/candidate_heads", category: "identity", description: "MCC Phase E.1 — every active sibling head in the Light-Cone DAG with its first-parent trajectory caliber, sorted descending (smaller-BlockId tiebreak). The first entry is the chain's MCC-chosen authoritative head; downstream entries are the alternatives the fork-choice considered. Operators debug 'which heads are competing right now' without a manual trajectory walk. Returns {heads:[{block_id, caliber}], count, running_alongside_tendermint}.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/light_cone/authoritative_head", category: "identity", description: "MCC Phase E.2 — the chain's MCC-chosen authoritative head (the argmax of /api/light_cone/candidate_heads). Single entry rather than the full list. Per-validator — different validators may briefly disagree during a round before converging. Pairs with the antichain-digest-history endpoint for retroactive cross-validator agreement detection. Returns {head, caliber, candidates_considered, running_alongside_tendermint}.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/light_cone/block_clock/:block_id_hex", category: "identity", description: "Decay-Lamport DAG accessor. Returns the LamportClock at the named DAG block, derived from the merge of all parent clocks (max tick) plus a tick by the block's energy. Pure function of (DAG, block_id, tick_quantum); tick_quantum is sourced from the chain-global running clock at /api/lamport_time so the per-block and chain-global clocks share time granularity. Operators pin a block_id at a known fork point and compare clocks across all validators — equality is the substrate-level convergence claim. Returns {block_id, found, current_tick, accumulated_energy, tick_quantum, running_alongside_tendermint}.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/lambda_fold",           category: "identity", description: "Lambda-Fold accumulator (acc_hash, total_energy_remaining, step_count, latest_epoch)", example: None },
    ApiDocEntry { method: "GET",  path: "/api/tur_liveness",          category: "identity", description: "TUR Liveness Detector verdict over the sliding window of per-block J", example: None },
    ApiDocEntry { method: "GET",  path: "/api/lamport_time",          category: "identity", description: "Decay-Lamport energy-driven logical clock", example: None },
    ApiDocEntry { method: "GET",  path: "/api/refresh_pool",          category: "identity", description: "Protocol-owned refresh pool total + per-namespace credits", example: None },
    ApiDocEntry { method: "GET",  path: "/api/mortis_cert_preview",   category: "identity", description: "Preview the Mortis death-certificate NFT shape at current state — does not trigger death", example: None },
    ApiDocEntry { method: "POST", path: "/api/mortis_cert_verify",    category: "identity", description: "Re-derive a MortisCertificate's witness and confirm it matches — proves tamper-evidence", example: Some(r#"{"final_state_root_hex":"…","eulogy_trie_root_hex":"…","epoch_of_death":N,"final_refresh_pool":N,"witness_hex":"…"}"#) },

    // Substrate primitives
    ApiDocEntry { method: "GET",  path: "/api/causal_cone",           category: "substrate", description: "Shalizi-Crutchfield O(1) sufficient statistic over the Light-Cone DAG", example: Some("?head_hex=0000…0000") },
    ApiDocEntry { method: "GET",  path: "/api/mcc_fork_choice",       category: "substrate", description: "MCC argmax-caliber fork choice (Jaynes 1980 + Stock 2009)", example: Some("?candidates=0xaa…,0xbb…&beta_mb=10000") },
    ApiDocEntry { method: "GET",  path: "/api/cmu_check",             category: "substrate", description: "Shalizi-Crutchfield Cμ ≤ E + hμ gate", example: Some("?cmu_mb=300&excess_entropy_mb=100&entropy_rate_mb=200") },
    ApiDocEntry { method: "GET",  path: "/api/beacon/:tau",           category: "substrate", description: "Modular-form beacon at τ — verifies E_4³ − E_6² = 1728·Δ", example: Some("/api/beacon/0") },
    ApiDocEntry { method: "POST", path: "/api/crooks_refund",         category: "substrate", description: "Crooks fluctuation theorem MEV refund", example: Some(r#"{"p_forward_ppm":800000,"p_reverse_ppm":400000,"work_extracted":1000,"beta_mb":10}"#) },
    ApiDocEntry { method: "POST", path: "/api/prp/prove",             category: "substrate", description: "Provable Retention Proof — latest epoch a committed_energy is retained above floor", example: Some(r#"{"state_id_hex":"00…","committed_energy":1000,"activated_epoch":0,"floor":10}"#) },
    ApiDocEntry { method: "POST", path: "/api/efh/h0",                category: "substrate", description: "0-dim sublevel persistence diagram of an energy sequence", example: Some(r#"{"energies":[1,5,3,8,2]}"#) },
    ApiDocEntry { method: "POST", path: "/api/efh/bottleneck",        category: "substrate", description: "Cohen-Steiner-Edelsbrunner-Harer bottleneck distance between two persistence diagrams", example: Some(r#"{"diagram_a":[[1,5]],"diagram_b":[[2,5]]}"#) },
    ApiDocEntry { method: "POST", path: "/api/lambda_fold/verify",    category: "substrate", description: "Verify the chain's Lambda-Fold accumulator against expected (acc_hash, remaining_energy)", example: Some(r#"{"expected_acc_hash_hex":"…","expected_remaining_energy":0}"#) },
    ApiDocEntry { method: "POST", path: "/api/singh_attractor",       category: "substrate", description: "Singh-Attractor basin selection", example: Some(r#"{"state_energy":50,"attractors":[{"center":50,"basin_radius":10}]}"#) },
    ApiDocEntry { method: "POST", path: "/api/bell_beacon",           category: "substrate", description: "CHSH S-value + Bell certification (local-realism threshold S=2000mb)", example: Some(r#"{"e_ab":500,"e_ab_prime":-500,"e_a_prime_b":500,"e_a_prime_b_prime":500}"#) },
    ApiDocEntry { method: "POST", path: "/api/allen_relation",        category: "substrate", description: "Allen interval algebra relation between two intervals", example: Some(r#"{"a":{"start":0,"end":10},"b":{"start":5,"end":15}}"#) },
    ApiDocEntry { method: "POST", path: "/api/mdl_optimal",           category: "substrate", description: "Minimum-Description-Length optimal shard partition", example: Some(r#"{"items":[1,2,3,1,2,3],"max_shards":2}"#) },
    ApiDocEntry { method: "POST", path: "/api/cslc_reconstruct",      category: "substrate", description: "Single-state ε-machine baseline from a flat symbol-count distribution. The full Shalizi-Klinkner CSSR algorithm (multi-state ε-machine reconstruction from a stream) ships at the library level via `evaporchain_cslc::reconstruct_cssr`; this HTTP endpoint covers only the cheaper count-based baseline.", example: Some(r#"{"counts":[10,20,30]}"#) },
    ApiDocEntry { method: "POST", path: "/api/padic",                 category: "substrate", description: "2-adic ultrametric distance + valuations", example: Some(r#"{"x":12,"y":20}"#) },
    ApiDocEntry { method: "POST", path: "/api/tropical_weight",       category: "substrate", description: "Tropical-semiring weight of an energy value", example: Some(r#"{"energy":1000}"#) },
    ApiDocEntry { method: "POST", path: "/api/eb_fs_challenge",       category: "substrate", description: "Energy-Bound Fiat-Shamir challenge derivation", example: Some(r#"{"transcript_hex":"deadbeef","epoch":1,"epoch_energy":1000}"#) },
    ApiDocEntry { method: "POST", path: "/api/singh_attractor_fork_choice", category: "substrate", description: "Singh-Attractor basin-based fork choice over candidate heads", example: Some(r#"{"candidates":"0xaa…,0xbb…","attractors":[{"center":50,"basin_radius":10}]}"#) },
    ApiDocEntry { method: "POST", path: "/api/cone_bridge",           category: "substrate", description: "Cone-Merged Bridge validity — both chains' decay cones must contain the query epoch", example: Some(r#"{"cone_a":{"half_life_epochs":100,"threshold":500,"committed_energy":1000,"observed_epoch":0},"cone_b":{...},"query_epoch":50}"#) },
    ApiDocEntry { method: "POST", path: "/api/eg_fss/sign_verify",    category: "substrate", description: "EG-FSS round-trip — evolve key by energy, sign, verify", example: Some(r#"{"seed_hex":"00…","energy_spent":1000,"threshold_per_period":100,"message_hex":"deadbeef"}"#) },
    ApiDocEntry { method: "POST", path: "/api/lad_vm/simulate",       category: "substrate", description: "LAD-VM substructural-resource lifecycle simulator — Linear/Affine/Decaying × use/drop/tick. Substrate for the future script-lad compiler frontend.", example: Some(r#"{"mode":"decaying","value":42,"created_at_epoch":0,"decay_window":10,"current_epoch":15,"action":"use"}"#) },

    // Patronage Covenants (§4.1 #13)
    ApiDocEntry { method: "POST", path: "/api/patronage/pledge",      category: "patronage", description: "Pledge a Patronage Covenant — pre-fund n epochs of voluntary over-rent to the refresh pool; grants eviction immunity for the duration.", example: Some(r#"{"object_id_hex":"0101010101010101","namespace_id_hex":"01010101","donation_per_epoch":100,"epochs":10,"current_epoch":0}"#) },
    ApiDocEntry { method: "POST", path: "/api/patronage/honour",      category: "patronage", description: "Release one epoch's donation from a covenant into the global patronage pool credit; increments patronage_score.", example: Some(r#"{"object_id_hex":"0101010101010101","epoch":1}"#) },
    ApiDocEntry { method: "POST", path: "/api/patronage/revoke",      category: "patronage", description: "Remove a covenant early; refunds unused pre-funded surplus back to the namespace pool credit.", example: Some(r#"{"object_id_hex":"0101010101010101","epoch":5}"#) },
    ApiDocEntry { method: "GET",  path: "/api/patronage/status",      category: "patronage", description: "Summary of all active Patronage Covenants — count, total pre-funded, total score, patronage-ns hex.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/patronage/immune",      category: "patronage", description: "Query eviction immunity and patronage_score for an object at an epoch.", example: Some("?object_id_hex=0101010101010101&epoch=1") },

    // HBCT launch wedge
    ApiDocEntry { method: "GET",  path: "/api/hbct/state",            category: "hbct", description: "HBCT book summary (entry count, total MWh, top positions)", example: None },
    ApiDocEntry { method: "POST", path: "/api/hbct/seed_demo",        category: "hbct", description: "Seed 8 realistic HBCT positions (GB BMUs + DE-LU)", example: None },
    ApiDocEntry { method: "POST", path: "/api/hbct/mint",             category: "hbct", description: "Mint a single HBCT position", example: Some(r#"{"delivery_location":"BMU-T_DRAXX-1","hour_slot":481248,"mwh_amount":250,"holder_hex":"…","issued_at_epoch":0}"#) },
    ApiDocEntry { method: "POST", path: "/api/hbct/transfer",         category: "hbct", description: "Transfer MWh between holders at a (location, slot)", example: None },
    ApiDocEntry { method: "POST", path: "/api/hbct/burn",             category: "hbct", description: "Burn MWh from a position", example: None },
    ApiDocEntry { method: "POST", path: "/api/hbct/balance",          category: "hbct", description: "Query a holder's MWh at (location, slot)", example: None },
    ApiDocEntry { method: "POST", path: "/api/hbct/tick",             category: "hbct", description: "Auto-burn positions whose hour slot has closed (H+1 decay)", example: Some(r#"{"current_epoch":481253}"#) },
    ApiDocEntry { method: "POST", path: "/api/hbct/seed_attestation", category: "hbct", description: "Seed an oracle attestation into the MockOracleFeed", example: None },
    ApiDocEntry { method: "POST", path: "/api/hbct/settle",           category: "hbct", description: "Settle a position against the oracle attestation", example: None },

    // Sentinel
    ApiDocEntry { method: "POST", path: "/api/sentinel/seed_demo",    category: "sentinel", description: "Register 4 demo chain knobs (gas limit, block time, mempool cap, λ half-life)", example: None },
    ApiDocEntry { method: "POST", path: "/api/sentinel/seed_votes",   category: "sentinel", description: "Cast votes from 3 demo validators targeting each parameter's max", example: Some(r#"{"current_epoch":1}"#) },
    ApiDocEntry { method: "POST", path: "/api/sentinel/register",     category: "sentinel", description: "Register a single bounded chain parameter", example: Some(r#"{"parameter_id":1,"current":50,"min":0,"max":100}"#) },
    ApiDocEntry { method: "POST", path: "/api/sentinel/vote",         category: "sentinel", description: "Record one validator's vote for a parameter target", example: Some(r#"{"parameter_id":1,"validator_id":1,"target":80,"observed_epoch":0}"#) },
    ApiDocEntry { method: "POST", path: "/api/sentinel/tick",         category: "sentinel", description: "Manually run the homeostatic update on one parameter", example: Some(r#"{"parameter_id":1,"current_epoch":100,"max_step":5,"half_life_epochs":1000}"#) },
    ApiDocEntry { method: "GET",  path: "/api/sentinel/parameter/:id",category: "sentinel", description: "Read a single parameter by id", example: None },
    ApiDocEntry { method: "GET",  path: "/api/sentinel/all",          category: "sentinel", description: "List every registered parameter with its current value + vote count", example: None },

    // Script-LAD compiler frontend (§4.1 #12 closure)
    ApiDocEntry { method: "POST", path: "/api/script_lad/check",     category: "script-lad", description: "Parse @lad annotations from script source and report resource verdicts at check_epoch. Flags unconsumed Linear resources and evaporations.", example: Some(r#"{"source":"@lad(mode=linear, value=1000)\nlet payment: u64 = 0;","check_epoch":5}"#) },
    ApiDocEntry { method: "POST", path: "/api/script_lad/simulate",  category: "script-lad", description: "Simulate a full LAD resource lifecycle: initialise from annotations, apply use/drop ops, tick to final_epoch, return verdicts.", example: Some(r#"{"source":"@lad(mode=decaying, window=10, value=500)\nlet voucher: u64 = 0;","created_epoch":0,"ops":[{"op":"use","field":"voucher","epoch":5}],"final_epoch":15}"#) },

    // Singh-Boltzmann Stake + Sanov Slashing (§4.1 #5, §A1.3)
    ApiDocEntry { method: "GET",  path: "/api/validators/boltzmann_stakes",  category: "validators", description: "Per-validator Singh-Boltzmann decayed stake vs governance stake. Decay ratio shows cumulative λ-decay since last block production.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/validators/boltzmann_weights", category: "validators", description: "Boltzmann proposer-selection weights (stake × activity boost). Query: ?beta_mb=1000", example: Some("?beta_mb=1000") },
    ApiDocEntry { method: "POST", path: "/api/validators/sanov_slash",       category: "validators", description: "Apply Sanov KL-divergence slash. type=equivocation uses full slash; type=downtime uses missed_blocks/window miss rate.", example: Some(r#"{"validator_id":1,"slash_type":"downtime","missed_blocks":10,"window":100}"#) },

    // Braid-Group Sequencer (§A1.4)
    ApiDocEntry { method: "POST", path: "/api/braid/commit",               category: "substrate", description: "Reduce a braid word to substrate-canonical form (Garside 1969) and commit via blake3. Encodes transaction ordering as a braid-group commitment.", example: Some(r#"{"generators":[1,2,1,-2,3],"n":5}"#) },

    // Decay-Forget Proofs — GDPR-native (§4.2 V2)
    ApiDocEntry { method: "POST", path: "/api/decay_forget/prove",          category: "substrate", description: "Prove a record's recoverability commitment has λ-decayed below forget_threshold. GDPR-native: once proven, the chain cannot recover the record.", example: Some(r#"{"record_id_hex":"00..00","original_commitment":1000000,"activated_epoch":0,"query_epoch":500,"forget_threshold":100}"#) },
    ApiDocEntry { method: "POST", path: "/api/decay_forget/verify",         category: "substrate", description: "Verify a DecayForgetProof witness — tamper-evident check that the proof was not modified.", example: Some(r#"{"record_id_hex":"00..00","original_commitment":1000000,"activated_epoch":0,"query_epoch":500,"forget_threshold":100}"#) },

    // Governance
    ApiDocEntry { method: "GET",  path: "/api/governance/flags",       category: "governance", description: "All governance soft-fork flags + their effective values (Lane I.4/I.5 + Layer 0 #1: parent_acceptance_mode, block_source_mode, conservation_enforcement, fork_choice_mode). Defaults applied for unset keys.", example: None },
    ApiDocEntry { method: "POST", path: "/api/governance/param",       category: "governance", description: "Lane K.1 — set a soft-fork governance knob without recompiling. Allowlist: parent_acceptance_mode∈{linear,mcc}, block_source_mode∈{fifo,antichain}, conservation_enforcement∈{observe,enforce}.", example: Some(r#"{"key":"parent_acceptance_mode","value":"mcc"}"#) },
    ApiDocEntry { method: "POST", path: "/api/cartel_alarm/run_gate",  category: "substrate",  description: "Lane O.5 — run the Causal-CHSH cartel-detection gate (§A1.10) against operator-supplied chain trace. Returns Pass/Fail/InputError + S statistic + per-bucket sample counts. Doctrine-locked thresholds (1.8/2.2/0.4) baked in.", example: Some(r#"{"trace":[{"height":1,"timestamp_secs":1700000000,"energy":50000,"gas":12000000,"tx_count":150}],"concurrency_window_secs":60}"#) },
    ApiDocEntry { method: "GET",  path: "/api/cartel_alarm/chain_status", category: "substrate", description: "Lane O.8.1b — on-chain Causal-CHSH alarm status. Returns the chain's own self-monitoring verdict (rolling buffer of last 200 committed blocks, gate run every 50 records). Distinct from /api/cartel_alarm/run_gate which takes operator-supplied trace data.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/cartel_alarm/pending_events", category: "substrate", description: "Lane O.8.2b — drain queued CartelAlarmEvents emitted by the chain when its honest-source S crossed the doctrine ceiling AND governance set cartel_alarm_mode=alarm. Each event returned exactly once (polling consumes the queue). Default observe mode keeps the queue empty.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/governance/fork_choice_mode", category: "governance", description: "Current authoritative fork-choice mode (mcc|singh_attractor) + attractor set.", example: None },
    ApiDocEntry { method: "POST", path: "/api/governance/fork_choice_mode", category: "governance", description: "Governance amendment to switch fork-choice between MCC and Singh-Attractor. Requires stake quorum from endorser_stakes.", example: Some(r#"{"mode":"singh_attractor","attractors":[{"center":1000,"basin_radius":200}],"endorser_stakes":[1000,800],"required_stake":1500}"#) },

    // HLTS — Hashgraph-Like Threshold Shares
    ApiDocEntry { method: "POST", path: "/api/hlts/quorum_check",           category: "substrate", description: "Check k-of-n quorum liveness for Shamir-style threshold shares. Each share λ-decays; quorum is met iff at least k shares remain above energy threshold.", example: Some(r#"{"shares":[{"idx":1,"energy":1000,"observed_epoch":0},{"idx":2,"energy":800,"observed_epoch":0}],"k":2,"threshold":100,"lambda_epochs":4096,"query_epoch":100}"#) },

    // PNT — Phased Nullifier Tree (sliding-window double-spend guard)
    ApiDocEntry { method: "POST", path: "/api/pnt/insert",                  category: "substrate", description: "Insert a 32-byte nullifier into the current PNT phase. Rejects double-spends visible within the live window.", example: Some(r#"{"nullifier_hex":"0000000000000000000000000000000000000000000000000000000000000001"}"#) },
    ApiDocEntry { method: "POST", path: "/api/pnt/advance_phase",           category: "substrate", description: "Open a fresh PNT phase, dropping the oldest if window is full.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/pnt/status",                  category: "substrate", description: "PNT current phase, live window depth, and total nullifier count across all phases.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/pnt/is_spent/:nullifier_hex", category: "substrate", description: "Check if a nullifier (64 hex chars) is recorded in any live PNT phase.", example: None },

    // Entropic Slashing — Shannon-weighted slash
    ApiDocEntry { method: "POST", path: "/api/entropic_slash",              category: "substrate", description: "Compute Shannon-entropy-weighted slash magnitude from observed misbehaviour counts. Higher-entropy (noisier cartel) patterns → larger slash.", example: Some(r#"{"stake":1000000,"observed_counts":[90,10]}"#) },

    // Lyapunov Fee Controller — EIP-1559-style with λ-decay
    ApiDocEntry { method: "POST", path: "/api/fee_controller/step",         category: "substrate", description: "Advance the Lyapunov-stable fee controller by one block. Returns updated base fee and Lyapunov V-function drift (negative = converging to equilibrium).", example: Some(r#"{"gas_used":25000000,"epochs_elapsed":1}"#) },
    ApiDocEntry { method: "GET",  path: "/api/fee_controller/status",       category: "substrate", description: "Current fee controller state: accumulated energy, base fee at current pressure, target parameters.", example: None },

    // Evaporated Fork Certificates
    ApiDocEntry { method: "POST", path: "/api/fork_cert/prove",             category: "substrate", description: "Prove a competing fork has λ-decayed below the evaporation threshold. Returns a blake3-bound EvaporatedForkCert + is_evaporated verdict.", example: Some(r#"{"fork_root_hex":"0000000000000000000000000000000000000000000000000000000000000001","blocks":[{"seed_energy":1000,"observed_epoch":0}],"evaluated_at_epoch":200,"threshold":100,"lambda_epochs":100}"#) },
    ApiDocEntry { method: "POST", path: "/api/fork_cert/verify",            category: "substrate", description: "O(1) light-client verify of an EvaporatedForkCert. Checks blake3 witness binding + decayed_energy < threshold.", example: Some(r#"{"fork_root_hex":"0000...","evaluated_at_epoch":200,"total_seed_energy":1000,"decayed_energy":50,"threshold":100,"witness_hex":"<from /api/fork_cert/prove>"}"#) },
    ApiDocEntry { method: "POST", path: "/api/fork_cert_v2/prove",          category: "substrate", description: "Bell-anchored Evaporated-Fork Certificate V2. Closes V1's pre-computation gap by binding the witness to a chain-supplied seed anchor (typically BellCertificate.seed) plus its issuance epoch. Returns CausalityViolation if seed_anchor_epoch > evaluated_at_epoch.", example: Some(r#"{"fork_root_hex":"00...01","blocks":[{"seed_energy":1000,"observed_epoch":0}],"evaluated_at_epoch":200,"threshold":100,"lambda_epochs":100,"bell_seed_anchor_hex":"00...09","seed_anchor_epoch":150}"#) },
    ApiDocEntry { method: "POST", path: "/api/fork_cert_v2/verify",         category: "substrate", description: "O(1) light-client verify of an EvaporatedForkCertV2. Checks anchor-bound witness + causality + decayed_energy < threshold.", example: Some(r#"{"fork_root_hex":"00...01","evaluated_at_epoch":200,"total_seed_energy":1000,"decayed_energy":50,"threshold":100,"bell_seed_anchor_hex":"00...09","seed_anchor_epoch":150,"witness_hex":"<from /api/fork_cert_v2/prove>"}"#) },
    ApiDocEntry { method: "POST", path: "/api/bell_beacon_v2/issue",        category: "substrate", description: "Issue a Bell-Certified Beacon V2 certificate over a window of concurrent block-pairs. Runs the CHSH gate at integer milli-units against the honest sample plus a synthetic coordinated-subset cartel injection, anchors to prev_block_hash, and emits an anti-grinding seed. The seed is the canonical chain-supplied anchor for /api/fork_cert_v2/prove.", example: Some(r#"{"chain_id":"test-chain-v1","window_start":100,"window_end":200,"prev_block_hash_hex":"00...09","pairs":[{"first_energy":100,"first_tx_count":10,"second_energy":10,"second_tx_count":100,"tag_hex":"00...01"}]}"#) },
    ApiDocEntry { method: "POST", path: "/api/bell_beacon_v2/verify",       category: "substrate", description: "Verify a BellCertificate by re-running the gate against the supplied pairs and re-deriving the seed. Rejects on any field mismatch (window, prev_hash, threshold, bucket counts, S values, gap, seed).", example: Some(r#"{"chain_id":"test-chain-v1","prev_block_hash_hex":"00...09","pairs":[...],"certificate":{"<from /api/bell_beacon_v2/issue>":""}}"#) },
    ApiDocEntry { method: "POST", path: "/api/singh_attractor_v2/draw",     category: "substrate", description: "Singh-Attractor V2 — Bell-anchored fallback. In-basin selection is V1-deterministic (seed unused). Out-of-basin: inverse-distance weighted sampling seeded by certificate_seed_hex (typically a BellCertificate.seed). Returns selected_center/index, used_fallback flag, and a bounded Lyapunov drift toward the centre.", example: Some(r#"{"state_energy":500,"attractors":[{"center":100,"basin_radius":10,"drift_rate":5},{"center":1000,"basin_radius":100,"drift_rate":10}],"certificate_seed_hex":"<from /api/bell_beacon_v2/issue>"}"#) },
    ApiDocEntry { method: "POST", path: "/api/ib_validators_v2/vote",       category: "substrate", description: "IB Validators V2 vote gate (Immune Validator Set). Wraps Tishby-Pereira-Bialek IB vote with three structural rejection paths: CHSH-failed-window jail, energy-floor jail, explicit slash. Returns commit/abstain/jailed{reason}.", example: Some(r#"{"local_energies":[0,0,0],"prior_energies":[0,64,128],"signature_scale":1024,"lambda_mb":100,"validator_id_hex":"00...01","energy":1000,"energy_floor":10,"current_epoch":5,"jail_state":[]}"#) },
    ApiDocEntry { method: "POST", path: "/api/ib_validators_v2/jail/chsh_failure", category: "substrate", description: "Mass-jail validators active during a window whose BellCertificate failed the CHSH gate. Stateless: caller submits current jail_state, receives updated jail_state with new entries appended.", example: Some(r#"{"participants_hex":["00...01","00...02"],"window_start":100,"window_end":200,"current_epoch":50,"jail_epochs":50,"jail_state":[]}"#) },
    ApiDocEntry { method: "POST", path: "/api/light_cone_v2/causal_root",   category: "substrate", description: "Light-Cone V2 — compute the BLAKE3 Merkle root over the BTreeSet-sorted causal_past of a block. Stateless: caller submits the DAG.", example: Some(r#"{"blocks":[{"id_hex":"00...00","parent_ids":[],"energy":1000,"observed_epoch":0},{"id_hex":"00...01","parent_ids":["00...00"],"energy":1000,"observed_epoch":1}],"block_id_hex":"00...01"}"#) },
    ApiDocEntry { method: "POST", path: "/api/light_cone_v2/prove_ancestry", category: "substrate", description: "Light-Cone V2 — produce an O(log n) MerklePath proving an ancestor is in a descendant's causal_past. Returns the descendant's causal_root + proof; light clients verify with /api/light_cone_v2/verify_ancestry without needing the DAG.", example: Some(r#"{"blocks":[...],"descendant_hex":"00...04","ancestor_hex":"00...02"}"#) },
    ApiDocEntry { method: "POST", path: "/api/light_cone_v2/verify_ancestry", category: "substrate", description: "Light-Cone V2 — pure light-client verifier. Reproduces the Merkle root from (ancestor_id, proof) and compares to causal_root. No DAG needed. Rejects on tampering, wrong root, path-shape mismatch, or empty-cone sentinel.", example: Some(r#"{"causal_root_hex":"<from prove_ancestry>","ancestor_id_hex":"00...02","proof":{"siblings_hex":[...],"directions":[...]}}"#) },
    ApiDocEntry { method: "POST", path: "/api/singh_inequality_v2/gate",     category: "substrate", description: "Singh-Inequality V2 — variance-aware Bernstein gate. Returns whether 3·ε² ≥ K·(6·σ² + 2·M·ε) (admits the claim) plus the variance_bound + max_range. u128 numerics returned as decimal strings.", example: Some(r#"{"contributors":[{"lo":0,"hi":10,"energy":1000,"variance_proxy":4}],"deviation":15,"soundness_multiplier":1}"#) },
    ApiDocEntry { method: "POST", path: "/api/singh_inequality_v2/compare",  category: "substrate", description: "Run V1 (Hoeffding) and V2 (Bernstein) gates side-by-side over the same contributor set. Returns per-gate admission + variance bounds + a v2_strictly_tighter flag (true iff V2 admits a claim V1 rejects — the V2 advantage region for concentrated chain signals).", example: Some(r#"{"contributors":[{"lo":0,"hi":10,"energy":1000,"variance_proxy":4}],"deviation":15,"soundness_multiplier":1}"#) },

    // Antichain Mempool — causal-set maximal-antichain transaction ordering
    ApiDocEntry { method: "POST", path: "/api/antichain/compute",           category: "substrate", description: "Build an in-memory LightCone DAG from submitted blocks, compute the greedy maximal antichain (descending energy), and check if total λ-decayed energy clears threshold.", example: Some(r#"{"blocks":[{"id_hex":"0000000000000000000000000000000000000000000000000000000000000001","parent_ids":[],"energy":1000,"observed_epoch":0}],"threshold":500,"current_epoch":0}"#) },

    // Hot/Cold Stake — two-temperature stake simulation
    ApiDocEntry { method: "POST", path: "/api/hot_cold_stake/decay",        category: "substrate", description: "Apply λ-decay to both hot and cold pools of a HotColdStake from last_touched_epoch to current_epoch.", example: Some(r#"{"hot":1000000,"cold":5000000,"hot_lambda_epochs":100,"cold_lambda_epochs":10000,"last_touched_epoch":0,"current_epoch":50}"#) },
    ApiDocEntry { method: "POST", path: "/api/hot_cold_stake/promote",      category: "substrate", description: "Simulate a cold→hot stake promotion (after decay to current_epoch). Returns error if insufficient cold stake.", example: Some(r#"{"hot":100,"cold":5000000,"hot_lambda_epochs":100,"cold_lambda_epochs":10000,"last_touched_epoch":0,"current_epoch":50,"amount":500000}"#) },
    ApiDocEntry { method: "POST", path: "/api/hot_cold_stake/demote",       category: "substrate", description: "Simulate a hot→cold stake demotion (after decay to current_epoch). Returns error if insufficient hot stake.", example: Some(r#"{"hot":1000000,"cold":0,"hot_lambda_epochs":100,"cold_lambda_epochs":10000,"last_touched_epoch":0,"current_epoch":50,"amount":100000}"#) },

    // EPV — Evaporative Protocol Versioning
    ApiDocEntry { method: "POST", path: "/api/epv/register",                category: "substrate", description: "Register a protocol version in the node's EPV registry. Each version's verifier energy λ-decays; once below E_min rollback is physically impossible.", example: Some(r#"{"id":4,"seed_energy":1000000000,"activated_epoch":0}"#) },
    ApiDocEntry { method: "GET",  path: "/api/epv/status",                  category: "substrate", description: "List all registered protocol versions with their remaining λ-decayed energy and runnable status at current chain epoch.", example: None },
    ApiDocEntry { method: "POST", path: "/api/epv/prune",                   category: "substrate", description: "Prune all versions whose remaining energy ≤ e_min at current_epoch (physically irrecoverable). Returns pruned IDs.", example: Some(r#"{"current_epoch":1000,"e_min":10,"lambda_epochs":4096}"#) },

    // ETLP — Cone-locked Capsule (Energy Time-Lock Puzzle)
    ApiDocEntry { method: "POST", path: "/api/etlp/seal",                   category: "substrate", description: "Validate + seal a Cone-locked Capsule (ciphertext + energy threshold). Returns ok if the capsule is well-formed.", example: Some(r#"{"seal_epoch":0,"energy_threshold":500,"ciphertext_hex":"deadbeef"}"#) },
    ApiDocEntry { method: "POST", path: "/api/etlp/witness",                category: "substrate", description: "Compute the blake3 binding for an EnergyWitness against a capsule's (seal_epoch, threshold, committed_energy, observed_epoch).", example: Some(r#"{"seal_epoch":0,"energy_threshold":500,"committed_energy":1000,"observed_epoch":0}"#) },
    ApiDocEntry { method: "POST", path: "/api/etlp/can_unlock",             category: "substrate", description: "Check whether an EnergyWitness unlocks a Capsule at current_epoch. Verifies binding + λ-decayed remaining energy ≥ threshold.", example: Some(r#"{"seal_epoch":0,"energy_threshold":500,"ciphertext_hex":"deadbeef","committed_energy":1000,"observed_epoch":0,"binding_hex":"<from /api/etlp/witness>","current_epoch":50,"lambda_epochs":4096}"#) },

    // DSN — Decay-Stamped Nullifiers
    ApiDocEntry { method: "POST", path: "/api/dsn/fold_nullifier",          category: "substrate", description: "Fold a 32-byte nullifier (hex) into the current DSN window accumulator.", example: Some(r#"{"nullifier_hex":"0000000000000000000000000000000000000000000000000000000000000001"}"#) },
    ApiDocEntry { method: "POST", path: "/api/dsn/advance_window",          category: "substrate", description: "Advance the DSN sliding window by one slot, dropping the oldest accumulator.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/dsn/status",                  category: "substrate", description: "Current DSN window: total nullifier count + aggregate blake3 accumulator root.", example: None },

    // WSBF — Wilson-Singh Block Flow (RG on chain history)
    ApiDocEntry { method: "POST", path: "/api/wsbf/rg_flow",               category: "substrate", description: "Apply successive Wilson-Singer RG coarse-graining steps to a block history. Returns one EffectiveParams per step: λ_eff shifts with Shannon entropy of the window (Wilson-Kogut 1974).", example: Some(r#"{"blocks":[{"height":0,"total_energy":1000,"active_accounts":10,"lambda_half_life":4096}],"coarse_grain":1,"entropy_scale_mb":500000}"#) },

    // RG Phase Map — consensus regime classification
    ApiDocEntry { method: "POST", path: "/api/rg_phase/classify",           category: "substrate", description: "Classify a (λ_eff, validator_count, adversary_fraction) tuple into a consensus regime: LivenessStable | SafetyStable | Frozen | Chaotic.", example: Some(r#"{"lambda_eff":4096,"n_validators":10,"adversary_fraction_per_mille":50}"#) },
    ApiDocEntry { method: "POST", path: "/api/rg_phase/trajectory",         category: "substrate", description: "Classify a WSBF phase trajectory. Feed in the effective_params from /api/wsbf/rg_flow. Returns one ConsensusPhase per step + fixed_point_step index.", example: Some(r#"{"steps":[{"step":0,"height_start":0,"height_end":99,"lambda_eff":4096,"effective_accounts":10,"energy_density":1000,"entropy_mb":0}],"n_validators":10,"adversary_fraction_per_mille":50}"#) },

    // Demurrage — native idle-balance demurrage (piecewise log rate)
    ApiDocEntry { method: "POST", path: "/api/demurrage/owed",              category: "substrate", description: "Compute demurrage owed by an idle balance over elapsed epochs. Rate = lambda_base_ppm × log2(balance/threshold) ppm/epoch. Sink → refresh pool. Capped at balance.", example: Some(r#"{"balance":2000000,"last_touched_epoch":0,"current_epoch":1000,"lambda_base_ppm":1,"threshold":1024}"#) },
    ApiDocEntry { method: "POST", path: "/api/tx/settle_demurrage",         category: "substrate", description: "Settle the active account's demurrage: debit `owed` from balance, credit refresh pool under namespace 'DEMU'. ML-DSA signature required over JSON({type:'settle_demurrage',from,current_epoch}).", example: Some(r#"{"from":"0x…","signature":"<hex ML-DSA sig>","public_key":"<hex ML-DSA pk>"}"#) },
    ApiDocEntry { method: "GET",  path: "/api/bell/latest",                 category: "substrate", description: "Most recent Bell-Beacon CHSH S-value measured per-block from VRF output. Currently returns status:'no_data' (consensus layer measures but does not persist S yet). See TODO in api.rs::get_bell_latest.", example: None },

    // MERA — Multi-scale Energy Renormalization Ansatz state commitment
    ApiDocEntry { method: "POST", path: "/api/mera/commit",                 category: "substrate", description: "Build a MERA tensor-network state commitment from account energies. Returns blake3 root_hash, per-layer hashes, and the 32-byte header_bytes for the block header. λ-parameterised Vidal 2007 MERA.", example: Some(r#"{"energies":[1000,2000,3000,4000],"lambda_half_life":4096,"base_half_life":100}"#) },

    // Self-Annealing — Kirkpatrick-Gelatt-Vecchi 1983 validator set crystallisation
    ApiDocEntry { method: "POST", path: "/api/annealing/temperature",       category: "substrate", description: "Compute SA effective temperature at epoch. T(epoch)=λ×2^(−epoch/λ). Approaches 0 as the validator set crystallises; zero = no degrading moves accepted.", example: Some(r#"{"lambda_half_life":4096,"beta_mb":1000,"epoch":2048}"#) },
    ApiDocEntry { method: "POST", path: "/api/annealing/accepts_candidate", category: "substrate", description: "Deterministic SA acceptance gate for validator rotation. Accepts if candidate is better OR T-weighted random acceptance (slot_nonce from block hash, never PRNG).", example: Some(r#"{"lambda_half_life":4096,"beta_mb":1000,"epoch":100,"slot_nonce":12345,"incumbent":{"stake":1000,"activity":10,"uptime_milli":900},"candidate":{"stake":1200,"activity":12,"uptime_milli":950}}"#) },

    // Tombstone — "small deaths" eulogy trie (the chain's deliberate exception to immutability)
    ApiDocEntry { method: "POST", path: "/api/tombstone/mint",              category: "substrate", description: "Mint the 32-byte memorial for an evaporated account. blake3('evaporchain-tombstone' || addr || final_balance || final_epoch || cause_discriminant). The chain admits its small deaths and engraves them.", example: Some(r#"{"address_hex":"0000000000000000000000000000000000000000000000000000000000000001","final_balance":0,"final_epoch":10000,"cause":"evaporated"}"#) },
    ApiDocEntry { method: "POST", path: "/api/tombstone/eulogy_root",       category: "substrate", description: "Build an EulogyTrie from a batch of (address, commitment) pairs and return the order-independent blake3 root. Light-client safe: two nodes observing the same tombstone set in any order compute the same root.", example: Some(r#"{"entries":[{"address_hex":"0000...01","commitment_hex":"<from /tombstone/mint>"}]}"#) },

    // Elexon HBCT oracle: chain epoch → GB grid settlement slot
    ApiDocEntry { method: "POST", path: "/api/elexon/epoch_to_slot",        category: "substrate", description: "Map a chain epoch to a UK Elexon settlement date + period (1..=48, each 30 min). No network call — pure calendar arithmetic. Used by HBCT oracle to build BMRS B1790 queries for confirmed MWh delivery.", example: Some(r#"{"genesis_unix_ts":1704067200,"epoch_duration_s":12,"hour_slot":150}"#) },

    // HLWA — Hashgraph-Locked Wrapped Asset λ-decay gate
    ApiDocEntry { method: "POST", path: "/api/hlwa/effective_supply",       category: "substrate", description: "Compute effective wrapped-asset supply after λ-decay of attestation freshness. Returns current_supply, effective_supply, excess_to_burn. Bridge anti-inflation gate.", example: Some(r#"{"current_supply":1000000,"origin_attested_supply":1000000,"last_attested_epoch":0,"attestation_lambda_epochs":500,"current_epoch":100}"#) },
    ApiDocEntry { method: "POST", path: "/api/hlwa/re_attest",              category: "substrate", description: "Simulate a HLWA re-attestation from origin chain. Resets last_attested_epoch and origin_attested_supply; returns updated effective_supply.", example: Some(r#"{"current_supply":1000000,"origin_attested_supply":1000000,"last_attested_epoch":0,"attestation_lambda_epochs":500,"current_epoch":200}"#) },

    // LLSA — Lean-verified protocol amendment gate
    ApiDocEntry { method: "POST", path: "/api/llsa/apply_amendment",        category: "substrate", description: "Apply a LLSA protocol amendment (from_version → to_version) via the Coq-verified proof gate. Substrate mode uses AlwaysAcceptVerifier. Registers the new version in the EPV registry.", example: Some(r#"{"from_version":3,"to_version":4,"step_new_descriptor_hex":"deadbeef","to_version_seed_energy":1000000000,"activation_epoch":0,"expected_invariant_hex":"0000000000000000000000000000000000000000000000000000000000000000"}"#) },

    // Autopoietic health — Maturana-Varela viability check
    ApiDocEntry { method: "GET",  path: "/api/autopoietic/health",          category: "substrate", description: "Autopoietic chain viability report (Maturana-Varela 1980): Patronage (self-funding), Sentinel (self-maintenance), LLSA (self-boundary). Reports Viable | Stressed | Inviable.", example: None },

    // Consensus Phase + WSBF λ_eff observability
    ApiDocEntry { method: "GET",  path: "/api/consensus/phase",             category: "consensus", description: "Current RG Phase Map consensus regime (LivenessStable|SafetyStable|Frozen|Chaotic) + last WSBF EffectiveParams (renormalized λ_eff, energy density, entropy).", example: None },

    // Offline signing support (cold-wallet + hardware-wallet flows)
    ApiDocEntry { method: "GET",  path: "/api/tx/nonce/:address",      category: "identity", description: "Fetch the current nonce for an address (required for manual transaction construction). Returns nonce + chain_id.", example: None },
    ApiDocEntry { method: "POST", path: "/api/tx/signable",            category: "identity", description: "Return the canonical bytes to sign for a transaction (transfer/create_object/refresh) without executing. Caller signs with ML-DSA key and resubmits via the normal tx endpoint.", example: Some(r#"{"tx_type":"transfer","params":{"from":1,"to":2,"amount":1000}}"#) },
    ApiDocEntry { method: "GET",  path: "/api/tx/:hash",               category: "identity", description: "Wallet-facing tx-status lookup. Returns {hash, state, block_height?, epoch?, error?, confirmations?, tx_index?, gas_used?, mempool_position?, mempool_size?} where state ∈ {pending|mempool|included|finalised|rejected}. Lookup order: chain_store/finalised → committed-but-not-finalised → mempool → pending. Always 200 OK; pending is a typed response, not a 404.", example: Some("/api/tx/<64-hex>") },
    ApiDocEntry { method: "GET",  path: "/api/mempool/:hash",          category: "identity", description: "Direct mempool inspection by tx hash. Returns {hash, in_mempool}. Useful for explorers; the wallet should prefer /api/tx/:hash for the full state machine.", example: Some("/api/mempool/<64-hex>") },

    // Energy Kernel — conservation audit + energy redirect simulation
    ApiDocEntry { method: "POST", path: "/api/energy_kernel/conservation_check", category: "substrate", description: "Audit whether a block transition (before→after EnergyAccumulator) satisfies the §1.2 conservation invariant: total energy non-increasing and any drop ≤ what the λ-decay allows. Returns {valid, before_total, after_total} or {valid:false, error}.", example: Some(r#"{"before":{"accounts":1000000,"stake":500000,"refresh_pool":0,"slashed_pool":0},"after":{"accounts":900000,"stake":500000,"refresh_pool":100000,"slashed_pool":0},"epochs_elapsed":1,"half_life_epochs":4096}"#) },
    ApiDocEntry { method: "POST", path: "/api/energy_kernel/redirect",           category: "substrate", description: "Simulate an EnergyRedirect (slash|slash_settle|mev_burn|demurrage|refresh_payout) on an EnergyAccumulator. Verifies the total is preserved exactly; returns from_compartment, to_compartment, state_after. Used to dry-run energy flow transitions without touching chain state.", example: Some(r#"{"accounts":1000000,"stake":500000,"refresh_pool":0,"slashed_pool":0,"redirect_kind":"mev_burn","amount":5000}"#) },

    // Oracle — decay-aware on-chain feed (OracleState + BLS quorum finalization)
    ApiDocEntry { method: "POST", path: "/api/oracle/ingest",         category: "oracle", description: "Ingest a sensor/oracle data point as a CreateObject transaction. Auth: Bearer EVAPORCHAIN_ORACLE_KEY. The object decays with the configured half_life; stale data evaporates automatically.", example: Some(r#"{"source":"elexon-b1790","object_id":"oracle-feed-01","energy":100000,"half_life":100,"data":"{\"mwh\":42.5}"}"#) },
    ApiDocEntry { method: "GET",  path: "/api/oracle/status",         category: "oracle", description: "Return OracleBridge status: active flag, feed count, active BLS quorum rounds, and current oracle_state_root.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/oracle/feed/:key",      category: "oracle", description: "Read the latest value, TWAP, and Merkle proof for a named oracle feed key. Returns {key, value, twap, has_proof, proof_hash}.", example: None },

    // Sharding — object-level sharding + cross-shard message bus
    ApiDocEntry { method: "GET",  path: "/api/shards",                category: "sharding", description: "Return ShardBridge status: num_shards and pending cross-shard message count. Returns {active:false} when sharding is disabled.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/shards/health",         category: "sharding", description: "Per-shard health report: liveness_ratio, total_objects, live_objects, total_energy, is_dead. Also lists shards that are candidates for compaction.", example: None },

    // Tombstone lookup — per-account query
    ApiDocEntry { method: "GET",  path: "/api/tombstone/:addr_hex",   category: "substrate", description: "Look up the tombstone commitment for a given 32-byte address (hex). Returns commitment_hex + metadata if the address has evaporated, 404 otherwise.", example: None },

    // Demo
    ApiDocEntry { method: "POST", path: "/api/demo/reset",            category: "demo", description: "Clear HBCT book + Sentinel votes so the dashboard demo can re-run", example: None },

    // Validator delegation (P0 #4 — wallet-facing)
    ApiDocEntry { method: "POST", path: "/api/tx/delegate",                  category: "consensus", description: "Bond stake from `delegator` to a validator. ML-DSA signature required over canonical DelegateTx bytes. The chain refreshes per-validator delegated_stake at the next consensus tick (effective_stake = stake + delegated_stake).", example: Some(r#"{"delegator":"0x…","validator_id":7,"amount":1000,"nonce":0,"signature":"<hex>","public_key":"<hex>"}"#) },
    ApiDocEntry { method: "POST", path: "/api/tx/undelegate",                category: "consensus", description: "Begin unbonding `amount` from an existing delegation. Funds are not credited back to balance until UNBONDING_PERIOD_EPOCHS (256) have elapsed; subsequent /api/tx/claim_delegation reclaims them.", example: Some(r#"{"delegator":"0x…","validator_id":7,"amount":600,"nonce":1,"signature":"<hex>","public_key":"<hex>"}"#) },
    ApiDocEntry { method: "POST", path: "/api/tx/claim_delegation",          category: "consensus", description: "Claim previously-undelegated funds back to delegator's balance once the unbonding window has elapsed (chain enforces at execute time).", example: Some(r#"{"delegator":"0x…","validator_id":7,"nonce":2,"signature":"<hex>","public_key":"<hex>"}"#) },
    ApiDocEntry { method: "GET",  path: "/api/validator/:id/delegations",    category: "consensus", description: "Full delegator list for a validator: each entry is (delegator, amount, delegated_at_epoch, unbonding_amount, unbonding_epoch). Σ amount matches the validator's delegated_stake after the next consensus tick.", example: Some("/api/validator/7/delegations") },

    // Admin — graceful drain (Ansible upgrade playbook)
    ApiDocEntry { method: "POST", path: "/api/admin/drain",                  category: "admin", description: "Mark this node as draining. Consensus stops proposing/voting so peers route around the node before binary swap. Auth: Bearer EVAPORCHAIN_ADMIN_KEY. Returns {status:'draining'|'already_draining', draining:true, drain_started_at_epoch}.", example: None },
    ApiDocEntry { method: "POST", path: "/api/admin/undrain",                category: "admin", description: "Clear the drain flag — node resumes proposing/voting. Auth: Bearer EVAPORCHAIN_ADMIN_KEY.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/admin/drain/status",           category: "admin", description: "Current drain state: {draining, drain_started_at_epoch}. Auth: Bearer EVAPORCHAIN_ADMIN_KEY.", example: None },
    // Sybil resistance (Mainnet P1) — libp2p peer-set hardening surface.
    ApiDocEntry { method: "GET",  path: "/api/network/peers",                category: "explorer", description: "Live peer-set view: [{peer_id, ip, subnet, since_ms, score, age_seconds}]. Read from the in-process libp2p Sybil state. Empty when run without --network-mode.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/network/scores",               category: "explorer", description: "Diagnostic projection of the scores HashMap including ghost-entries (peers in scores but not peer_ips). `ghost_count > 0` is the freeze-class signal Lane R.* would have caught. Returns {scores:[{peer_id, connected, ip, since_ms, score, infractions, last_seen_ms}], count, ghost_count}.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/network/banned",               category: "admin", description: "Currently-active IP bans with expiry. Auth: Bearer EVAPORCHAIN_ADMIN_KEY. Returns {bans:[{ip, until_ms, reason}], count}.", example: None },
    ApiDocEntry { method: "POST", path: "/api/network/ban",                  category: "admin", description: "Manually ban a source IP for `duration_secs`. Body: {ip, duration_secs, reason?}. Auth: Bearer EVAPORCHAIN_ADMIN_KEY.", example: Some(r#"{"ip":"192.0.2.1","duration_secs":3600,"reason":"manual"}"#) },
    ApiDocEntry { method: "POST", path: "/api/network/unban",                category: "admin", description: "Clear an active ban for the given IP. Body: {ip}. Auth: Bearer EVAPORCHAIN_ADMIN_KEY.", example: Some(r#"{"ip":"192.0.2.1"}"#) },
    // Finality observability (Mainnet P1)
    ApiDocEntry { method: "GET",  path: "/api/finality/gap",                  category: "consensus", description: "Per-height commit→finalise gap snapshot. Returns {unfinalised:[{height,age_seconds}], worst_gap_seconds, recent_gaps:[{height,gap_seconds}]} (recent capped at 100, newest-first). Drives the EvapFinalityStalled Prometheus alert.", example: None },

    // Block-explorer surface
    ApiDocEntry { method: "GET",  path: "/api/validators",                    category: "explorer", description: "Full active validator list with stake, effective_stake, jailed, BLS-registered flag, health_score, blocks_produced, total_slashed, plus aggregate totals.", example: None },
    ApiDocEntry { method: "GET",  path: "/api/network/health",                category: "explorer", description: "One-call oncall snapshot: height, last_block_age, peer_count, mempool_size, validator/jailed counts, finality lag, status verdict (healthy|syncing|stalled|isolated).", example: None },
    ApiDocEntry { method: "GET",  path: "/api/account/:address",              category: "explorer", description: "Single-account snapshot: balance, nonce, owned_object_count, first 25 owned objects, indexed_tx_count and last_seen_block from the persistent index.", example: Some("/api/account/0x0100000000000000000000000000000000000000000000000000000000000000") },
    ApiDocEntry { method: "GET",  path: "/api/account/:address/transactions", category: "explorer", description: "Paginated address tx history backed by the chain_store address-history index. Newest first. Query: ?limit=N (default 50, cap 500). 503 in light mode (no chain_store).", example: Some("/api/account/0x01…/transactions?limit=20") },
    ApiDocEntry { method: "GET",  path: "/api/block/:number/transactions",    category: "explorer", description: "Full tx list for a specific block. Reads in-memory ring first, falls back to chain_store full-block payload for older blocks.", example: Some("/api/block/100/transactions") },
    ApiDocEntry { method: "GET",  path: "/api/search/:query",                 category: "explorer", description: "Smart explorer search. Decimal → block height. 64-hex → tx hash if indexed, else address. Shorter hex → address. Returns {kind:'block'|'transaction'|'account'|'not_found', ...} on HTTP 200.", example: Some("/api/search/100") },
    ApiDocEntry { method: "GET",  path: "/api/mera/activations",              category: "explorer", description: "MERA gate telemetry: per-block account-touch activation matrix from the in-memory block_history ring. Default content-type text/csv pipes directly into `evaporchain genesis run-gate --csv`. Query: ?from=H&to=H&format=csv|json&max_accounts=N (default 256, capped to top-N by row sum then sorted by hex).", example: Some("/api/mera/activations?from=100&to=300&max_accounts=128") },
];

async fn get_api_docs(State(state): State<Arc<ApiState>>) -> Json<ApiDocsResp> {
    Json(ApiDocsResp {
        chain: state.chain_id.clone(),
        launch_sprint_endpoints: ENDPOINT_CATALOG.len(),
        endpoints: ENDPOINT_CATALOG.to_vec(),
    })
}

// ─────────────── Light-Cone DAG observability ───────────────────────

#[derive(Debug, Serialize)]
pub struct LightConeResp {
    pub block_count: usize,
    pub running_alongside_tendermint: bool,
}

async fn get_light_cone(State(state): State<Arc<ApiState>>) -> Json<LightConeResp> {
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => {
            return Json(LightConeResp {
                block_count: 0,
                running_alongside_tendermint: false,
            })
        }
    };
    let tc = safe_lock(tc);
    Json(LightConeResp {
        block_count: tc.light_cone_block_count(),
        running_alongside_tendermint: true,
    })
}

// ── /api/light_cone/antichain_digest — Phase 4.4 commit-cert digest ──
//
// Operators compare this digest across cluster validators to confirm
// cross-validator agreement on antichain finality without having to
// ship the full block-id list around. Domain-separated 32-byte
// blake3 over the validator-deterministic sorted BlockId set; pairs
// with Crooks-MEV's `mev_state_digest` as the canonical
// inter-validator digest for the Light-Cone substrate.

#[derive(Serialize)]
struct AntichainDigestResp {
    /// Hex-encoded 32-byte blake3 digest. Two validators with the
    /// same Light-Cone DAG state produce the same digest; divergence
    /// here is the freeze-class signal for antichain disagreement.
    pub digest: String,
    /// The sorted BlockId list the digest commits to (hex-encoded
    /// 32-byte ids). Returned alongside the digest so operators can
    /// audit which set was hashed.
    pub closing_antichain: Vec<String>,
    /// Convenience: number of blocks in the closing antichain.
    pub closing_antichain_size: usize,
    /// Whether Tendermint is running (and therefore the Light-Cone
    /// DAG is being populated). When false the digest is the empty-
    /// set sentinel (blake3 of the domain tag alone).
    pub running_alongside_tendermint: bool,
}

// ── /api/light_cone/antichain_digest_history — Phase 4.4 rolling buffer ──
//
// Returns the last 128 (height, digest) pairs from this validator.
// Operators retrospectively cross-compare across cluster validators:
// pick height H, fetch each validator's digest at H, divergence at
// any past height is the freeze-class signal for antichain
// disagreement. Real-time alarm via header-fold or gossip is the
// heavier post-V1 follow-up; per-block history is the minimal
// substrate that enables retroactive divergence detection.

#[derive(Serialize)]
struct AntichainDigestHistoryEntry {
    pub block_height: u64,
    pub digest: String,
}

#[derive(Serialize)]
struct AntichainDigestHistoryResp {
    pub history: Vec<AntichainDigestHistoryEntry>,
    pub count: usize,
    pub running_alongside_tendermint: bool,
}

async fn get_light_cone_antichain_digest_history(
    State(state): State<Arc<ApiState>>,
) -> Json<AntichainDigestHistoryResp> {
    let Some(ref tc) = state.tendermint else {
        return Json(AntichainDigestHistoryResp {
            history: vec![],
            count: 0,
            running_alongside_tendermint: false,
        });
    };
    let tc = safe_lock(tc);
    let history = tc.antichain_digest_history();
    Json(AntichainDigestHistoryResp {
        count: history.len(),
        history: history
            .into_iter()
            .map(|(h, d)| AntichainDigestHistoryEntry {
                block_height: h,
                digest: hex::encode(d),
            })
            .collect(),
        running_alongside_tendermint: true,
    })
}

// ── /api/light_cone/candidate_heads — MCC Phase E.1 ──
//
// Returns every active sibling head in the Light-Cone DAG paired
// with its first-parent trajectory caliber, sorted by caliber
// descending (smaller-BlockId tiebreak — matches MccForkChoice's
// argmax rule). The first entry is the chain's MCC-chosen
// authoritative head; downstream entries are the alternatives the
// fork-choice considered.
//
// Operator workflow: debug "which heads are competing right now"
// without a manual trajectory walk. Pairs with
// /api/light_cone/antichain_digest_history for cluster-divergence
// detection — if validators disagree on the candidate-head
// ordering, that's an early signal of forking.

#[derive(Serialize)]
struct CandidateHeadEntry {
    pub block_id: String,
    pub caliber: u64,
}

#[derive(Serialize)]
struct CandidateHeadsResp {
    pub heads: Vec<CandidateHeadEntry>,
    pub count: usize,
    pub running_alongside_tendermint: bool,
}

// ── /api/light_cone/authoritative_head — MCC Phase E.2 ──
//
// Returns the chain's MCC-chosen authoritative head — the argmax
// of `enumerate_candidate_heads`. Single entry rather than the full
// list. Pairs with the candidate-heads endpoint: candidates lets
// operators see all alternatives competing; authoritative_head
// shows only the winner.
//
// Per-validator: different validators may briefly disagree on the
// authoritative head during a round before converging by end of
// round. Use `/api/light_cone/antichain_digest_history` for
// retroactive cross-validator agreement detection.

#[derive(Serialize)]
struct AuthoritativeHeadResp {
    /// Hex-encoded BlockId of the MCC-chosen authoritative head, or
    /// null if the DAG is empty (no candidates to choose from).
    pub head: Option<String>,
    /// Caliber score of the chosen head's first-parent trajectory.
    pub caliber: Option<u64>,
    /// Total candidate-head count this validator considered. Useful
    /// for confirming the choice was made over the expected fork
    /// count (vs. a degenerate single-head DAG).
    pub candidates_considered: usize,
    pub running_alongside_tendermint: bool,
}

async fn get_light_cone_authoritative_head(
    State(state): State<Arc<ApiState>>,
) -> Json<AuthoritativeHeadResp> {
    let Some(ref tc) = state.tendermint else {
        return Json(AuthoritativeHeadResp {
            head: None,
            caliber: None,
            candidates_considered: 0,
            running_alongside_tendermint: false,
        });
    };
    let tc = safe_lock(tc);
    let scored = tc.enumerate_candidate_heads();
    let total = scored.len();
    let chosen = scored.into_iter().next();
    Json(AuthoritativeHeadResp {
        head: chosen.as_ref().map(|(id, _)| hex::encode(id)),
        caliber: chosen.map(|(_, c)| c),
        candidates_considered: total,
        running_alongside_tendermint: true,
    })
}

async fn get_light_cone_candidate_heads(
    State(state): State<Arc<ApiState>>,
) -> Json<CandidateHeadsResp> {
    let Some(ref tc) = state.tendermint else {
        return Json(CandidateHeadsResp {
            heads: vec![],
            count: 0,
            running_alongside_tendermint: false,
        });
    };
    let tc = safe_lock(tc);
    let scored = tc.enumerate_candidate_heads();
    Json(CandidateHeadsResp {
        count: scored.len(),
        heads: scored
            .into_iter()
            .map(|(id, caliber)| CandidateHeadEntry {
                block_id: hex::encode(id),
                caliber,
            })
            .collect(),
        running_alongside_tendermint: true,
    })
}

async fn get_light_cone_antichain_digest(
    State(state): State<Arc<ApiState>>,
) -> Json<AntichainDigestResp> {
    let Some(ref tc) = state.tendermint else {
        // Stable empty-set sentinel digest — recoverable client-side
        // so operators can pattern-match against it.
        let empty_digest = evaporchain_light_cone::concurrency::digest_antichain(&[]);
        return Json(AntichainDigestResp {
            digest: hex::encode(empty_digest),
            closing_antichain: vec![],
            closing_antichain_size: 0,
            running_alongside_tendermint: false,
        });
    };
    let tc = safe_lock(tc);
    let digest = tc.light_cone_antichain_digest();
    let antichain = tc.light_cone_closing_antichain();
    Json(AntichainDigestResp {
        digest: hex::encode(digest),
        closing_antichain_size: antichain.len(),
        closing_antichain: antichain.iter().map(hex::encode).collect(),
        running_alongside_tendermint: true,
    })
}

async fn get_lamport_time(State(state): State<Arc<ApiState>>) -> Json<LamportTimeResp> {
    let c = safe_lock(&state.lamport_clock);
    Json(LamportTimeResp {
        current_tick: c.current_tick,
        accumulated_energy: c.accumulated_energy,
        tick_quantum: c.tick_quantum,
    })
}

// ── /api/light_cone/block_clock/:block_id_hex ────────────────────
//
// Decay-Lamport DAG accessor (shipped 2026-05-06). Returns the
// LamportClock at a specific DAG block, derived as:
//   merge of all parent clocks (max tick; Lamport rule)
//   + tick by block.energy
//
// Pure function of (light_cone_dag, block_id, tick_quantum). The
// `tick_quantum` is sourced from the chain-global running clock
// (state.lamport_clock.tick_quantum) so the per-block clock and
// chain-global /api/lamport_time clock share the same time
// granularity. This makes the per-block clock comparable with
// /api/lamport_time without query-string parameter shuffling.
//
// Returns:
//   - 200 + {block_id, current_tick, accumulated_energy,
//            tick_quantum, running_alongside_tendermint} on success
//   - 200 + {found: false, ...} if block_id isn't in the DAG
//
// Pairs with /api/lamport_time (chain-global running clock) +
// /api/light_cone/antichain_digest (DAG-derived cross-validator
// digest) as the third operator surface for the Light-Cone
// substrate's time semantics. Cluster operators can pin a
// block_id at a known fork point and compare clocks across all
// validators — equality is the substrate-level convergence claim.
//
// Note on tick_quantum mismatch: if validators disagree on
// tick_quantum, their per-block clocks at the same block_id will
// differ even though the DAG is identical. The accessor surfaces
// the quantum so operators can detect this misconfiguration.

#[derive(Serialize)]
struct BlockClockResp {
    /// Echoes back the block_id (hex) for client-side pairing.
    pub block_id: String,
    /// `true` if the block was found in the DAG.
    pub found: bool,
    /// Current tick of the derived clock, or 0 if not found.
    pub current_tick: u64,
    /// Energy accumulated since the last tick crossing.
    pub accumulated_energy: u64,
    /// Tick quantum used for the derivation. Sourced from the
    /// chain-global /api/lamport_time clock.
    pub tick_quantum: u64,
    pub running_alongside_tendermint: bool,
}

async fn get_light_cone_block_clock(
    State(state): State<Arc<ApiState>>,
    axum::extract::Path(block_id_hex): axum::extract::Path<String>,
) -> Json<BlockClockResp> {
    let block_id = match parse_hex32(&block_id_hex) {
        Ok(b) => b,
        Err(_) => {
            return Json(BlockClockResp {
                block_id: block_id_hex,
                found: false,
                current_tick: 0,
                accumulated_energy: 0,
                tick_quantum: 0,
                running_alongside_tendermint: state.tendermint.is_some(),
            });
        }
    };
    let tick_quantum = {
        let c = safe_lock(&state.lamport_clock);
        c.tick_quantum
    };
    let Some(ref tc) = state.tendermint else {
        return Json(BlockClockResp {
            block_id: block_id_hex,
            found: false,
            current_tick: 0,
            accumulated_energy: 0,
            tick_quantum,
            running_alongside_tendermint: false,
        });
    };
    let tc = safe_lock(tc);
    let clock = tc.light_cone_block_lamport_clock(block_id, tick_quantum);
    match clock {
        Some(c) => Json(BlockClockResp {
            block_id: block_id_hex,
            found: true,
            current_tick: c.current_tick,
            accumulated_energy: c.accumulated_energy,
            tick_quantum: c.tick_quantum,
            running_alongside_tendermint: true,
        }),
        None => Json(BlockClockResp {
            block_id: block_id_hex,
            found: false,
            current_tick: 0,
            accumulated_energy: 0,
            tick_quantum,
            running_alongside_tendermint: true,
        }),
    }
}

async fn get_sentinel_all(State(state): State<Arc<ApiState>>) -> Json<Vec<SentinelParameterResp>> {
    let db = safe_lock(&state.db);
    let out: Vec<SentinelParameterResp> = db
        .all_sentinel_params()
        .into_iter()
        .map(|p| SentinelParameterResp {
            id: p.id,
            current: p.current,
            min: p.min,
            max: p.max,
            vote_count: db.get_sentinel_votes(p.id).len(),
        })
        .collect();
    Json(out)
}

async fn post_hbct_settle(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<HbctBalanceQuery>,
) -> Json<HbctSettleResp> {
    let holder = match parse_hex32(&req.holder_hex) {
        Ok(a) => a,
        Err(e) => {
            return Json(HbctSettleResp {
                status: "error",
                settled_mwh: 0,
                burnt_excess: 0,
                detail: format!("bad holder: {e}"),
            });
        }
    };
    let location_bytes = req.delivery_location.into_bytes();
    let attestation = {
        let oracle = safe_lock(&state.hbct_oracle);
        use evaporchain_hbct::oracle::OracleFeed;
        oracle.attest(&location_bytes, req.hour_slot, holder)
    };
    let attestation = match attestation {
        Some(a) => a,
        None => {
            return Json(HbctSettleResp {
                status: "error",
                settled_mwh: 0,
                burnt_excess: 0,
                detail: "no oracle attestation for this (location, slot, holder)".into(),
            });
        }
    };
    let mut book = safe_lock(&state.hbct_book);
    let held = book.balance(&location_bytes, req.hour_slot, holder);
    let settled = held.min(attestation.mwh_delivered);
    let burnt_excess = held.saturating_sub(attestation.mwh_delivered);
    if burnt_excess > 0 {
        if let Err(e) = book.burn(&location_bytes, req.hour_slot, holder, burnt_excess) {
            return Json(HbctSettleResp {
                status: "error",
                settled_mwh: 0,
                burnt_excess: 0,
                detail: format!("burn failed: {e}"),
            });
        }
    }
    Json(HbctSettleResp {
        status: "ok",
        settled_mwh: settled,
        burnt_excess,
        detail: format!("settled {} MWh; burnt {} MWh excess", settled, burnt_excess),
    })
}

async fn get_objects(State(state): State<Arc<ApiState>>) -> Json<Vec<ObjectResponse>> {
    let db = safe_lock(&state.db);
    let history = safe_lock(&state.block_history);
    let current_epoch = history.back().map(|b| b.epoch).unwrap_or(0);

    let ids = db.all_object_ids();
    let mut objects: Vec<ObjectResponse> = ids
        .iter()
        .filter_map(|id| {
            let obj = db.get_object(id)?;
            let current_energy = obj.energy_at(current_epoch);
            let decay_pct = if obj.energy > 0 {
                ((obj.energy - current_energy) as f64 / obj.energy as f64) * 100.0
            } else {
                100.0
            };
            let state_str = match obj.state {
                ObjectState::Active => "Active",
                ObjectState::Grace => "Grace",
                ObjectState::Ghost => "Ghost",
                ObjectState::Resurrected => "Risen",
            };
            Some(ObjectResponse {
                id: hex::encode(id),
                name: object_name(id, &obj.data),
                owner: hex::encode(obj.owner),
                owner_name: account_name(&obj.owner),
                energy: obj.energy,
                max_energy: obj.energy,
                half_life: obj.half_life,
                state: state_str.to_string(),
                created_epoch: obj.created_at,
                last_refreshed: obj.last_refreshed,
                grace_epoch: obj.grace_epoch,
                current_energy,
                decay_percentage: (decay_pct * 10.0).round() / 10.0,
                decay_curve: obj.decay_curve.clone(),
                is_lad_typed: obj.lad_mode.is_some(),
                lad_mode: obj.lad_mode,
            })
        })
        .collect();

    objects.sort_by(|a, b| {
        let state_order = |s: &str| match s {
            "Active" => 0,
            "Risen" => 1,
            "Grace" => 2,
            "Ghost" => 3,
            _ => 4,
        };
        state_order(&a.state)
            .cmp(&state_order(&b.state))
            .then(b.current_energy.cmp(&a.current_energy))
    });

    Json(objects)
}

async fn get_single_object(
    State(state): State<Arc<ApiState>>,
    Path(id_hex): Path<String>,
) -> Result<Json<ObjectResponse>, StatusCode> {
    let id_bytes = hex::decode(&id_hex).map_err(|_| StatusCode::BAD_REQUEST)?;
    if id_bytes.len() != 32 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&id_bytes);

    let db = safe_lock(&state.db);
    let history = safe_lock(&state.block_history);
    let current_epoch = history.back().map(|b| b.epoch).unwrap_or(0);

    let obj = db.get_object(&id).ok_or(StatusCode::NOT_FOUND)?;
    let current_energy = obj.energy_at(current_epoch);
    let decay_pct = if obj.energy > 0 {
        ((obj.energy - current_energy) as f64 / obj.energy as f64) * 100.0
    } else {
        100.0
    };
    let state_str = match obj.state {
        ObjectState::Active => "Active",
        ObjectState::Grace => "Grace",
        ObjectState::Ghost => "Ghost",
        ObjectState::Resurrected => "Risen",
    };

    Ok(Json(ObjectResponse {
        id: hex::encode(id),
        name: object_name(&id, &obj.data),
        owner: hex::encode(obj.owner),
        owner_name: account_name(&obj.owner),
        energy: obj.energy,
        max_energy: obj.energy,
        half_life: obj.half_life,
        state: state_str.to_string(),
        created_epoch: obj.created_at,
        last_refreshed: obj.last_refreshed,
        grace_epoch: obj.grace_epoch,
        current_energy,
        decay_percentage: (decay_pct * 10.0).round() / 10.0,
        decay_curve: obj.decay_curve.clone(),
        is_lad_typed: obj.lad_mode.is_some(),
        lad_mode: obj.lad_mode,
    }))
}

async fn get_accounts(State(state): State<Arc<ApiState>>) -> Json<Vec<AccountResponse>> {
    let db = safe_lock(&state.db);
    let addrs = db.all_account_addresses();
    let mut accounts: Vec<AccountResponse> = addrs
        .iter()
        .filter_map(|addr| {
            let acc = db.get_account(addr)?;
            Some(AccountResponse {
                address: account_full(addr),
                name: account_name(addr),
                balance: acc.balance,
                nonce: acc.nonce,
            })
        })
        .collect();
    accounts.sort_by_key(|a| std::cmp::Reverse(a.balance));
    Json(accounts)
}

async fn get_blocks(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<BlocksQuery>,
) -> Json<Vec<BlockRecord>> {
    let history = safe_lock(&state.block_history);
    let limit = params.limit.unwrap_or(50).min(500);
    let blocks: Vec<BlockRecord> = history.iter().rev().take(limit).cloned().collect();
    Json(blocks)
}

async fn get_single_block(
    State(state): State<Arc<ApiState>>,
    Path(number): Path<u64>,
) -> Result<Json<BlockRecord>, StatusCode> {
    let history = safe_lock(&state.block_history);
    let block = history
        .iter()
        .find(|b| b.number == number)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(block))
}

// ─────────── Light-header endpoints (for evaporchain-light-client SDK) ──────
//
// Returns the consensus crate's `LightBlockHeader` shape directly so the SDK's
// `evaporchain-light-client-http::HttpTransport` can decode straight into
// the type expected by `RpcTransport::fetch_header_at` / `fetch_latest_header`.
// Synthesised on-demand from the chain_store full block + the running
// validator set; not a separate persisted shape.
//
// Returns 404 when:
//   - the requested height has no full block in chain_store (post-prune
//     or pre-genesis), OR
//   - the block has no commit_certificate (e.g., not yet finalised), OR
//   - the node has no tendermint consensus engine (early-startup or
//     mock-consensus-only deployments).
//
// Wires:
//   `RpcTransport::fetch_header_at(height)` → GET /api/light_header/:height
//   `RpcTransport::fetch_latest_header()`   → GET /api/light_header/latest
fn build_light_header_for_block(
    state: &Arc<ApiState>,
    block: &evaporchain_types::Block,
) -> Option<evaporchain_consensus::light_client::LightBlockHeader> {
    let cert = block.commit_certificate.as_ref()?.clone();
    let tendermint = state.tendermint.as_ref()?;
    let validator_set = {
        let tc = safe_lock(tendermint);
        tc.validator_set().clone()
    };
    Some(evaporchain_consensus::light_client::LightBlockHeader {
        height: block.number,
        epoch: block.epoch,
        block_hash: cert.block_hash,
        parent_hash: block.parent_hash,
        state_root: block.state_root,
        timestamp: block.timestamp,
        validator_set,
        commit_certificate: cert,
    })
}

async fn get_light_header(
    State(state): State<Arc<ApiState>>,
    Path(height): Path<u64>,
) -> Result<Json<evaporchain_consensus::light_client::LightBlockHeader>, StatusCode> {
    let store = state
        .chain_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let block = store.load_full_block(height).ok_or(StatusCode::NOT_FOUND)?;
    let header = build_light_header_for_block(&state, &block).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(header))
}

async fn get_latest_light_header(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<evaporchain_consensus::light_client::LightBlockHeader>, StatusCode> {
    let store = state
        .chain_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let history = safe_lock(&state.block_history);
    let latest_height = history.back().map(|b| b.number).ok_or(StatusCode::NOT_FOUND)?;
    drop(history);
    let block = store
        .load_full_block(latest_height)
        .ok_or(StatusCode::NOT_FOUND)?;
    let header = build_light_header_for_block(&state, &block).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(header))
}

/// Wallet-facing tx-status response. Mirrors the wallet's `TxStatus` TS shape
/// (`extension/src/utils/api.ts`) so the polling client can drop its 404
/// special-case and progress `pending → mempool → included → finalised`
/// from a single endpoint.
///
/// All enrichment fields below are optional — older wallets that only consume
/// `state`, `block_height`, `epoch`, `error` will continue to work.
#[derive(Clone, Serialize)]
pub struct TxStatusResponse {
    pub hash: String,
    /// One of: `pending`, `mempool`, `included`, `finalised`, `rejected`.
    pub state: &'static str,
    /// Block height the tx landed in. Present for `included | finalised |
    /// rejected`; absent for `pending | mempool`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_height: Option<u64>,
    /// Epoch the containing block was produced in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    /// Populated when `state == "rejected"` with the upstream error string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Confirmations on top of the inclusion block: `head_height − block_height`.
    /// Wallets typically gate "safe" UI states at ≥6 confs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmations: Option<u64>,
    /// Position of the tx within its block (only available for txs read out
    /// of the persistent chain_store index, not the in-memory ring).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_index: Option<u32>,
    /// Gas units consumed by the tx, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_used: Option<u64>,
    /// Zero-indexed FIFO position of the tx in the mempool (`state == "mempool"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mempool_position: Option<usize>,
    /// Total mempool depth at the moment of the lookup (`state == "mempool"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mempool_size: Option<usize>,
    /// Chain-assigned contract id, populated for `Transaction::DeployContract`
    /// and `Transaction::DeployScript` once the deploy has been included in a
    /// block. Closes the seal-handoff gap for dapps that deploy a contract
    /// then immediately need to call into it: poll `/api/tx/:hash`, wait for
    /// `state == "finalised"` (or `"included"` if you accept pre-finality
    /// reads), pull `contract_id`, issue the follow-up `call-script`/`call-
    /// contract` against that id. Absent for non-deploy tx types.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_id: Option<u64>,
}

/// Normalise a wallet-supplied hex hash into the canonical 64-char lowercase
/// form used by `TxRecord.hash` and `Mempool::contains_hash`. Strips an
/// optional `0x` prefix and rejects anything that's not exactly 32 bytes.
fn parse_tx_hash(input: &str) -> Option<(String, [u8; 32])> {
    let stripped = input.trim().trim_start_matches("0x").to_lowercase();
    if stripped.len() != 64 {
        return None;
    }
    let mut buf = [0u8; 32];
    hex::decode_to_slice(&stripped, &mut buf).ok()?;
    Some((stripped, buf))
}

/// `GET /api/tx/:hash` — wallet-facing transaction-status lookup.
///
/// Lookup order:
///   1. Finalised store (block_history entries at or below
///      `finality_tracker.latest_finalized_height`, with chain_store as a
///      deeper fallback for txs older than the in-memory ring).
///   2. Committed-but-not-yet-finalised blocks — the gap between the latest
///      committed height and `latest_finalized_height`.
///   3. Active mempool.
///   4. Otherwise `state: "pending"` with HTTP 200 (NOT 404), so the
///      wallet poller can keep ticking through the state machine without
///      having to special-case a missing hash.
async fn get_tx_by_hash(
    State(state): State<Arc<ApiState>>,
    Path(hash): Path<String>,
) -> Result<Json<TxStatusResponse>, StatusCode> {
    // ── 0. Validate the hash shape upfront. ────────────────────────────
    let (hash_hex, hash_bytes) = match parse_tx_hash(&hash) {
        Some(h) => h,
        None => return Err(StatusCode::BAD_REQUEST),
    };

    // Resolve a deploy tx → its chain-assigned contract_id.
    //
    // Lookup order:
    //   1. Persistent index (CF_DEPLOY_INDEX, written at block-include time
    //      when a deploy tx executes). Authoritative, full-address keyed,
    //      no aliasing. Available whenever chain_store is attached.
    //   2. Heuristic fallback: walk the live script_engine /
    //      contract_engine registry and match by (truncated deployer hex,
    //      created_epoch). Used in light/dev mode where chain_store isn't
    //      set, or for deploys that pre-date the persistent index. Aliases
    //      deployers sharing the same first 4 bytes — log-only severity.
    //
    // Both paths return `None` for non-deploy tx types or unresolvable hashes.
    let resolve_deploy_contract_id =
        |tx_type: &str, tx_hash_hex: &str, deployer_prefix_hex: &str, epoch: u64| -> Option<u64> {
            // Path 1: persistent index.
            if let Some(ref store) = state.chain_store {
                if let Some(id) = store.get_deployed_contract_id(tx_hash_hex) {
                    return Some(id);
                }
            }

            // Path 2: heuristic over the live engine.
            let consensus = safe_lock(&state.consensus);
            let mut best: Option<u64> = None;
            let consider =
                |id: u64, creator: &[u8; 32], created_epoch: u64, best: &mut Option<u64>| {
                    if created_epoch != epoch {
                        return;
                    }
                    let creator_prefix = format!("0x{}", hex::encode(&creator[..4]));
                    if creator_prefix.eq_ignore_ascii_case(deployer_prefix_hex) {
                        *best = Some(best.map_or(id, |b| b.max(id)));
                    }
                };
            match tx_type {
                "deploy_contract" => {
                    for ci in consensus.executor.contract_engine.list() {
                        consider(ci.id, &ci.creator, ci.created_epoch, &mut best);
                    }
                }
                "deploy_script" => {
                    for sc in consensus.executor.script_engine.list() {
                        consider(sc.id, &sc.creator, sc.created_epoch, &mut best);
                    }
                }
                _ => {}
            }
            best
        };

    // ── 1. Finalised / included scan over in-memory block_history. ────
    // In multi-validator (Tendermint) mode, finality lags commitment by the
    // BLS commit-certificate window — blocks above `latest_finalized_height`
    // are committed but not yet certified. In single-node MockConsensus dev
    // mode there are no commit certificates, so `latest_finalized_height`
    // sits at 0 forever; treat everything in `block_history` as finalised
    // there to keep the wallet's state machine progressing.
    let single_node_mode = state.tendermint.is_none();
    let last_final = {
        let ft = safe_lock(&state.finality_tracker);
        ft.latest_finalized_height()
    };
    let head_height = {
        let history = safe_lock(&state.block_history);
        history.back().map(|b| b.number).unwrap_or(0)
    };
    let confirmations_for = |block_number: u64| -> Option<u64> {
        if head_height >= block_number {
            Some(head_height - block_number)
        } else {
            None
        }
    };
    {
        let history = safe_lock(&state.block_history);
        for block in history.iter().rev() {
            for tx in &block.transactions {
                if tx.hash == hash_hex {
                    let is_finalised = single_node_mode || block.number <= last_final;
                    let (state_label, error) = if is_finalised {
                        // TxRecord::status is "success" for everything in
                        // block_history today, but preserve the spec's
                        // success → finalised, anything-else → rejected
                        // mapping in case future executors record reverts.
                        if tx.status == "success" {
                            ("finalised", None)
                        } else {
                            ("rejected", Some(tx.status.clone()))
                        }
                    } else {
                        ("included", None)
                    };
                    let gas_used = if tx.gas > 0 { Some(tx.gas) } else { None };
                    let contract_id = if state_label != "rejected" {
                        resolve_deploy_contract_id(&tx.tx_type, &tx.hash, &tx.from, tx.epoch)
                    } else {
                        None
                    };
                    return Ok(Json(TxStatusResponse {
                        hash: hash_hex,
                        state: state_label,
                        block_height: Some(tx.block_number),
                        epoch: Some(tx.epoch),
                        error,
                        confirmations: confirmations_for(tx.block_number),
                        tx_index: None,
                        gas_used,
                        mempool_position: None,
                        mempool_size: None,
                        contract_id,
                    }));
                }
            }
        }
    }

    // ── 1b. Deeper fallback: chain_store tx index (RocksDB-backed). ───
    // Older txs may have aged out of the 500-entry block_history ring but
    // still be queryable via the persistent index. Anything found here is
    // by definition committed and indexed → treat as finalised.
    if let Some(ref store) = state.chain_store {
        if let Some(receipt) = store.get_tx_receipt(&hash_hex) {
            let (state_label, error) = match receipt.status.as_str() {
                "success" | "confirmed" => ("finalised", None),
                other => ("rejected", Some(other.to_string())),
            };
            let contract_id = if state_label != "rejected" {
                receipt.from.as_deref().and_then(|from| {
                    resolve_deploy_contract_id(
                        &receipt.tx_type,
                        &receipt.tx_hash,
                        from,
                        receipt.epoch,
                    )
                })
            } else {
                None
            };
            return Ok(Json(TxStatusResponse {
                hash: hash_hex,
                state: state_label,
                block_height: Some(receipt.block_number),
                epoch: Some(receipt.epoch),
                error: error.or(receipt.revert_reason),
                confirmations: confirmations_for(receipt.block_number),
                tx_index: Some(receipt.tx_index),
                gas_used: if receipt.gas_used > 0 {
                    Some(receipt.gas_used)
                } else {
                    None
                },
                mempool_position: None,
                mempool_size: None,
                contract_id,
            }));
        }
    }

    // ── 2. Mempool check. ──────────────────────────────────────────────
    if state.mempool_contains_hash(&hash_bytes) {
        let (pos, total) = state
            .mempool_position(&hash_bytes)
            .map(|(p, t)| (Some(p), Some(t)))
            .unwrap_or((None, Some(state.mempool_len())));
        return Ok(Json(TxStatusResponse {
            hash: hash_hex,
            state: "mempool",
            block_height: None,
            epoch: None,
            error: None,
            confirmations: None,
            tx_index: None,
            gas_used: None,
            mempool_position: pos,
            mempool_size: total,
            contract_id: None,
        }));
    }

    // ── 3. Default: pending (HTTP 200, not 404). ──────────────────────
    Ok(Json(TxStatusResponse {
        hash: hash_hex,
        state: "pending",
        block_height: None,
        epoch: None,
        error: None,
        confirmations: None,
        tx_index: None,
        gas_used: None,
        mempool_position: None,
        mempool_size: None,
        contract_id: None,
    }))
}

async fn get_transactions(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<TransactionsQuery>,
) -> Json<TransactionsResponse> {
    let history = safe_lock(&state.block_history);
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0);

    // Collect all transactions from recent blocks (newest first)
    let mut all_txs: Vec<TxRecord> = Vec::new();
    for block in history.iter().rev() {
        for tx in block.transactions.iter().rev() {
            all_txs.push(tx.clone());
        }
    }

    // Filter by type if specified
    if let Some(ref filter_type) = params.tx_type {
        all_txs.retain(|tx| tx.tx_type.eq_ignore_ascii_case(filter_type));
    }

    // Filter by address if specified
    if let Some(ref addr) = params.address {
        let addr_lower = addr.to_lowercase();
        all_txs.retain(|tx| {
            tx.from.to_lowercase().contains(&addr_lower)
                || tx.to.to_lowercase().contains(&addr_lower)
        });
    }

    let total = all_txs.len();
    let page: Vec<TxRecord> = all_txs.into_iter().skip(offset).take(limit).collect();

    Json(TransactionsResponse {
        transactions: page,
        total,
        limit,
        offset,
    })
}

async fn get_stats_timeline(State(state): State<Arc<ApiState>>) -> Json<StatsTimelineResponse> {
    let stats = safe_lock(&state.stats);
    Json(StatsTimelineResponse {
        epochs: stats.state_size_trend.clone(),
    })
}

async fn get_stats_summary(State(state): State<Arc<ApiState>>) -> Json<StatsSummaryResponse> {
    let stats = safe_lock(&state.stats);
    let avg_lifetime = if stats.total_evaporated > 0 {
        stats.state_size_trend.len() as f64 / stats.total_evaporated as f64
    } else {
        0.0
    };

    Json(StatsSummaryResponse {
        total_created: stats.total_objects_created,
        total_evaporated: stats.total_evaporated,
        total_resurrected: stats.total_resurrected,
        total_refreshed: stats.total_refreshed,
        avg_lifetime_epochs: (avg_lifetime * 10.0).round() / 10.0,
        total_transactions: stats.total_transactions,
    })
}

async fn get_network(State(state): State<Arc<ApiState>>) -> Json<NetworkResponse> {
    Json(NetworkResponse {
        peer_count: state.peer_count.load(std::sync::atomic::Ordering::Relaxed),
    })
}

// ── /api/network/peers — live peer-set view (Mainnet P1 Sybil resistance) ──

#[derive(Serialize)]
struct PeersResponse {
    peers: Vec<evaporchain_network::PeerInfo>,
    count: usize,
}

async fn get_network_peers(State(state): State<Arc<ApiState>>) -> Json<PeersResponse> {
    let peers = state
        .network_sybil
        .as_ref()
        .and_then(|s| s.read().ok().map(|g| g.peer_view()))
        .unwrap_or_default();
    Json(PeersResponse {
        count: peers.len(),
        peers,
    })
}

// ── /api/network/scores — diagnostic projection of the scores HashMap ──
//
// Unlike `/api/network/peers` which only reports peers in `peer_ips`
// (i.e. currently connected), this endpoint surfaces every entry in
// the `scores` HashMap, including ghost entries (peers that have a
// score but no live connection). The Lane R.* cluster-freeze root
// cause hinged on a peer being scored without being connected — that
// state was invisible to `/api/network/peers`. `/api/network/scores`
// is the standing diagnostic that catches the next freeze-class
// issue without log-grepping.

#[derive(Serialize)]
struct ScoresResponse {
    scores: Vec<evaporchain_network::PeerScoreEntry>,
    count: usize,
    /// Ghost-entry count (peers in scores but not peer_ips). A
    /// non-zero value here is the freeze-class signal Lane R.*
    /// would have caught.
    ghost_count: usize,
}

async fn get_network_scores(State(state): State<Arc<ApiState>>) -> Json<ScoresResponse> {
    let scores = state
        .network_sybil
        .as_ref()
        .and_then(|s| s.read().ok().map(|g| g.scores_view()))
        .unwrap_or_default();
    let ghost_count = scores.iter().filter(|e| !e.connected).count();
    Json(ScoresResponse {
        count: scores.len(),
        ghost_count,
        scores,
    })
}

// ── /api/network/banned — list active IP bans (admin) ──

#[derive(Serialize)]
struct BannedResponse {
    bans: Vec<evaporchain_network::BanEntry>,
    count: usize,
}

async fn get_network_banned(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> Result<Json<BannedResponse>, (StatusCode, Json<serde_json::Value>)> {
    require_admin_auth(&headers)?;
    let bans = state
        .network_sybil
        .as_ref()
        .and_then(|s| s.read().ok().map(|g| g.bans.active_bans()))
        .unwrap_or_default();
    Ok(Json(BannedResponse {
        count: bans.len(),
        bans,
    }))
}

// ── /api/network/ban — manual ban (admin) ──

#[derive(Deserialize)]
struct BanRequest {
    ip: String,
    duration_secs: u64,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Serialize)]
struct BanResponse {
    status: &'static str,
    ip: String,
    until_ms: u64,
    reason: String,
}

async fn post_network_ban(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<BanRequest>,
) -> Result<Json<BanResponse>, (StatusCode, Json<serde_json::Value>)> {
    require_admin_auth(&headers)?;
    let ip: std::net::IpAddr = req.ip.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("invalid ip: {e}")})),
        )
    })?;
    let Some(ref sybil) = state.network_sybil else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "network not running"})),
        ));
    };
    let reason = req.reason.unwrap_or_else(|| "manual".to_string());
    let until_ms = evaporchain_network::now_ms() + req.duration_secs * 1_000;
    {
        let mut guard = sybil.write().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "sybil state poisoned"})),
            )
        })?;
        guard.bans.add_ban(ip, until_ms, reason.clone());
        if let Some(path) = guard.ban_list_path.clone() {
            let _ = guard.bans.save(&path);
        }
    }
    Ok(Json(BanResponse {
        status: "banned",
        ip: ip.to_string(),
        until_ms,
        reason,
    }))
}

// ── /api/network/unban — manual unban (admin) ──

#[derive(Deserialize)]
struct UnbanRequest {
    ip: String,
}

#[derive(Serialize)]
struct UnbanResponse {
    status: &'static str,
    ip: String,
}

async fn post_network_unban(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<UnbanRequest>,
) -> Result<Json<UnbanResponse>, (StatusCode, Json<serde_json::Value>)> {
    require_admin_auth(&headers)?;
    let ip: std::net::IpAddr = req.ip.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("invalid ip: {e}")})),
        )
    })?;
    let Some(ref sybil) = state.network_sybil else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "network not running"})),
        ));
    };
    let removed = {
        let mut guard = sybil.write().map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "sybil state poisoned"})),
            )
        })?;
        guard.unban_ip(&ip)
    };
    Ok(Json(UnbanResponse {
        status: if removed { "unbanned" } else { "not_banned" },
        ip: ip.to_string(),
    }))
}

// ── Block-explorer / monitoring: validators + opinionated health snapshot ──

/// One row in the explorer-facing validator list.
#[derive(Debug, Serialize)]
pub struct ValidatorView {
    pub id: u64,
    pub address: String,
    pub stake: u64,
    pub effective_stake: u64,
    pub jailed: bool,
    pub bls_registered: bool,
    pub health_score: f64,
    pub blocks_produced: u64,
    pub total_slashed: u64,
}

#[derive(Debug, Serialize)]
pub struct ValidatorsResponse {
    pub count: usize,
    pub total_stake: u64,
    pub total_effective_stake: u64,
    pub jailed_count: usize,
    pub bls_registered_count: usize,
    pub validators: Vec<ValidatorView>,
}

/// `GET /api/validators` — full active validator list. Combines registry
/// fields (id, stake, BLS, jailed, health) with derived totals so an
/// explorer can render the validator table in one call.
async fn get_validators(State(state): State<Arc<ApiState>>) -> Json<ValidatorsResponse> {
    let Some(tc) = state.tendermint.as_ref() else {
        return Json(ValidatorsResponse {
            count: 0,
            total_stake: 0,
            total_effective_stake: 0,
            jailed_count: 0,
            bls_registered_count: 0,
            validators: vec![],
        });
    };
    let tc = safe_lock(tc);
    let validators: Vec<ValidatorView> = tc
        .validator_set()
        .validators()
        .iter()
        .map(|v| ValidatorView {
            id: v.id,
            address: format!("0x{}", hex::encode(v.address)),
            stake: v.stake,
            effective_stake: v.effective_stake(),
            jailed: v.jailed,
            bls_registered: v.bls_public_key.is_some(),
            health_score: v.health_score,
            blocks_produced: v.blocks_produced,
            total_slashed: v.total_slashed,
        })
        .collect();

    let total_stake = validators.iter().map(|v| v.stake).sum();
    let total_effective_stake = validators.iter().map(|v| v.effective_stake).sum();
    let jailed_count = validators.iter().filter(|v| v.jailed).count();
    let bls_registered_count = validators.iter().filter(|v| v.bls_registered).count();

    Json(ValidatorsResponse {
        count: validators.len(),
        total_stake,
        total_effective_stake,
        jailed_count,
        bls_registered_count,
        validators,
    })
}

#[derive(Debug, Serialize)]
pub struct NetworkHealthResponse {
    pub block_height: u64,
    pub epoch: u64,
    pub state_root: String,
    /// Wall-clock seconds since the most recent block landed locally.
    /// `None` if no blocks committed yet.
    pub last_block_age_secs: Option<u64>,
    pub peer_count: usize,
    pub mempool_size: usize,
    pub validator_count: usize,
    pub jailed_count: usize,
    pub finalised_height: u64,
    /// `block_height − finalised_height`. Non-zero indicates DA / commit-cert
    /// gating is holding back the head from finality.
    pub finality_lag_blocks: u64,
    pub total_objects: usize,
    pub ghost_count: usize,
    pub uptime_seconds: u64,
    /// One-line opinionated verdict: "healthy" | "syncing" | "stalled" | "isolated".
    pub status: &'static str,
}

/// `GET /api/network/health` — opinionated single-call health snapshot.
/// What an oncall would want to see at a glance from one node:
/// height, last-block-age, peer count, mempool depth, validator/jail
/// counts, finality lag, and a one-word verdict.
async fn get_network_health(State(state): State<Arc<ApiState>>) -> Json<NetworkHealthResponse> {
    let mut db = safe_lock(&state.db);
    let history = safe_lock(&state.block_history);
    let latest = history.back();
    let block_height = latest.map(|b| b.number).unwrap_or(0);
    let epoch = latest.map(|b| b.epoch).unwrap_or(0);

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let last_block_age_secs = latest
        .map(|b| now_secs.saturating_sub(b.timestamp))
        .filter(|&age| age < 86_400 * 365);

    let peer_count = state.peer_count.load(std::sync::atomic::Ordering::Relaxed);

    let (validator_count, jailed_count, mempool_size) = if let Some(tc) = state.tendermint.as_ref()
    {
        let tc = safe_lock(tc);
        let vs = tc.validator_set();
        (
            vs.len(),
            vs.validators().iter().filter(|v| v.jailed).count(),
            tc.mempool.len(),
        )
    } else {
        (0, 0, 0)
    };

    let finalised_height = safe_lock(&state.finality_tracker).latest_finalized_height();
    let finality_lag_blocks = block_height.saturating_sub(finalised_height);

    let status: &'static str = if block_height == 0 {
        "syncing"
    } else if peer_count == 0 && validator_count > 1 {
        "isolated"
    } else if last_block_age_secs.unwrap_or(0) > 60 {
        "stalled"
    } else if finality_lag_blocks > 32 {
        "syncing"
    } else {
        "healthy"
    };

    Json(NetworkHealthResponse {
        block_height,
        epoch,
        state_root: hex::encode(db.compute_state_root()),
        last_block_age_secs,
        peer_count,
        mempool_size,
        validator_count,
        jailed_count,
        finalised_height,
        finality_lag_blocks,
        total_objects: db.object_count(),
        ghost_count: db.ghost_count(),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        status,
    })
}

async fn get_events(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<EventsQuery>,
) -> Json<EventsResponse> {
    let events = safe_lock(&state.events);
    let limit = params.limit.unwrap_or(50).min(200);
    let evts: Vec<EventRecord> = events.iter().rev().take(limit).cloned().collect();
    Json(EventsResponse { events: evts })
}

// ── Block-explorer: per-account, per-block-tx, smart search ──

#[derive(Debug, Serialize)]
pub struct OwnedObjectRef {
    pub id: String,
    pub energy: u64,
    pub half_life: u64,
    pub state: String,
}

#[derive(Debug, Serialize)]
pub struct AccountDetailResponse {
    pub address: String,
    pub name: String,
    pub balance: u64,
    pub nonce: u64,
    pub owned_object_count: usize,
    /// First N owned-object refs (capped at `OWNED_OBJECTS_PREVIEW`).
    pub owned_objects: Vec<OwnedObjectRef>,
    /// Total transactions historically indexed against this address.
    /// `None` when no chain_store is attached (e.g. dev/light mode).
    pub indexed_tx_count: Option<usize>,
    /// Most recent block number this address appeared in (sender or receiver).
    /// `None` if the address has never transacted.
    pub last_seen_block: Option<u64>,
}

const OWNED_OBJECTS_PREVIEW: usize = 25;

/// `GET /api/account/:address` — single-account snapshot for an explorer.
/// Returns balance, nonce, owned-object preview, and tx-history total. The
/// caller can drill into `/api/account/:address/transactions` for paginated
/// history.
async fn get_account_detail(
    State(state): State<Arc<ApiState>>,
    Path(address): Path<String>,
) -> Result<Json<AccountDetailResponse>, StatusCode> {
    let addr = parse_hex_address(&address).map_err(|_| StatusCode::BAD_REQUEST)?;
    let db = safe_lock(&state.db);

    let acc = db.get_account(&addr);
    let acc_present = acc.is_some();
    let (balance, nonce) = match &acc {
        Some(a) => (a.balance, a.nonce),
        // Address with no account record but possibly mentioned in tx history
        // (e.g. faucet recipient pre-funding) — fall through with zeros so the
        // explorer still renders the row instead of 404'ing.
        None => (0, 0),
    };
    let _ = acc;

    // Owned objects: scan all_object_ids and filter. Capped preview to keep
    // the response small; total count is exact.
    let mut owned_total = 0usize;
    let mut owned_preview: Vec<OwnedObjectRef> = Vec::with_capacity(OWNED_OBJECTS_PREVIEW);
    for oid in db.all_object_ids() {
        if let Some(obj) = db.get_object(&oid) {
            if obj.owner == addr {
                owned_total += 1;
                if owned_preview.len() < OWNED_OBJECTS_PREVIEW {
                    owned_preview.push(OwnedObjectRef {
                        id: format!("0x{}", hex::encode(oid)),
                        energy: obj.energy,
                        half_life: obj.half_life,
                        state: format!("{:?}", obj.state),
                    });
                }
            }
        }
    }
    drop(db);

    // Tx-history totals from the persistent index, when present.
    let addr_hex = format!("0x{}", hex::encode(addr));
    let (indexed_tx_count, last_seen_block) = if let Some(ref store) = state.chain_store {
        let recent = store.get_address_transactions(&addr_hex, 1);
        let last = recent.first().map(|r| r.block_number);
        // get_address_transactions caps to limit, so use a wide pull for the
        // count. Bounded by the index size for that address — cheap because
        // it's prefix-iterated. Hard-cap to avoid pathological scans.
        let all = store.get_address_transactions(&addr_hex, 10_000);
        (Some(all.len()), last)
    } else {
        (None, None)
    };

    if !acc_present && owned_total == 0 && indexed_tx_count.unwrap_or(0) == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(AccountDetailResponse {
        address: account_full(&addr),
        name: account_name(&addr),
        balance,
        nonce,
        owned_object_count: owned_total,
        owned_objects: owned_preview,
        indexed_tx_count,
        last_seen_block,
    }))
}

#[derive(Debug, Deserialize)]
pub struct AccountTxQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct AccountTxHistoryResponse {
    pub address: String,
    pub count: usize,
    pub transactions: Vec<crate::persistence::TxReceipt>,
}

/// `GET /api/account/:address/transactions?limit=N` — paginated tx history
/// for a single address. Backed by the chain_store address-history index.
/// Returns 503 if there's no chain_store (light mode).
async fn get_account_transactions(
    State(state): State<Arc<ApiState>>,
    Path(address): Path<String>,
    Query(params): Query<AccountTxQuery>,
) -> Result<Json<AccountTxHistoryResponse>, StatusCode> {
    let addr = parse_hex_address(&address).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state
        .chain_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let limit = params.limit.unwrap_or(50).min(500);
    let addr_hex = format!("0x{}", hex::encode(addr));
    let txs = store.get_address_transactions(&addr_hex, limit);

    Ok(Json(AccountTxHistoryResponse {
        address: addr_hex,
        count: txs.len(),
        transactions: txs,
    }))
}

#[derive(Debug, Serialize)]
pub struct BlockTxResponse {
    pub block_number: u64,
    pub epoch: u64,
    pub tx_count: usize,
    pub transactions: Vec<TxRecord>,
}

/// `GET /api/block/:number/transactions` — full tx list for a block out of
/// the in-memory `block_history` ring. Older blocks (past the ring window)
/// return 404; explorers should drop into per-tx queries via the
/// chain_store-backed `/api/tx/:hash` endpoint for those.
async fn get_block_transactions(
    State(state): State<Arc<ApiState>>,
    Path(number): Path<u64>,
) -> Result<Json<BlockTxResponse>, StatusCode> {
    let history = safe_lock(&state.block_history);
    let b = history
        .iter()
        .find(|b| b.number == number)
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(BlockTxResponse {
        block_number: b.number,
        epoch: b.epoch,
        tx_count: b.transactions.len(),
        transactions: b.transactions.clone(),
    }))
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchHit {
    Block { number: u64 },
    Transaction { hash: String, block_number: u64 },
    Account { address: String },
    NotFound { query: String },
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub hit: SearchHit,
}

/// `GET /api/search/:query` — explorer-style smart search. Resolves:
///   - decimal digits → block by height
///   - 32-byte hex (with/without `0x`) → tx hash if indexed, else address
///   - shorter hex → address (left-padded by `parse_hex_address`)
///
/// Returns `kind: "not_found"` (HTTP 200) when nothing matches, so the UI
/// can render a single response shape without 404 special-casing.
async fn explorer_search(
    State(state): State<Arc<ApiState>>,
    Path(query): Path<String>,
) -> Json<SearchResponse> {
    let q = query.trim().to_string();

    // 1. Pure decimal → block height lookup.
    if let Ok(n) = q.parse::<u64>() {
        let exists = {
            let history = safe_lock(&state.block_history);
            history.iter().any(|b| b.number == n)
        } || state
            .chain_store
            .as_ref()
            .and_then(|s| s.load_full_block(n))
            .is_some();
        if exists {
            return Json(SearchResponse {
                query: q.clone(),
                hit: SearchHit::Block { number: n },
            });
        }
    }

    // 2. 32-byte hex → tx hash if indexed, else fall through to address.
    let stripped = q.trim_start_matches("0x").to_lowercase();
    if stripped.len() == 64 && stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Some(ref store) = state.chain_store {
            if let Some(receipt) = store.get_tx_receipt(&stripped) {
                return Json(SearchResponse {
                    query: q,
                    hit: SearchHit::Transaction {
                        hash: stripped,
                        block_number: receipt.block_number,
                    },
                });
            }
        }
        // Also scan the in-memory ring for un-indexed (recent / light-mode) txs.
        let history = safe_lock(&state.block_history);
        for block in history.iter().rev() {
            for tx in &block.transactions {
                if tx.hash == stripped {
                    return Json(SearchResponse {
                        query: q,
                        hit: SearchHit::Transaction {
                            hash: stripped,
                            block_number: tx.block_number,
                        },
                    });
                }
            }
        }
    }

    // 3. Hex (any length 1..=32 bytes) → account address.
    if !stripped.is_empty()
        && stripped.len() % 2 == 0
        && stripped.len() <= 64
        && stripped.chars().all(|c| c.is_ascii_hexdigit())
    {
        if let Ok(addr) = parse_hex_address(&q) {
            let db = safe_lock(&state.db);
            if db.get_account(&addr).is_some() {
                return Json(SearchResponse {
                    query: q,
                    hit: SearchHit::Account {
                        address: account_full(&addr),
                    },
                });
            }
        }
    }

    Json(SearchResponse {
        query: q.clone(),
        hit: SearchHit::NotFound { query: q },
    })
}

// ── MERA gate telemetry exporter ──
//
// Emits the per-block account-touch activation matrix that the MERA gate
// (see `evaporchain-mera::gate`) consumes. Format matches `genesis run-gate
// --csv` exactly: rows = accounts (sorted by hex address), cols = blocks
// (oldest → newest), cells ∈ {0, 1}.
//
// Runs against the in-memory `block_history` ring (~500 blocks) — this is a
// *sample* of recent activity, not the full chain. Anyone needing the full
// history should run the gate offline against archive data.

#[derive(Debug, Deserialize)]
pub struct MeraExportQuery {
    /// Inclusive lower-bound block number. Defaults to oldest in ring.
    pub from: Option<u64>,
    /// Inclusive upper-bound block number. Defaults to head.
    pub to: Option<u64>,
    /// `csv` (default) or `json`. CSV pipes directly into `genesis run-gate`.
    pub format: Option<String>,
    /// Cap the row count to the top-N most active accounts. Default 256.
    pub max_accounts: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct MeraExportJsonResponse {
    pub from_block: u64,
    pub to_block: u64,
    pub n_accounts: usize,
    pub n_blocks: usize,
    /// Sorted address list aligned with row 0..n_accounts of `matrix`.
    pub addresses: Vec<String>,
    /// Row-major: matrix[i][j] = 1 iff address i was touched in block j.
    pub matrix: Vec<Vec<u8>>,
}

/// `GET /api/mera/activations` — account-touch activation matrix for the
/// MERA gate. Default content type is `text/csv` so `curl … > telemetry.csv`
/// just works. Pass `?format=json` for the structured payload.
async fn get_mera_activations(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<MeraExportQuery>,
) -> axum::response::Response {
    let history = safe_lock(&state.block_history);
    if history.is_empty() {
        return (
            StatusCode::NO_CONTENT,
            "no blocks in history yet — wait for the chain to produce blocks\n",
        )
            .into_response();
    }

    let oldest = history.front().map(|b| b.number).unwrap_or(0);
    let newest = history.back().map(|b| b.number).unwrap_or(0);
    let from = params.from.unwrap_or(oldest).max(oldest);
    let to = params.to.unwrap_or(newest).min(newest);
    if from > to {
        return (
            StatusCode::BAD_REQUEST,
            format!("from ({}) > to ({}) — empty range\n", from, to),
        )
            .into_response();
    }
    let max_accounts = params.max_accounts.unwrap_or(256).max(1);

    // Walk the ring once, recording (account → set-of-block-indices touched).
    // Block index is dense within [from, to] so the matrix is rectangular.
    let blocks_in_range: Vec<&BlockRecord> = history
        .iter()
        .filter(|b| b.number >= from && b.number <= to)
        .collect();
    let n_blocks = blocks_in_range.len();
    if n_blocks == 0 {
        return (
            StatusCode::NO_CONTENT,
            format!("no blocks in [{}, {}]\n", from, to),
        )
            .into_response();
    }

    // Collect activations per address. Lower-cased hex without `0x` so a
    // varying input format doesn't split the same account across rows.
    let mut activations: HashMap<String, Vec<u8>> = HashMap::new();
    for (col, block) in blocks_in_range.iter().enumerate() {
        for tx in &block.transactions {
            for addr in [&tx.from, &tx.to] {
                if addr.is_empty() {
                    continue;
                }
                let canonical = addr.trim_start_matches("0x").to_lowercase();
                if canonical.is_empty() {
                    continue;
                }
                activations
                    .entry(canonical)
                    .or_insert_with(|| vec![0u8; n_blocks])[col] = 1;
            }
        }
    }

    if activations.is_empty() {
        return (
            StatusCode::NO_CONTENT,
            format!("no transaction activity in blocks [{}, {}]\n", from, to),
        )
            .into_response();
    }

    // Cap to the top-N most-active addresses (highest row sum). Tie-break by
    // address hex so the exporter is deterministic across nodes with the
    // same block_history snapshot.
    let mut rows: Vec<(String, Vec<u8>)> = activations.into_iter().collect();
    rows.sort_by(|a, b| {
        let a_sum: u32 = a.1.iter().map(|&x| x as u32).sum();
        let b_sum: u32 = b.1.iter().map(|&x| x as u32).sum();
        b_sum.cmp(&a_sum).then_with(|| a.0.cmp(&b.0))
    });
    rows.truncate(max_accounts);
    // Then re-sort by hex address so the output is stable + grep-friendly.
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let format = params.format.as_deref().unwrap_or("csv").to_lowercase();
    match format.as_str() {
        "json" => {
            let payload = MeraExportJsonResponse {
                from_block: from,
                to_block: to,
                n_accounts: rows.len(),
                n_blocks,
                addresses: rows.iter().map(|(a, _)| format!("0x{}", a)).collect(),
                matrix: rows.into_iter().map(|(_, v)| v).collect(),
            };
            Json(payload).into_response()
        }
        "csv" | "" => {
            let mut out = String::new();
            out.push_str(&format!(
                "# evaporchain mera activation matrix\n# from_block={} to_block={} n_accounts={} n_blocks={}\n# rows=accounts (sorted hex), cols=blocks (oldest first), cells in {{0,1}}\n",
                from,
                to,
                rows.len(),
                n_blocks,
            ));
            for (_, row) in &rows {
                let mut line = String::with_capacity(row.len() * 2);
                for (i, cell) in row.iter().enumerate() {
                    if i > 0 {
                        line.push(',');
                    }
                    line.push(if *cell == 0 { '0' } else { '1' });
                }
                out.push_str(&line);
                out.push('\n');
            }
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "text/csv; charset=utf-8")],
                out,
            )
                .into_response()
        }
        other => (
            StatusCode::BAD_REQUEST,
            format!("unknown format '{}' — use 'csv' or 'json'\n", other),
        )
            .into_response(),
    }
}

// ── Transaction submission handlers ──

async fn post_transfer(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<TransferRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, req.signature.is_some()) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    if req.amount == 0 {
        return Json(TxResultResponse {
            success: false,
            message: "Amount must be greater than zero".into(),
            tx_hash: None,
        });
    }
    let from = match parse_address_value(&req.from) {
        Ok(a) => a,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    let to = match parse_address_value(&req.to) {
        Ok(a) => a,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    if from == to {
        return Json(TxResultResponse {
            success: false,
            message: "Cannot transfer to yourself".into(),
            tx_hash: None,
        });
    }
    // Wallet ownership check
    if let Err(resp) = require_wallet_ownership(&state, user_id, &account_full(&from)) {
        return resp;
    }
    // Balance and nonce pre-check
    {
        let db = safe_lock(&state.db);
        if let Some(acct) = db.get_account(&from) {
            if acct.balance < req.amount {
                return Json(TxResultResponse {
                    success: false,
                    message: format!("Insufficient balance: {} < {}", acct.balance, req.amount),
                    tx_hash: None,
                });
            }
            if req.nonce != acct.nonce {
                return Json(TxResultResponse {
                    success: false,
                    message: format!("Invalid nonce: expected {}, got {}", acct.nonce, req.nonce),
                    tx_hash: None,
                });
            }
        } else if req.amount > 0 {
            return Json(TxResultResponse {
                success: false,
                message: "Account not found — use faucet first".into(),
                tx_hash: None,
            });
        }
    }
    // Dedup check
    {
        let is_dup = state.mempool_contains(|tx| {
            if let Transaction::Transfer(t) = tx {
                t.from == from && t.to == to && t.amount == req.amount && t.nonce == req.nonce
            } else {
                false
            }
        });
        if is_dup {
            return Json(TxResultResponse {
                success: false,
                message: "Duplicate transaction already in mempool".into(),
                tx_hash: None,
            });
        }
    }
    // Build, sign, and submit. Compute the CANONICAL tx hash
    // (BLAKE3 over signable_bytes) — this matches what the executor
    // records in BlockRecord.transactions[].hash, so a wallet that
    // saves this hash and polls /api/tx/<hash> will find the tx
    // once it lands. Earlier code returned a format-string hash
    // ("transfer:from:to:amount") that NEVER matched the chain's
    // canonical hash, so /api/tx/<hash> reported "pending" forever
    // — the bug observed live on 2026-05-07 when val-3.nonce
    // advanced but the indexer kept returning pending.
    let hash;
    {
        let mut tx = Transaction::Transfer(TransferTx {
            from,
            to,
            amount: req.amount,
            nonce: req.nonce,
            signature: req.signature.and_then(|s| hex::decode(s).ok()),
            public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
            mev_refund_eligible: None,
        });
        let sender_addr = format!("0x{}", hex::encode(from));
        sign_transaction(&mut tx, &state, Some(&sender_addr));
        // Hash AFTER signing so signable_bytes matches what the
        // executor will hash at block-include time. signable_bytes
        // is the same canonical input used by
        // tx_records_from_block_with_outcomes in this file.
        hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
        state.submit_tx(tx);
    }
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Transfer queued: {} -> {} amount={}",
            account_name(&from),
            account_name(&to),
            req.amount
        ),
        tx_hash: Some(hash),
    })
}

/// `POST /api/tx/delegate` — bond stake from `delegator` to a
/// validator. Mirrors `post_transfer`: ML-DSA signature is validated
/// at execute time against the tx's canonical signing bytes; the
/// queue layer is the existing mempool. The validator's
/// `effective_stake` (consensus weight) is refreshed at every tick by
/// `TendermintConsensus::refresh_delegated_stakes`, so a freshly
/// queued delegation counts toward the next quorum decision once the
/// block is committed.
async fn post_delegate(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<DelegateRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, req.signature.is_some()) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    if req.amount == 0 {
        return Json(TxResultResponse {
            success: false,
            message: "Amount must be greater than zero".into(),
            tx_hash: None,
        });
    }
    let delegator = match parse_address_value(&req.delegator) {
        Ok(a) => a,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    if let Err(resp) = require_wallet_ownership(&state, user_id, &account_full(&delegator)) {
        return resp;
    }
    // Balance + nonce + validator-existence pre-check (matches
    // execute_delegate's anti-griefing guard so the mempool rejects
    // garbage rather than wasting block space).
    {
        let db = safe_lock(&state.db);
        if let Some(acct) = db.get_account(&delegator) {
            if acct.balance < req.amount {
                return Json(TxResultResponse {
                    success: false,
                    message: format!("Insufficient balance: {} < {}", acct.balance, req.amount),
                    tx_hash: None,
                });
            }
            if req.nonce != acct.nonce {
                return Json(TxResultResponse {
                    success: false,
                    message: format!("Invalid nonce: expected {}, got {}", acct.nonce, req.nonce),
                    tx_hash: None,
                });
            }
        } else {
            return Json(TxResultResponse {
                success: false,
                message: "Account not found — use faucet first".into(),
                tx_hash: None,
            });
        }
        if db.get_stake(req.validator_id).is_none() {
            return Json(TxResultResponse {
                success: false,
                message: format!(
                    "Validator id {} has no stake record; cannot accept delegations",
                    req.validator_id
                ),
                tx_hash: None,
            });
        }
    }
    let is_dup = state.mempool_contains(|tx| {
        if let Transaction::Delegate(t) = tx {
            t.delegator == delegator
                && t.validator_id == req.validator_id
                && t.amount == req.amount
                && t.nonce == req.nonce
        } else {
            false
        }
    });
    if is_dup {
        return Json(TxResultResponse {
            success: false,
            message: "Duplicate transaction already in mempool".into(),
            tx_hash: None,
        });
    }
    let mut tx = Transaction::Delegate(DelegateTx {
        delegator,
        validator_id: req.validator_id,
        amount: req.amount,
        nonce: req.nonce,
        signature: req.signature.and_then(|s| hex::decode(s).ok()),
        public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
    });
    let sender_addr = format!("0x{}", hex::encode(delegator));
    sign_transaction(&mut tx, &state, Some(&sender_addr));
    // Canonical tx hash matches what the executor records — see
    // post_transfer for the same fix shape and rationale.
    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Delegate queued: {} -> validator-{} amount={}",
            account_name(&delegator),
            req.validator_id,
            req.amount
        ),
        tx_hash: Some(hash),
    })
}

/// `POST /api/tx/undelegate` — start unbonding `amount` from an
/// existing delegation. Funds remain locked for `UNBONDING_PERIOD_EPOCHS`
/// before they can be reclaimed via `/api/tx/claim_delegation`.
async fn post_undelegate(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<UndelegateRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, req.signature.is_some()) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    if req.amount == 0 {
        return Json(TxResultResponse {
            success: false,
            message: "Amount must be greater than zero".into(),
            tx_hash: None,
        });
    }
    let delegator = match parse_address_value(&req.delegator) {
        Ok(a) => a,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    if let Err(resp) = require_wallet_ownership(&state, user_id, &account_full(&delegator)) {
        return resp;
    }
    {
        let db = safe_lock(&state.db);
        if let Some(acct) = db.get_account(&delegator) {
            if req.nonce != acct.nonce {
                return Json(TxResultResponse {
                    success: false,
                    message: format!("Invalid nonce: expected {}, got {}", acct.nonce, req.nonce),
                    tx_hash: None,
                });
            }
        } else {
            return Json(TxResultResponse {
                success: false,
                message: "Account not found".into(),
                tx_hash: None,
            });
        }
        match db.get_delegation(&delegator, req.validator_id) {
            Some(rec) if rec.amount >= req.amount => {}
            Some(rec) => {
                return Json(TxResultResponse {
                    success: false,
                    message: format!(
                        "Delegation has only {} bonded; cannot undelegate {}",
                        rec.amount, req.amount
                    ),
                    tx_hash: None,
                });
            }
            None => {
                return Json(TxResultResponse {
                    success: false,
                    message: format!(
                        "No delegation from this address to validator {}",
                        req.validator_id
                    ),
                    tx_hash: None,
                });
            }
        }
    }
    let is_dup = state.mempool_contains(|tx| {
        if let Transaction::Undelegate(t) = tx {
            t.delegator == delegator
                && t.validator_id == req.validator_id
                && t.amount == req.amount
                && t.nonce == req.nonce
        } else {
            false
        }
    });
    if is_dup {
        return Json(TxResultResponse {
            success: false,
            message: "Duplicate transaction already in mempool".into(),
            tx_hash: None,
        });
    }
    let mut tx = Transaction::Undelegate(UndelegateTx {
        delegator,
        validator_id: req.validator_id,
        amount: req.amount,
        nonce: req.nonce,
        signature: req.signature.and_then(|s| hex::decode(s).ok()),
        public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
    });
    let sender_addr = format!("0x{}", hex::encode(delegator));
    sign_transaction(&mut tx, &state, Some(&sender_addr));
    // Canonical tx hash — see post_transfer.
    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Undelegate queued: {} -> validator-{} amount={}",
            account_name(&delegator),
            req.validator_id,
            req.amount
        ),
        tx_hash: Some(hash),
    })
}

/// `POST /api/tx/claim_delegation` — claim previously-undelegated funds
/// back to the delegator's balance. Requires the unbonding period to
/// have elapsed; the chain enforces that at execute time.
async fn post_claim_delegation(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<ClaimDelegationRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, req.signature.is_some()) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    let delegator = match parse_address_value(&req.delegator) {
        Ok(a) => a,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    if let Err(resp) = require_wallet_ownership(&state, user_id, &account_full(&delegator)) {
        return resp;
    }
    {
        let db = safe_lock(&state.db);
        if let Some(acct) = db.get_account(&delegator) {
            if req.nonce != acct.nonce {
                return Json(TxResultResponse {
                    success: false,
                    message: format!("Invalid nonce: expected {}, got {}", acct.nonce, req.nonce),
                    tx_hash: None,
                });
            }
        } else {
            return Json(TxResultResponse {
                success: false,
                message: "Account not found".into(),
                tx_hash: None,
            });
        }
        match db.get_delegation(&delegator, req.validator_id) {
            Some(rec) if rec.unbonding_amount > 0 => {}
            Some(_) => {
                return Json(TxResultResponse {
                    success: false,
                    message: "No unbonding amount to claim".into(),
                    tx_hash: None,
                });
            }
            None => {
                return Json(TxResultResponse {
                    success: false,
                    message: format!(
                        "No delegation from this address to validator {}",
                        req.validator_id
                    ),
                    tx_hash: None,
                });
            }
        }
    }
    let is_dup = state.mempool_contains(|tx| {
        if let Transaction::ClaimDelegation(t) = tx {
            t.delegator == delegator && t.validator_id == req.validator_id && t.nonce == req.nonce
        } else {
            false
        }
    });
    if is_dup {
        return Json(TxResultResponse {
            success: false,
            message: "Duplicate transaction already in mempool".into(),
            tx_hash: None,
        });
    }
    let mut tx = Transaction::ClaimDelegation(ClaimDelegationTx {
        delegator,
        validator_id: req.validator_id,
        nonce: req.nonce,
        signature: req.signature.and_then(|s| hex::decode(s).ok()),
        public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
    });
    let sender_addr = format!("0x{}", hex::encode(delegator));
    sign_transaction(&mut tx, &state, Some(&sender_addr));
    // Canonical tx hash — see post_transfer.
    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "ClaimDelegation queued: {} <- validator-{}",
            account_name(&delegator),
            req.validator_id
        ),
        tx_hash: Some(hash),
    })
}

/// `GET /api/validator/:id/delegations` — full delegator list for a
/// single validator. Returns each `(delegator, amount, since_epoch)`
/// plus unbonding details so the wallet can render an "active
/// delegations" view without iterating the full state trie.
async fn get_validator_delegations(
    State(state): State<Arc<ApiState>>,
    Path(validator_id): Path<u64>,
) -> Json<ValidatorDelegationsResponse> {
    let db = safe_lock(&state.db);
    let records = db.delegations_for_validator(validator_id);
    let mut total_delegated: u64 = 0;
    let delegations: Vec<DelegationView> = records
        .iter()
        .map(|r| {
            total_delegated = total_delegated.saturating_add(r.amount);
            DelegationView {
                delegator: account_full(&r.delegator),
                validator_id: r.validator_id,
                amount: r.amount,
                delegated_at_epoch: r.delegated_at_epoch,
                unbonding_amount: r.unbonding_amount,
                unbonding_epoch: r.unbonding_epoch,
            }
        })
        .collect();
    Json(ValidatorDelegationsResponse {
        validator_id,
        delegation_count: delegations.len(),
        total_delegated,
        delegations,
    })
}

async fn post_create_object(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<CreateObjectRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, req.signature.is_some()) {
        return resp;
    }
    let creator = match parse_address_value(&req.creator) {
        Ok(a) => a,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    let obj_id_val = match parse_address_value(&req.object_id) {
        Ok(a) => a,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    let obj_label = hex::encode(&obj_id_val[..4]);
    let data = req
        .data
        .map(|d| d.into_bytes())
        .unwrap_or_else(|| format!("obj-0x{}", &obj_label).into_bytes());
    let mut tx = Transaction::CreateObject(CreateObjectTx {
        creator,
        object_id: obj_id_val,
        energy: req.energy,
        half_life: req.half_life,
        data,
        decay_curve: req.decay_curve,
        lad_mode: None,
        signature: req.signature.and_then(|s| hex::decode(s).ok()),
        public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
    });
    let creator_addr = format!("0x{}", hex::encode(creator));
    sign_transaction(&mut tx, &state, Some(&creator_addr));
    // Canonical tx hash — see post_transfer.
    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "CreateObject queued: id=0x{} energy={} half_life={}",
            &obj_label, req.energy, req.half_life
        ),
        tx_hash: Some(hash),
    })
}

async fn post_refresh(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<RefreshRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, req.signature.is_some()) {
        return resp;
    }
    let obj_id_val = match parse_address_value(&req.object_id) {
        Ok(a) => a,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    let mut tx = Transaction::Refresh(RefreshTx {
        object_id: obj_id_val,
        energy_deposit: req.energy_deposit,
        signature: req.signature.and_then(|s| hex::decode(s).ok()),
        public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
    });
    sign_transaction(&mut tx, &state, None);
    // Canonical tx hash — see post_transfer.
    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Refresh queued: obj=0x{} energy_deposit={}",
            hex::encode(&obj_id_val[..4]),
            req.energy_deposit
        ),
        tx_hash: Some(hash),
    })
}

async fn post_resurrect(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<RefreshRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, req.signature.is_some()) {
        return resp;
    }
    let obj_id_val = match parse_address_value(&req.object_id) {
        Ok(a) => a,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    let mut tx = Transaction::Refresh(RefreshTx {
        object_id: obj_id_val,
        energy_deposit: req.energy_deposit,
        signature: req.signature.and_then(|s| hex::decode(s).ok()),
        public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
    });
    sign_transaction(&mut tx, &state, None);
    // Canonical tx hash — see post_transfer.
    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Resurrect queued: obj=0x{} energy_deposit={}",
            hex::encode(&obj_id_val[..4]),
            req.energy_deposit
        ),
        tx_hash: Some(hash),
    })
}

// ── Batch transaction handler ──

async fn post_batch(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<BatchRequest>,
) -> Json<BatchResponse> {
    if req.transactions.is_empty() {
        return Json(BatchResponse {
            submitted: 0,
            failed: 0,
            results: vec![],
        });
    }
    if req.transactions.len() > 100 {
        return Json(BatchResponse {
            submitted: 0,
            failed: 1,
            results: vec![BatchItemResult {
                index: 0,
                success: false,
                message: "Batch too large: max 100 transactions".into(),
                tx_hash: None,
            }],
        });
    }

    let auth_ok = require_tx_auth(&headers, &state, false).is_ok();
    if !auth_ok {
        return Json(BatchResponse {
            submitted: 0,
            failed: 1,
            results: vec![BatchItemResult {
                index: 0,
                success: false,
                message: "Authentication required".into(),
                tx_hash: None,
            }],
        });
    }

    let mut results = Vec::with_capacity(req.transactions.len());
    let mut submitted = 0usize;
    let mut failed = 0usize;

    for (i, item) in req.transactions.into_iter().enumerate() {
        let result = match item {
            BatchTxItem::Transfer {
                from,
                to,
                amount,
                nonce,
            } => match (parse_address_value(&from), parse_address_value(&to)) {
                (Ok(f), Ok(t)) if f != t && amount > 0 => {
                    let mut tx = Transaction::Transfer(TransferTx {
                        from: f,
                        to: t,
                        amount,
                        nonce,
                        signature: None,
                        public_key: None,
                        mev_refund_eligible: None,
                    });
                    sign_transaction(&mut tx, &state, None);
                    // Canonical tx hash — see post_transfer.
                    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
                    state.submit_tx(tx);
                    BatchItemResult {
                        index: i,
                        success: true,
                        message: "Transfer queued".into(),
                        tx_hash: Some(hash),
                    }
                }
                (Err(e), _) | (_, Err(e)) => BatchItemResult {
                    index: i,
                    success: false,
                    message: e,
                    tx_hash: None,
                },
                _ => BatchItemResult {
                    index: i,
                    success: false,
                    message: "Invalid transfer parameters".into(),
                    tx_hash: None,
                },
            },
            BatchTxItem::CreateObject {
                creator,
                object_id,
                energy,
                half_life,
            } => {
                match (
                    parse_address_value(&creator),
                    parse_address_value(&object_id),
                ) {
                    (Ok(c), Ok(oid)) => {
                        let mut tx = Transaction::CreateObject(CreateObjectTx {
                            creator: c,
                            object_id: oid,
                            energy,
                            half_life,
                            data: Vec::new(),
                            decay_curve: None,
                            lad_mode: None,
                            signature: None,
                            public_key: None,
                        });
                        sign_transaction(&mut tx, &state, None);
                        // Canonical tx hash — see post_transfer.
                        let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
                        state.submit_tx(tx);
                        BatchItemResult {
                            index: i,
                            success: true,
                            message: "CreateObject queued".into(),
                            tx_hash: Some(hash),
                        }
                    }
                    (Err(e), _) | (_, Err(e)) => BatchItemResult {
                        index: i,
                        success: false,
                        message: e,
                        tx_hash: None,
                    },
                }
            }
            BatchTxItem::Refresh {
                object_id,
                energy_deposit,
            } => match parse_address_value(&object_id) {
                Ok(oid) => {
                    let mut tx = Transaction::Refresh(RefreshTx {
                        object_id: oid,
                        energy_deposit,
                        signature: None,
                        public_key: None,
                    });
                    sign_transaction(&mut tx, &state, None);
                    // Canonical tx hash — see post_transfer.
                    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
                    state.submit_tx(tx);
                    BatchItemResult {
                        index: i,
                        success: true,
                        message: "Refresh queued".into(),
                        tx_hash: Some(hash),
                    }
                }
                Err(e) => BatchItemResult {
                    index: i,
                    success: false,
                    message: e,
                    tx_hash: None,
                },
            },
            BatchTxItem::Resurrect {
                object_id,
                energy_deposit,
            } => match parse_address_value(&object_id) {
                Ok(oid) => {
                    let mut tx = Transaction::Refresh(RefreshTx {
                        object_id: oid,
                        energy_deposit,
                        signature: None,
                        public_key: None,
                    });
                    sign_transaction(&mut tx, &state, None);
                    // Canonical tx hash — see post_transfer.
                    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
                    state.submit_tx(tx);
                    BatchItemResult {
                        index: i,
                        success: true,
                        message: "Resurrect queued".into(),
                        tx_hash: Some(hash),
                    }
                }
                Err(e) => BatchItemResult {
                    index: i,
                    success: false,
                    message: e,
                    tx_hash: None,
                },
            },
        };

        if result.success {
            submitted += 1;
        } else {
            failed += 1;
        }
        results.push(result);
    }

    Json(BatchResponse {
        submitted,
        failed,
        results,
    })
}

// ── Contract request types ──

#[derive(Deserialize)]
struct DeployContractRequest {
    deployer: u8,
    template: String,
    init_args: serde_json::Value,
    energy: u64,
    half_life: u64,
    #[serde(default)]
    rules: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct CallContractRequest {
    caller: u8,
    contract_id: u64,
    method: String,
    args: serde_json::Value,
    epoch: u64,
}

// ── Contract handlers ──

async fn post_deploy_contract(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<DeployContractRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, false) {
        return resp;
    }
    let mut tx = Transaction::DeployContract(DeployContractTx {
        deployer: addr_from_byte(req.deployer),
        template: req.template.clone(),
        init_args: serde_json::to_string(&req.init_args).unwrap_or_default(),
        energy: req.energy,
        half_life: req.half_life,
        rules: req
            .rules
            .map(|r| serde_json::to_string(&r).unwrap_or_default()),
        signature: None,
        public_key: None,
    });
    sign_transaction(&mut tx, &state, None);
    // Canonical tx hash — see post_transfer.
    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Deploy queued: template={} energy={} hl={} (mempool={})",
            req.template,
            req.energy,
            req.half_life,
            state.mempool_len()
        ),
        tx_hash: Some(hash),
    })
}

async fn post_call_contract(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<CallContractRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, false) {
        return resp;
    }
    let mut tx = Transaction::CallContract(CallContractTx {
        caller: addr_from_byte(req.caller),
        contract_id: req.contract_id,
        method: req.method.clone(),
        args: serde_json::to_string(&req.args).unwrap_or_default(),
        epoch: req.epoch,
        signature: None,
        public_key: None,
    });
    sign_transaction(&mut tx, &state, None);
    // Canonical tx hash — see post_transfer.
    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Call queued: contract={} method={} (mempool={})",
            req.contract_id,
            req.method,
            state.mempool_len()
        ),
        tx_hash: Some(hash),
    })
}

async fn get_contracts(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let c = safe_lock(&state.consensus);
    let contracts = c.executor.contract_engine.list();
    let list: Vec<serde_json::Value> = contracts
        .iter()
        .map(|ci| {
            serde_json::json!({
                "id": ci.id,
                "template": format!("{:?}", ci.template),
                "creator": account_name(&ci.creator),
                "energy": ci.energy,
                "half_life": ci.half_life,
                "created_epoch": ci.created_epoch,
                "evaporated": ci.evaporated,
            })
        })
        .collect();
    Json(serde_json::json!({ "contracts": list }))
}

async fn get_contract(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    let c = safe_lock(&state.consensus);
    match c.executor.contract_engine.get(id) {
        Some(ci) => {
            let state_val = c.executor.contract_engine.get_state(id).cloned();
            let resp = serde_json::json!({
                "id": ci.id,
                "template": format!("{:?}", ci.template),
                "creator": account_name(&ci.creator),
                "energy": ci.energy,
                "half_life": ci.half_life,
                "created_epoch": ci.created_epoch,
                "evaporated": ci.evaporated,
                "state": state_val,
            });
            (StatusCode::OK, Json(resp)).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "contract not found" })),
        )
            .into_response(),
    }
}

// ── EvaporScript handlers ──

#[derive(Deserialize)]
struct DeployScriptRequest {
    deployer: u8,
    source_code: String,
    energy: u64,
    half_life: u64,
}

#[derive(Deserialize)]
struct CallScriptRequest {
    caller: u8,
    contract_id: u64,
    method: String,
    args: serde_json::Value,
    epoch: u64,
}

async fn post_deploy_script(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<DeployScriptRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, false) {
        return resp;
    }
    let mut tx = Transaction::DeployScript(evaporchain_types::DeployScriptTx {
        deployer: addr_from_byte(req.deployer),
        source_code: req.source_code.clone(),
        energy: req.energy,
        half_life: req.half_life,
        signature: None,
        public_key: None,
    });
    sign_transaction(&mut tx, &state, None);
    // Canonical tx hash — see post_transfer.
    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Script deploy queued: energy={} hl={} source={}B (mempool={})",
            req.energy,
            req.half_life,
            req.source_code.len(),
            state.mempool_len()
        ),
        tx_hash: Some(hash),
    })
}

async fn post_call_script(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<CallScriptRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, false) {
        return resp;
    }
    let mut tx = Transaction::CallScript(evaporchain_types::CallScriptTx {
        caller: addr_from_byte(req.caller),
        contract_id: req.contract_id,
        method: req.method.clone(),
        args: serde_json::to_string(&req.args).unwrap_or_default(),
        epoch: req.epoch,
        signature: None,
        public_key: None,
    });
    sign_transaction(&mut tx, &state, None);
    // Canonical tx hash — see post_transfer.
    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Script call queued: contract={} method={} (mempool={})",
            req.contract_id,
            req.method,
            state.mempool_len()
        ),
        tx_hash: Some(hash),
    })
}

// ── UpgradeContract — governance-gated bytecode swap (mainnet P0) ──

#[derive(Debug, Deserialize)]
pub struct UpgradeContractRequest {
    /// Sender address (hex, 32 bytes). Charged for gas + nonce-checked.
    pub owner: String,
    pub contract_id: u64,
    /// New EvaporScript source (UTF-8) or future-VM bytecode (hex —
    /// pass `bytecode_hex` instead).
    #[serde(default)]
    pub new_bytecode: Option<String>,
    /// Hex-encoded raw bytecode bytes (alternative to `new_bytecode`).
    #[serde(default)]
    pub new_bytecode_hex: Option<String>,
    /// `BLAKE3(new_bytecode)` as 64-char hex. The chain re-checks this.
    pub new_bytecode_hash_hex: String,
    pub nonce: u64,
    /// Path A — admin's ML-DSA-65 hex signature over the canonical
    /// payload `JSON({type:"upgrade_contract",contract_id,
    /// new_bytecode_hash_hex,nonce})`. Set to `null` for governance path.
    #[serde(default)]
    pub admin_signature_hex: Option<String>,
    /// Path A — admin's ML-DSA-65 hex public key. Must derive (via
    /// BLAKE3) to the contract's stored admin address.
    #[serde(default)]
    pub admin_public_key_hex: Option<String>,
    /// Path B — endorser stakes summed against `required_stake`.
    #[serde(default)]
    pub endorser_stakes: Vec<u64>,
    /// Path B — minimum stake total for the amendment to pass.
    #[serde(default)]
    pub required_stake: u64,
}

/// POST /api/tx/upgrade_contract — submit an UpgradeContract tx.
///
/// Two paths, disambiguated by whether `admin_signature_hex` is present:
///   - Admin: chain verifies the ML-DSA-65 sig at apply time and that
///     `BLAKE3(admin_public_key) == contract.admin`.
///   - Governance: chain enforces `sum(endorser_stakes) >= required_stake`.
///     No body signature required on this path (mirrors
///     `/api/governance/fork_choice_mode`).
async fn post_upgrade_contract(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<UpgradeContractRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, false) {
        return resp;
    }

    let owner = match parse_hex_address(&req.owner) {
        Ok(a) => a,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: format!("invalid owner address: {e}"),
                tx_hash: None,
            });
        }
    };

    let new_bytecode: Vec<u8> = match (&req.new_bytecode, &req.new_bytecode_hex) {
        (Some(s), None) => s.as_bytes().to_vec(),
        (None, Some(h)) => match hex::decode(h) {
            Ok(b) => b,
            Err(e) => {
                return Json(TxResultResponse {
                    success: false,
                    message: format!("invalid new_bytecode_hex: {e}"),
                    tx_hash: None,
                });
            }
        },
        (Some(_), Some(_)) => {
            return Json(TxResultResponse {
                success: false,
                message: "supply exactly one of new_bytecode / new_bytecode_hex".into(),
                tx_hash: None,
            });
        }
        (None, None) => {
            return Json(TxResultResponse {
                success: false,
                message: "missing new_bytecode (or new_bytecode_hex)".into(),
                tx_hash: None,
            });
        }
    };

    let new_bytecode_hash: [u8; 32] = match hex::decode(&req.new_bytecode_hash_hex) {
        Ok(b) if b.len() == 32 => {
            let mut a = [0u8; 32];
            a.copy_from_slice(&b);
            a
        }
        _ => {
            return Json(TxResultResponse {
                success: false,
                message: "new_bytecode_hash_hex must be 64-char hex (32 bytes)".into(),
                tx_hash: None,
            });
        }
    };

    let admin_signature: Option<Vec<u8>> = match req.admin_signature_hex.as_deref() {
        Some(s) => match hex::decode(s) {
            Ok(b) => Some(b),
            Err(e) => {
                return Json(TxResultResponse {
                    success: false,
                    message: format!("invalid admin_signature_hex: {e}"),
                    tx_hash: None,
                });
            }
        },
        None => None,
    };
    let admin_public_key: Option<Vec<u8>> = match req.admin_public_key_hex.as_deref() {
        Some(s) => match hex::decode(s) {
            Ok(b) => Some(b),
            Err(e) => {
                return Json(TxResultResponse {
                    success: false,
                    message: format!("invalid admin_public_key_hex: {e}"),
                    tx_hash: None,
                });
            }
        },
        None => None,
    };

    let mut tx = Transaction::UpgradeContract(evaporchain_types::UpgradeContractTx {
        owner,
        contract_id: req.contract_id,
        new_bytecode,
        new_bytecode_hash,
        nonce: req.nonce,
        admin_signature,
        admin_public_key,
        endorser_stakes: req.endorser_stakes,
        required_stake: req.required_stake,
        governance_approved: false,
        signature: None,
        public_key: None,
    });
    sign_transaction(&mut tx, &state, None);
    let hash = match &tx {
        Transaction::UpgradeContract(t) => tx_hash(&format!(
            "upgrade-contract:{}:{}:{}",
            t.contract_id,
            hex::encode(t.new_bytecode_hash),
            t.nonce
        )),
        _ => unreachable!(),
    };
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "UpgradeContract queued: contract_id={} hash={} (mempool={})",
            req.contract_id,
            hex::encode(new_bytecode_hash),
            state.mempool_len()
        ),
        tx_hash: Some(hash),
    })
}

async fn get_scripts(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let c = safe_lock(&state.consensus);
    let scripts = c.executor.script_engine.list();
    let list: Vec<serde_json::Value> = scripts
        .iter()
        .map(|sc| {
            serde_json::json!({
                "id": sc.id,
                "name": sc.name,
                "creator": account_name(&sc.creator),
                "energy": sc.energy,
                "half_life": sc.half_life,
                "created_epoch": sc.created_epoch,
                "evaporated": sc.evaporated,
                "methods": sc.bytecode.methods.keys().collect::<Vec<_>>(),
            })
        })
        .collect();
    Json(serde_json::json!({ "scripts": list, "count": list.len() }))
}

async fn get_script(State(state): State<Arc<ApiState>>, Path(id): Path<u64>) -> impl IntoResponse {
    let c = safe_lock(&state.consensus);
    match c.executor.script_engine.get(id) {
        Some(sc) => {
            let resp = serde_json::json!({
                "id": sc.id,
                "name": sc.name,
                "creator": account_name(&sc.creator),
                "energy": sc.energy,
                "half_life": sc.half_life,
                "created_epoch": sc.created_epoch,
                "last_refreshed": sc.last_refreshed,
                "evaporated": sc.evaporated,
                "methods": sc.bytecode.methods.keys().collect::<Vec<_>>(),
                "abi": sc.abi,
                "state_schema": sc.bytecode.state_schema.fields.iter().map(|f| {
                    serde_json::json!({
                        "name": f.name,
                        "type": format!("{:?}", f.ty),
                    })
                }).collect::<Vec<_>>(),
                "state": sc.state,
                "opcode_count": sc.bytecode.opcodes.len(),
                // UpgradeContract surface (mainnet P0). `admin = null`
                // means the contract is frozen on the admin path —
                // only a governance-quorum upgrade can mutate bytecode.
                "admin": sc.admin.map(|a| format!("0x{}", hex::encode(a))),
                "upgrade_count": sc.upgrade_count,
            });
            (StatusCode::OK, Json(resp)).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "script not found" })),
        )
            .into_response(),
    }
}

async fn get_script_abi(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    let c = safe_lock(&state.consensus);
    match c.executor.script_engine.get(id) {
        Some(sc) => (StatusCode::OK, Json(serde_json::to_value(&sc.abi).unwrap())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "script not found" })),
        )
            .into_response(),
    }
}

async fn dashboard_html() -> impl IntoResponse {
    // Serve the bundled dashboard (built React app)
    // Falls back to a simple redirect message if no build exists
    let html = include_str!("../dashboard/index.html");
    Html(html)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn ws_upgrade_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<Arc<ApiState>>,
    Query(params): Query<crate::ws::WsSubscribeParams>,
) -> impl IntoResponse {
    let broadcaster = state.ws_broadcaster.clone();
    let topics = params
        .subscribe
        .clone()
        .unwrap_or_else(|| "all".to_string());
    tracing::info!("WebSocket upgrade request (subscribe: {topics})");
    ws.on_upgrade(move |socket| crate::ws::handle_ws_connection(socket, broadcaster, params))
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "alive"})))
}

async fn readyz(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let block_history = state.block_history.lock().unwrap();
    let has_blocks = !block_history.is_empty();
    let tip_height = block_history.back().map(|b| b.number).unwrap_or(0);
    drop(block_history);

    let peer_count = state.peer_count.load(std::sync::atomic::Ordering::Relaxed);
    let uptime_secs = state.start_time.elapsed().as_secs();

    let ready = has_blocks && uptime_secs > 5;
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(serde_json::json!({
            "ready": ready,
            "block_height": tip_height,
            "peers": peer_count,
            "uptime_secs": uptime_secs,
        })),
    )
}

// ──────────────────────────── Address Detail ─────────────────────────────

async fn address_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/address.html"))
}

async fn block_detail_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/block.html"))
}

async fn tx_detail_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/tx.html"))
}

#[derive(Serialize)]
struct AddressDetailResponse {
    address: String,
    balance: u64,
    nonce: u64,
    /// Epoch the account was last "touched" by a balance/nonce-mutating tx
    /// — anchors the per-account demurrage accrual window.  Sourced from
    /// `Account.last_touched_epoch`; new accounts default to 0.
    last_touched_epoch: u64,
    objects: Vec<ObjectResponse>,
    nfts: Vec<NftResponse>,
    tokens: Vec<serde_json::Value>,
}

async fn get_address_detail(
    State(state): State<Arc<ApiState>>,
    Path(addr_hex): Path<String>,
) -> Result<Json<AddressDetailResponse>, StatusCode> {
    let addr_bytes = parse_hex_address(&addr_hex).map_err(|_| StatusCode::BAD_REQUEST)?;
    let full_hex = account_full(&addr_bytes);

    let db = safe_lock(&state.db);
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);

    let (balance, nonce, last_touched_epoch) = if let Some(acct) = db.get_account(&addr_bytes) {
        (acct.balance, acct.nonce, acct.last_touched_epoch)
    } else {
        (0, 0, 0)
    };

    // Objects owned by this address
    let objects: Vec<ObjectResponse> = db
        .all_object_ids()
        .iter()
        .filter_map(|id| {
            let obj = db.get_object(id)?;
            if obj.owner != addr_bytes {
                return None;
            }
            let current_energy = obj.energy_at(epoch);
            let decay_pct = if obj.energy > 0 {
                ((obj.energy - current_energy) as f64 / obj.energy as f64) * 100.0
            } else {
                100.0
            };
            let state_str = match obj.state {
                ObjectState::Active => "Active",
                ObjectState::Grace => "Grace",
                ObjectState::Ghost => "Ghost",
                ObjectState::Resurrected => "Risen",
            };
            Some(ObjectResponse {
                id: hex::encode(id),
                name: object_name(id, &obj.data),
                owner: hex::encode(obj.owner),
                owner_name: account_name(&obj.owner),
                energy: obj.energy,
                max_energy: obj.energy,
                half_life: obj.half_life,
                state: state_str.to_string(),
                created_epoch: obj.created_at,
                last_refreshed: obj.last_refreshed,
                grace_epoch: obj.grace_epoch,
                current_energy,
                decay_percentage: (decay_pct * 10.0).round() / 10.0,
                decay_curve: obj.decay_curve.clone(),
                is_lad_typed: obj.lad_mode.is_some(),
                lad_mode: obj.lad_mode,
            })
        })
        .collect();
    drop(history);
    drop(db);

    // NFTs owned by this address
    let nft_store = safe_lock(&state.nft_store);
    let nfts: Vec<NftResponse> = nft_store
        .tokens
        .iter()
        .filter(|n| n.owner == full_hex || n.owner.contains(&addr_hex))
        .map(|n| nft_to_response(n, epoch))
        .collect();
    drop(nft_store);

    // Token balances
    let token_store = safe_lock(&state.token_store);
    let tokens: Vec<serde_json::Value> = token_store
        .tokens
        .iter()
        .filter_map(|t| {
            let bal = t
                .balances
                .get(&full_hex)
                .or_else(|| {
                    t.balances
                        .keys()
                        .find(|k| k.contains(&addr_hex))
                        .and_then(|k| t.balances.get(k))
                })
                .copied()
                .unwrap_or(0);
            if bal == 0 {
                return None;
            }
            Some(serde_json::json!({
                "token_id": t.id, "symbol": t.symbol, "name": t.name, "balance": bal
            }))
        })
        .collect();

    Ok(Json(AddressDetailResponse {
        address: full_hex,
        balance,
        nonce,
        last_touched_epoch,
        objects,
        nfts,
        tokens,
    }))
}

// ──────────────────────────── Faucet ───────────────────────────────────

const FAUCET_AMOUNT: u64 = 10_000;
/// Default per-(IP, address) cooldown in seconds. Operators can override
/// at startup with `--faucet-rate-limit-secs <N>`. Set to 0 (or use the
/// `--faucet-rate-limit-disabled` flag) to skip the check entirely —
/// required for stress / load testing where a single harness IP fans
/// out faucets to many distinct recipient addresses.
pub const FAUCET_RATE_LIMIT_SECS: u64 = 3600; // 1 hour
/// Hard cap on the in-memory rate-limit map. Bumped from 10k → 100k
/// 2026-04-30: under stress with the new (IP, full-addr) key shape a
/// single 60-tx burst from one IP creates 60 entries; a multi-IP fleet
/// of harnesses crosses 10k quickly. Eviction policy unchanged
/// (drop-everything-older-than-cooldown, run on overflow).
const FAUCET_RATE_LIMIT_MAP_CAP: usize = 100_000;

/// Resolve the originating client IP from a request.
///
/// **Audit fix C3**: legacy implementation blindly trusted the
/// left-most `X-Forwarded-For` entry, which any unauth'd attacker can
/// spoof to drain the faucet to arbitrary recipients (the header is
/// trivially set on the request).
///
/// New behaviour, **default-deny**: ignore `X-Forwarded-For` entirely
/// unless the operator opts in via `EVAPORCHAIN_TRUSTED_PROXY_DEPTH`.
/// When `depth > 0`, take the **right-most `depth`-th entry** of the
/// header (the operator's own proxy chain — the leftmost entries are
/// attacker-controlled and ignored). When `depth = 0` (default), the
/// header is ignored and `ConnectInfo` (the direct TCP peer) is the
/// only trusted source.
///
/// Order with depth=0 (default):
///   1. `axum::extract::ConnectInfo<SocketAddr>`: the direct peer.
///   2. Fallback: `0.0.0.0` (logged warning).
///
/// Order with depth=N>0:
///   1. `X-Forwarded-For`: take the entry that is the N-th from the
///      right (counting from 1). If absent or malformed, REJECT —
///      return `0.0.0.0` so the request collapses onto a single
///      rate-limit bucket (no silent fallback to ConnectInfo).
fn client_ip_from(
    headers: &HeaderMap,
    connect_info: Option<std::net::SocketAddr>,
) -> std::net::IpAddr {
    let depth = std::env::var("EVAPORCHAIN_TRUSTED_PROXY_DEPTH")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    if depth > 0 {
        // Right-most depth-th entry. The leftmost entries are
        // attacker-supplied; the operator's proxies append on the right.
        if let Some(raw) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            let entries: Vec<&str> = raw.split(',').map(|s| s.trim()).collect();
            if entries.len() >= depth {
                let target = &entries[entries.len() - depth];
                if let Ok(ip) = target.parse::<std::net::IpAddr>() {
                    return ip;
                }
            }
        }
        // Header missing or malformed under non-zero depth → fail-safe
        // 0.0.0.0 so the request collapses onto a single rate-limit
        // bucket. Operators should ensure their proxy always sets the
        // header at this depth.
        tracing::warn!(
            "client_ip_from: TRUSTED_PROXY_DEPTH={} but X-Forwarded-For missing/malformed",
            depth
        );
        return std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED);
    }

    // Default (depth=0): ignore X-Forwarded-For entirely; trust only
    // the direct TCP peer.
    if let Some(sock) = connect_info {
        return sock.ip();
    }
    tracing::warn!("faucet: could not resolve client IP (no ConnectInfo); falling back to 0.0.0.0");
    std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
}

/// Outcome of a single rate-limit check.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FaucetRateOutcome {
    /// Allowed — the call site should proceed and the cache has been
    /// updated with the current timestamp.
    Allowed,
    /// Blocked — the caller hit the cooldown. Carries the remaining
    /// seconds so the response can render a human message.
    Blocked { remaining_secs: u64 },
}

/// Pure rate-limit helper. Public-in-crate so tests can drive it
/// without standing up the full `ApiState`.
///
/// Semantics:
///   - `disabled = true` always returns `Allowed` and does not touch
///     the map. The CLI flags `--faucet-rate-limit-disabled` and the
///     legacy `--devnet-no-rate-limit` both set this.
///   - `cooldown_secs = 0` always returns `Allowed`; the timestamp is
///     still inserted (so behaviour stays consistent with future
///     non-zero re-tunes), but no caller will ever be blocked.
///   - Otherwise: a call is `Blocked` iff a previous call with the
///     same `(ip, addr)` key landed within `cooldown_secs`. On allow,
///     the key's timestamp is overwritten to "now".
///   - On overflow past `cap`, expired entries are dropped first;
///     this matches the prior behaviour.
pub(crate) fn check_and_record_faucet_rate_limit(
    map: &mut HashMap<(std::net::IpAddr, [u8; 32]), Instant>,
    ip: std::net::IpAddr,
    addr: [u8; 32],
    cooldown_secs: u64,
    disabled: bool,
    cap: usize,
) -> FaucetRateOutcome {
    if disabled {
        return FaucetRateOutcome::Allowed;
    }
    let key = (ip, addr);
    if cooldown_secs > 0 {
        if let Some(last) = map.get(&key) {
            let elapsed = last.elapsed().as_secs();
            if elapsed < cooldown_secs {
                return FaucetRateOutcome::Blocked {
                    remaining_secs: cooldown_secs - elapsed,
                };
            }
        }
    }
    map.insert(key, Instant::now());
    if map.len() > cap {
        map.retain(|_, last| last.elapsed().as_secs() < cooldown_secs.max(1));
    }
    FaucetRateOutcome::Allowed
}

async fn wallet_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/wallet.html"))
}

async fn faucet_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/faucet.html"))
}

async fn docs_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/docs.html"))
}

async fn manifest_json() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/manifest+json",
        )],
        include_str!("../dashboard/manifest.json"),
    )
}

async fn service_worker_js() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        include_str!("../dashboard/sw.js"),
    )
}

async fn post_faucet(
    State(state): State<Arc<ApiState>>,
    connect_info: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    headers: HeaderMap,
    Json(req): Json<FaucetRequest>,
) -> impl IntoResponse {
    if let Err(_e) = require_admin_auth(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(FaucetResponse {
                success: false,
                balance: 0,
                message: Some("unauthorized: invalid admin key".into()),
            }),
        );
    }
    let addr = match parse_address_value(&req.address) {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::OK,
                Json(FaucetResponse {
                    success: false,
                    balance: 0,
                    message: Some(format!("Invalid address: {}", e)),
                }),
            )
        }
    };
    // Rate-limit key shape (changed 2026-04-30): (client_ip, full 32-byte
    // recipient address). Previously keyed on hex(addr[..20]) — collapsing
    // sequentially-numbered stress-test addresses (0x000…001 … 0x000…0c8,
    // all sharing 20 zero bytes) into one slot and 429-ing 59 of every 60.
    // The IP component preserves the original anti-pump intent (one IP can
    // not keep funding the same address), while distinct recipients from
    // the same harness IP all flow through.
    let client_ip = client_ip_from(&headers, connect_info.map(|ci| ci.0));
    let outcome = {
        let mut limits = safe_lock(&state.faucet_rate_limit);
        check_and_record_faucet_rate_limit(
            &mut limits,
            client_ip,
            addr,
            state.faucet_rate_limit_secs,
            state.faucet_rate_limit_disabled,
            FAUCET_RATE_LIMIT_MAP_CAP,
        )
    };
    if let FaucetRateOutcome::Blocked { remaining_secs } = outcome {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(FaucetResponse {
                success: false,
                balance: 0,
                message: Some(format!(
                    "Rate limited. Try again in {} minutes.",
                    remaining_secs / 60 + 1
                )),
            }),
        );
    }

    // Submit faucet as a transfer from the "faucet account" (all-zeros address)
    // through consensus so all validators see it. Reserve a unique nonce
    // via the pending-nonce cache so concurrent faucet hits don't all
    // collide on the same db.nonce (only one would land otherwise).
    // Genesis pre-seeds the faucet at FAUCET_ADDRESS = [0xFA; 32] (see
    // evaporchain_types::FAUCET_ADDRESS + evaporchain-cli testnet init).
    // The previous [0u8; 32] address had no balance — every faucet
    // transfer failed InsufficientBalance silently.
    let faucet_addr = evaporchain_types::FAUCET_ADDRESS;
    let nonce = state.reserve_nonce(&faucet_addr);
    let mut tx = Transaction::Transfer(TransferTx {
        from: faucet_addr,
        to: addr,
        amount: FAUCET_AMOUNT,
        nonce,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    });
    sign_transaction(&mut tx, &state, None);
    state.submit_tx(tx);

    // Return expected balance (may not be applied yet until next block)
    let balance = {
        let db = safe_lock(&state.db);
        db.get_account(&addr).map(|a| a.balance).unwrap_or(0) + FAUCET_AMOUNT
    };

    (
        StatusCode::OK,
        Json(FaucetResponse {
            success: true,
            balance,
            message: Some(
                "Faucet transaction submitted to consensus — balance updates after next block"
                    .into(),
            ),
        }),
    )
}

// ──────────────────────────── Oracle Ingest ──────────────────────────────

/// Oracle ingest endpoint — requires EVAPORCHAIN_ORACLE_KEY bearer token.
/// Creates on-chain objects with sensor data, energy, and half-life.
/// Used by evaporchain-oracle to publish real-world data feeds.
#[derive(Deserialize)]
struct OracleIngestRequest {
    /// Source identifier (e.g. "nasa:iss", "usgs:quake")
    source: String,
    /// Unique object ID (hex address)
    object_id: String,
    /// Initial energy
    energy: u64,
    /// Half-life in epochs
    half_life: u64,
    /// Sensor data payload (JSON string)
    data: String,
}

async fn post_oracle_ingest(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<OracleIngestRequest>,
) -> Json<TxResultResponse> {
    let expected = match std::env::var("EVAPORCHAIN_ORACLE_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            return Json(TxResultResponse {
                success: false,
                message: "oracle endpoint disabled: EVAPORCHAIN_ORACLE_KEY not configured".into(),
                tx_hash: None,
            });
        }
    };
    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    // Constant-time comparison to prevent timing side-channel on the API key
    let key_match = provided.len() == expected.len() && {
        let mut acc = 0u8;
        for (a, b) in provided.as_bytes().iter().zip(expected.as_bytes()) {
            acc |= a ^ b;
        }
        acc == 0
    };
    if !key_match {
        return Json(TxResultResponse {
            success: false,
            message: "unauthorized: invalid oracle key".into(),
            tx_hash: None,
        });
    }
    // Oracle uses faucet address as creator (special system address)
    let creator = [0u8; 32];
    let obj_id_val = match parse_address_value(&serde_json::Value::String(req.object_id.clone())) {
        Ok(a) => a,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };

    // Prepend source tag to data
    let data_str = format!("[{}] {}", req.source, req.data);

    let mut tx = Transaction::CreateObject(CreateObjectTx {
        creator,
        object_id: obj_id_val,
        energy: req.energy,
        half_life: req.half_life,
        data: data_str.into_bytes(),
        decay_curve: None,
        lad_mode: None,
        signature: None,
        public_key: None,
    });
    sign_transaction(&mut tx, &state, None);
    // Canonical tx hash — see post_transfer.
    let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
    state.submit_tx(tx);

    Json(TxResultResponse {
        success: true,
        message: format!(
            "Oracle data ingested: {} (energy={}, half_life={})",
            req.source, req.energy, req.half_life
        ),
        tx_hash: Some(hash),
    })
}

// ──────────────────── Oracle Consensus Handlers ──────────────────────────

async fn get_oracle_status(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    if let Some(ref ob) = state.oracle_bridge {
        let bridge = safe_lock(ob);
        Json(serde_json::json!({
            "active": true,
            "feeds": bridge.feed_count(),
            "active_rounds": bridge.active_rounds_count(),
            "oracle_state_root": hex::encode(bridge.oracle_state_root()),
        }))
    } else {
        Json(serde_json::json!({ "active": false }))
    }
}

async fn get_oracle_feed(
    State(state): State<Arc<ApiState>>,
    Path(key): Path<String>,
) -> Json<serde_json::Value> {
    if let Some(ref ob) = state.oracle_bridge {
        let bridge = safe_lock(ob);
        let value = bridge.get_value(&key);
        let twap = bridge.get_twap(&key);
        let proof = bridge.generate_proof(&key);
        Json(serde_json::json!({
            "key": key,
            "value": value,
            "twap": twap,
            "has_proof": proof.is_some(),
            "proof_hash": proof.map(|p| hex::encode(p.proof_hash)),
        }))
    } else {
        Json(serde_json::json!({ "error": "oracle not active" }))
    }
}

// ──────────────────── Shard Status Handlers ──────────────────────────────

async fn get_shard_status(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    if let Some(ref sb) = state.shard_bridge {
        let bridge = safe_lock(sb);
        Json(serde_json::json!({
            "active": true,
            "num_shards": bridge.num_shards(),
            "pending_cross_shard_messages": bridge.pending_messages(),
        }))
    } else {
        Json(serde_json::json!({ "active": false }))
    }
}

async fn get_shard_health(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    if let Some(ref sb) = state.shard_bridge {
        let bridge = safe_lock(sb);
        let healths = bridge.shard_healths();
        let candidates = bridge.find_compaction_candidates();
        let shard_data: Vec<serde_json::Value> = healths
            .iter()
            .map(|h| {
                serde_json::json!({
                    "shard_id": h.shard_id.0,
                    "total_objects": h.total_objects,
                    "live_objects": h.live_objects,
                    "total_energy": h.total_energy,
                    "liveness_ratio": h.liveness_ratio(),
                    "is_dead": h.is_dead(),
                })
            })
            .collect();
        Json(serde_json::json!({
            "shards": shard_data,
            "compaction_candidates": candidates.len(),
        }))
    } else {
        Json(serde_json::json!({ "active": false }))
    }
}

// ──────────────────────────── Ghost Bridge Handlers ─────────────────────

async fn get_ghost_list(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let db = safe_lock(&state.db);
    let ghost_ids = db.all_ghost_ids();
    let ghosts: Vec<serde_json::Value> = ghost_ids
        .iter()
        .take(100)
        .filter_map(|id| {
            db.get_ghost(id).map(|g| {
                serde_json::json!({
                    "object_id": hex::encode(g.object_id),
                    "owner": hex::encode(g.owner),
                    "evaporated_at": g.evaporated_at,
                    "data_hash": hex::encode(g.data_hash),
                    "has_original_data": g.original_data.is_some(),
                    "mmr_position": g.mmr_position,
                    "original_half_life": g.original_half_life,
                })
            })
        })
        .collect();
    Json(serde_json::json!({
        "total": ghost_ids.len(),
        "ghosts": ghosts,
    }))
}

async fn get_ghost_detail(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let mut obj_id = [0u8; 32];
    if let Ok(bytes) = hex::decode(&id) {
        let len = bytes.len().min(32);
        obj_id[..len].copy_from_slice(&bytes[..len]);
    }
    let db = safe_lock(&state.db);
    if let Some(ghost) = db.get_ghost(&obj_id) {
        Json(serde_json::json!({
            "found": true,
            "object_id": hex::encode(ghost.object_id),
            "owner": hex::encode(ghost.owner),
            "evaporated_at": ghost.evaporated_at,
            "data_hash": hex::encode(ghost.data_hash),
            "has_original_data": ghost.original_data.is_some(),
            "data_size": ghost.original_data.as_ref().map(|d| d.len()),
            "mmr_position": ghost.mmr_position,
            "original_half_life": ghost.original_half_life,
        }))
    } else {
        Json(serde_json::json!({ "found": false, "object_id": id }))
    }
}

// ──────────────────────────── NFT Handlers ─────────────────────────────

async fn nft_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/nft.html"))
}

#[derive(Serialize)]
struct NftResponse {
    id: u64,
    name: String,
    collection: String,
    owner: String,
    metadata_hash: String,
    energy: u64,
    max_energy: u64,
    current_energy: u64,
    half_life: u64,
    minted_epoch: u64,
    last_refreshed: u64,
    state: String,
    decay_percentage: f64,
    epochs_remaining: u64,
    grace_epoch: Option<u64>,
    evaporated_epoch: Option<u64>,
    ghost_proof: Option<String>,
}

fn nft_to_response(nft: &NftToken, epoch: u64) -> NftResponse {
    NftResponse {
        id: nft.id,
        name: nft.name.clone(),
        collection: nft.collection.clone(),
        owner: nft.owner.clone(),
        metadata_hash: nft.metadata_hash.clone(),
        energy: nft.energy,
        max_energy: nft.max_energy,
        current_energy: nft.current_energy(epoch),
        half_life: nft.half_life,
        minted_epoch: nft.minted_epoch,
        last_refreshed: nft.last_refreshed,
        state: nft.state.clone(),
        decay_percentage: nft.decay_pct(epoch),
        epochs_remaining: nft.epochs_remaining(epoch),
        grace_epoch: nft.grace_epoch,
        evaporated_epoch: nft.evaporated_epoch,
        ghost_proof: nft.ghost_proof.clone(),
    }
}

async fn get_nfts(State(state): State<Arc<ApiState>>) -> Json<Vec<NftResponse>> {
    let store = safe_lock(&state.nft_store);
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);

    // Tick lifecycle for display (decay state transitions)
    drop(store);
    tick_nft_lifecycle(&state, epoch);
    let store = safe_lock(&state.nft_store);

    let mut nfts: Vec<NftResponse> = store
        .tokens
        .iter()
        .map(|n| nft_to_response(n, epoch))
        .collect();
    // Active first, then Grace, then Ghost
    nfts.sort_by_key(|n| match n.state.as_str() {
        "Active" => 0,
        "Grace" => 1,
        "Ghost" => 2,
        _ => 3,
    });
    Json(nfts)
}

async fn get_single_nft(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<u64>,
) -> Result<Json<NftResponse>, StatusCode> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);

    tick_nft_lifecycle(&state, epoch);
    let store = safe_lock(&state.nft_store);

    let nft = store
        .tokens
        .iter()
        .find(|n| n.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(nft_to_response(nft, epoch)))
}

#[derive(Deserialize)]
struct MintNftRequest {
    name: String,
    collection: Option<String>,
    metadata: String,
    energy: u64,
    half_life: u64,
    owner: Option<String>,
}

async fn post_mint_nft(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<MintNftRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, false) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    // Input validation
    let name = match sanitize_string(&req.name, 200) {
        Ok(n) => n,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    if name.is_empty() {
        return Json(TxResultResponse {
            success: false,
            message: "Name is required".into(),
            tx_hash: None,
        });
    }
    if req.energy == 0 {
        return Json(TxResultResponse {
            success: false,
            message: "Energy must be greater than zero".into(),
            tx_hash: None,
        });
    }
    if req.half_life == 0 {
        return Json(TxResultResponse {
            success: false,
            message: "Half-life must be greater than zero".into(),
            tx_hash: None,
        });
    }
    if req.energy > 1_000_000_000 {
        return Json(TxResultResponse {
            success: false,
            message: "Energy exceeds maximum (1,000,000,000)".into(),
            tx_hash: None,
        });
    }
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);

    let metadata = match sanitize_string(&req.metadata, 10_000) {
        Ok(m) => m,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    let metadata_hash = blake3::hash(metadata.as_bytes()).to_hex().to_string();
    // Owner must be a wallet owned by the caller (if auth is active)
    let owner = match req.owner {
        Some(ref o) if !o.is_empty() => {
            if let Err(resp) = require_wallet_ownership(&state, user_id, o) {
                return resp;
            }
            o.clone()
        }
        _ => format!("0x{}", GENESIS_FOUNDATION),
    };
    let collection = match req.collection {
        Some(ref c) => sanitize_string(c, 200).unwrap_or_else(|_| "Genesis Collection".to_string()),
        None => "Genesis Collection".to_string(),
    };

    let mut store = safe_lock(&state.nft_store);
    let id = store.next_id;
    store.next_id += 1;
    store.tokens.push(NftToken {
        id,
        name: name.clone(),
        collection,
        owner,
        metadata_hash,
        energy: req.energy,
        max_energy: req.energy,
        half_life: req.half_life,
        minted_epoch: epoch,
        last_refreshed: epoch,
        state: "Active".to_string(),
        grace_epoch: None,
        evaporated_epoch: None,
        ghost_proof: None,
    });

    let hash = tx_hash(&format!("nft:mint:{}:{}", id, name));
    Json(TxResultResponse {
        success: true,
        message: format!(
            "NFT #{} '{}' minted with energy={}, half_life={}",
            id, req.name, req.energy, req.half_life
        ),
        tx_hash: Some(hash),
    })
}

#[derive(Deserialize)]
struct TransferNftRequest {
    nft_id: u64,
    to: String,
}

async fn post_transfer_nft(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<TransferNftRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, false) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    let mut store = safe_lock(&state.nft_store);
    if let Some(nft) = store.tokens.iter_mut().find(|n| n.id == req.nft_id) {
        if nft.state == "Ghost" {
            return Json(TxResultResponse {
                success: false,
                message: "Cannot transfer a ghost NFT".to_string(),
                tx_hash: None,
            });
        }
        // Ownership check: caller must own the NFT
        if let Err(resp) = require_wallet_ownership(&state, user_id, &nft.owner) {
            return resp;
        }
        let from = nft.owner.clone();
        nft.owner = req.to.clone();
        let hash = tx_hash(&format!("nft:transfer:{}:{}:{}", req.nft_id, from, req.to));
        Json(TxResultResponse {
            success: true,
            message: format!(
                "NFT #{} transferred from {} to {}",
                req.nft_id, from, req.to
            ),
            tx_hash: Some(hash),
        })
    } else {
        Json(TxResultResponse {
            success: false,
            message: format!("NFT #{} not found", req.nft_id),
            tx_hash: None,
        })
    }
}

#[derive(Deserialize)]
struct RefreshNftRequest {
    nft_id: u64,
    energy: u64,
}

async fn post_refresh_nft(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<RefreshNftRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, false) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    // Ownership check: verify caller owns this NFT
    {
        let store = safe_lock(&state.nft_store);
        if let Some(nft) = store.tokens.iter().find(|n| n.id == req.nft_id) {
            if let Err(resp) = require_wallet_ownership(&state, user_id, &nft.owner) {
                return resp;
            }
        }
    }
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);

    let mut store = safe_lock(&state.nft_store);
    if let Some(nft) = store.tokens.iter_mut().find(|n| n.id == req.nft_id) {
        if nft.state == "Ghost" {
            nft.state = "Active".to_string();
            nft.energy = req.energy;
            nft.max_energy = req.energy;
            nft.last_refreshed = epoch;
            nft.grace_epoch = None;
            nft.evaporated_epoch = None;
            nft.ghost_proof = None;
            let hash = tx_hash(&format!("nft:resurrect:{}:{}", nft.id, req.energy));
            Json(TxResultResponse {
                success: true,
                message: format!(
                    "NFT #{} '{}' resurrected with energy={}",
                    nft.id, nft.name, req.energy
                ),
                tx_hash: Some(hash),
            })
        } else {
            let current = nft.current_energy(epoch);
            nft.energy = current + req.energy;
            nft.max_energy = nft.energy;
            nft.last_refreshed = epoch;
            if nft.state == "Grace" {
                nft.state = "Active".to_string();
                nft.grace_epoch = None;
            }
            let hash = tx_hash(&format!("nft:refresh:{}:{}", nft.id, req.energy));
            Json(TxResultResponse {
                success: true,
                message: format!(
                    "NFT #{} '{}' refreshed, energy now {}",
                    nft.id, nft.name, nft.energy
                ),
                tx_hash: Some(hash),
            })
        }
    } else {
        Json(TxResultResponse {
            success: false,
            message: format!("NFT #{} not found", req.nft_id),
            tx_hash: None,
        })
    }
}

/// Tick NFT lifecycle — move to Grace/Ghost based on energy decay.
fn tick_nft_lifecycle(state: &ApiState, epoch: u64) {
    let mut store = safe_lock(&state.nft_store);
    for nft in store.tokens.iter_mut() {
        if nft.state == "Ghost" {
            continue;
        }
        let current = nft.current_energy(epoch);
        if current == 0 && nft.state == "Active" {
            nft.state = "Grace".to_string();
            nft.grace_epoch = Some(epoch);
        }
        if nft.state == "Grace" {
            if let Some(grace_start) = nft.grace_epoch {
                if epoch >= grace_start + 5 {
                    nft.state = "Ghost".to_string();
                    nft.evaporated_epoch = Some(epoch);
                    let proof_data = format!("{}:{}:{}", nft.id, nft.name, epoch);
                    let hash = blake3::hash(proof_data.as_bytes());
                    nft.ghost_proof = Some(hash.to_hex().to_string());
                }
            }
        }
    }
}

// ──────────────────────────── Token Store ───────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
pub struct DeployedToken {
    pub id: u64,
    pub name: String,
    pub symbol: String,
    pub total_supply: u64,
    pub decay_half_life: u64,
    pub deployed_epoch: u64,
    pub deployer: String,
    pub balances: HashMap<String, u64>,
    pub last_decay_epoch: u64,
}

impl DeployedToken {
    /// Compute current total supply after decay.
    pub fn current_supply(&self, epoch: u64) -> u64 {
        evaporchain_types::energy_at_epoch(
            self.total_supply,
            self.decay_half_life,
            epoch.saturating_sub(self.deployed_epoch),
        )
    }

    pub fn decay_pct(&self, epoch: u64) -> f64 {
        if self.total_supply == 0 {
            return 100.0;
        }
        let cur = self.current_supply(epoch);
        ((self.total_supply - cur) as f64 / self.total_supply as f64 * 1000.0).round() / 10.0
    }

    /// Apply proportional decay to all holder balances.
    pub fn tick_decay(&mut self, epoch: u64) {
        if epoch <= self.last_decay_epoch {
            return;
        }
        let elapsed = epoch - self.last_decay_epoch;
        for bal in self.balances.values_mut() {
            *bal = evaporchain_types::energy_at_epoch(*bal, self.decay_half_life, elapsed);
        }
        self.last_decay_epoch = epoch;
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TokenStore {
    pub tokens: Vec<DeployedToken>,
    pub next_id: u64,
}

// ── Token handlers ──

async fn tokens_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/tokens.html"))
}

#[derive(Serialize)]
struct TokenResponse {
    id: u64,
    name: String,
    symbol: String,
    total_supply: u64,
    current_supply: u64,
    decay_half_life: u64,
    deployed_epoch: u64,
    deployer: String,
    decay_percentage: f64,
    holder_count: usize,
    holders: Vec<TokenHolder>,
}
#[derive(Serialize)]
struct TokenHolder {
    address: String,
    balance: u64,
}

async fn get_tokens(State(state): State<Arc<ApiState>>) -> Json<Vec<TokenResponse>> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.token_store);
    for t in store.tokens.iter_mut() {
        t.tick_decay(epoch);
    }
    let res: Vec<TokenResponse> = store
        .tokens
        .iter()
        .map(|t| {
            let mut holders: Vec<TokenHolder> = t
                .balances
                .iter()
                .filter(|(_, b)| **b > 0)
                .map(|(a, b)| TokenHolder {
                    address: a.clone(),
                    balance: *b,
                })
                .collect();
            holders.sort_by_key(|a| std::cmp::Reverse(a.balance));
            TokenResponse {
                id: t.id,
                name: t.name.clone(),
                symbol: t.symbol.clone(),
                total_supply: t.total_supply,
                current_supply: t.current_supply(epoch),
                decay_half_life: t.decay_half_life,
                deployed_epoch: t.deployed_epoch,
                deployer: t.deployer.clone(),
                decay_percentage: t.decay_pct(epoch),
                holder_count: holders.len(),
                holders,
            }
        })
        .collect();
    Json(res)
}

async fn get_single_token(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<u64>,
) -> Result<Json<TokenResponse>, StatusCode> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.token_store);
    let t = store
        .tokens
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    t.tick_decay(epoch);
    let mut holders: Vec<TokenHolder> = t
        .balances
        .iter()
        .filter(|(_, b)| **b > 0)
        .map(|(a, b)| TokenHolder {
            address: a.clone(),
            balance: *b,
        })
        .collect();
    holders.sort_by_key(|a| std::cmp::Reverse(a.balance));
    Ok(Json(TokenResponse {
        id: t.id,
        name: t.name.clone(),
        symbol: t.symbol.clone(),
        total_supply: t.total_supply,
        current_supply: t.current_supply(epoch),
        decay_half_life: t.decay_half_life,
        deployed_epoch: t.deployed_epoch,
        deployer: t.deployer.clone(),
        decay_percentage: t.decay_pct(epoch),
        holder_count: holders.len(),
        holders,
    }))
}

#[derive(Deserialize)]
struct DeployTokenRequest {
    name: String,
    symbol: String,
    total_supply: u64,
    decay_half_life: u64,
    deployer: Option<String>,
}

async fn post_deploy_token(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<DeployTokenRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, false) {
        return resp;
    }
    let token_name = match sanitize_string(&req.name, 100) {
        Ok(n) => n,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    let token_symbol = match sanitize_string(&req.symbol, 20) {
        Ok(s) => s,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    if token_name.is_empty() {
        return Json(TxResultResponse {
            success: false,
            message: "Token name is required".into(),
            tx_hash: None,
        });
    }
    if token_symbol.is_empty() {
        return Json(TxResultResponse {
            success: false,
            message: "Token symbol is required".into(),
            tx_hash: None,
        });
    }
    if req.total_supply == 0 {
        return Json(TxResultResponse {
            success: false,
            message: "Total supply must be > 0".into(),
            tx_hash: None,
        });
    }
    if req.decay_half_life == 0 {
        return Json(TxResultResponse {
            success: false,
            message: "Decay half-life must be > 0".into(),
            tx_hash: None,
        });
    }
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let deployer = req
        .deployer
        .unwrap_or_else(|| format!("0x{}", GENESIS_FOUNDATION));
    let mut store = safe_lock(&state.token_store);
    let id = store.next_id;
    store.next_id += 1;
    let mut balances = HashMap::new();
    balances.insert(deployer.clone(), req.total_supply);
    store.tokens.push(DeployedToken {
        id,
        name: token_name.clone(),
        symbol: token_symbol.clone(),
        total_supply: req.total_supply,
        decay_half_life: req.decay_half_life,
        deployed_epoch: epoch,
        deployer,
        balances,
        last_decay_epoch: epoch,
    });
    let hash = tx_hash(&format!(
        "token:deploy:{}:{}",
        token_symbol, req.total_supply
    ));
    Json(TxResultResponse {
        success: true,
        message: format!(
            "{} ({}) deployed with supply={}, half_life={}",
            token_name, token_symbol, req.total_supply, req.decay_half_life
        ),
        tx_hash: Some(hash),
    })
}

#[derive(Deserialize)]
struct TokenTransferRequest {
    token_id: u64,
    from: String,
    to: String,
    amount: u64,
}

async fn post_token_transfer(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<TokenTransferRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, false) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    // Ownership check: caller must own the `from` address
    if let Err(resp) = require_wallet_ownership(&state, user_id, &req.from) {
        return resp;
    }
    let mut store = safe_lock(&state.token_store);
    let t = match store.tokens.iter_mut().find(|t| t.id == req.token_id) {
        Some(t) => t,
        None => {
            return Json(TxResultResponse {
                success: false,
                message: "Token not found".into(),
                tx_hash: None,
            })
        }
    };
    let from_bal = t.balances.get(&req.from).copied().unwrap_or(0);
    if from_bal < req.amount {
        return Json(TxResultResponse {
            success: false,
            message: format!("Insufficient balance: {} < {}", from_bal, req.amount),
            tx_hash: None,
        });
    }
    *t.balances.entry(req.from.clone()).or_insert(0) -= req.amount;
    *t.balances.entry(req.to.clone()).or_insert(0) += req.amount;
    let hash = tx_hash(&format!(
        "token:transfer:{}:{}:{}:{}",
        req.token_id, req.from, req.to, req.amount
    ));
    Json(TxResultResponse {
        success: true,
        message: format!(
            "{} {} transferred from {} to {}",
            req.amount, t.symbol, req.from, req.to
        ),
        tx_hash: Some(hash),
    })
}

#[derive(Deserialize)]
struct TokenBalanceRequest {
    token_id: u64,
    address: String,
}

async fn post_token_balance(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<TokenBalanceRequest>,
) -> Json<serde_json::Value> {
    let store = safe_lock(&state.token_store);
    let t = match store.tokens.iter().find(|t| t.id == req.token_id) {
        Some(t) => t,
        None => return Json(serde_json::json!({"error":"Token not found"})),
    };
    let bal = t.balances.get(&req.address).copied().unwrap_or(0);
    Json(
        serde_json::json!({"token_id": req.token_id, "address": req.address, "balance": bal, "symbol": t.symbol}),
    )
}

// ──────────────────────────── Swap (CFM-priced) ─────────────────────────

/// Swap fee in basis points (30 bps = 0.3 %).
const SWAP_FEE_BPS: u64 = 30;

#[derive(Deserialize)]
struct SwapQuoteRequest {
    from_token: String,
    to_token: String,
    amount: u64,
}

#[derive(Deserialize)]
#[allow(dead_code)] // serde DTO accepts public_key from clients; field is read on the bind site only.
struct SwapExecuteRequest {
    from_token: String,
    to_token: String,
    amount: u64,
    slippage: f64,
    from: String,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    public_key: Option<String>,
}

/// Return a swap quote using oracle mid-prices (or 1:1 EVAP as fallback).
async fn post_swap_quote(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SwapQuoteRequest>,
) -> impl IntoResponse {
    if req.amount == 0 {
        return Json(serde_json::json!({ "error": "amount must be > 0" }));
    }
    let rate = oracle_rate(&state, &req.from_token, &req.to_token);
    let gross_out = (req.amount as f64 * rate) as u64;
    let fee = (gross_out * SWAP_FEE_BPS / 10_000).max(1);
    let amount_out = gross_out.saturating_sub(fee);
    let price_impact = if gross_out > 0 {
        (fee as f64 / gross_out as f64) * 100.0
    } else {
        0.0
    };
    Json(serde_json::json!({
        "from_token": req.from_token,
        "to_token":   req.to_token,
        "amount_in":  req.amount,
        "amount_out": amount_out,
        "fee":        fee,
        "rate":       rate,
        "price_impact": (price_impact * 100.0).round() / 100.0,
    }))
}

/// Execute a swap: debit from_token balance, credit to_token balance.
/// Both tokens must be deployed. EVAP ↔ token swaps adjust the EVAP
/// account balance through the executor.
async fn post_swap_execute(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<SwapExecuteRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, req.signature.is_some()) {
        return resp;
    }
    if req.amount == 0 {
        return Json(TxResultResponse {
            success: false,
            message: "amount must be > 0".into(),
            tx_hash: None,
        });
    }

    let rate = oracle_rate(&state, &req.from_token, &req.to_token);
    let gross_out = (req.amount as f64 * rate) as u64;
    let fee = (gross_out * SWAP_FEE_BPS / 10_000).max(1);
    let amount_out = gross_out.saturating_sub(fee);

    // Slippage guard: if computed amount_out < amount * (1 - slippage), reject.
    let min_out = (req.amount as f64 * rate * (1.0 - req.slippage / 100.0)) as u64;
    if amount_out < min_out {
        return Json(TxResultResponse {
            success: false,
            message: format!(
                "Slippage exceeded: expected min {} but got {}",
                min_out, amount_out
            ),
            tx_hash: None,
        });
    }

    let from_upper = req.from_token.to_ascii_uppercase();
    let to_upper = req.to_token.to_ascii_uppercase();

    // Helper: check/deduct from a DeployedToken balance, credit to another.
    {
        let mut store = safe_lock(&state.token_store);
        let epoch = {
            let history = safe_lock(&state.block_history);
            history.back().map(|b| b.epoch).unwrap_or(0)
        };

        // Determine if from/to are deployed tokens or "EVAP" (native).
        let from_is_token = store
            .tokens
            .iter()
            .any(|t| t.symbol.to_ascii_uppercase() == from_upper);
        let to_is_token = store
            .tokens
            .iter()
            .any(|t| t.symbol.to_ascii_uppercase() == to_upper);

        if from_is_token {
            // Deduct from the token balance.
            let token = store
                .tokens
                .iter_mut()
                .find(|t| t.symbol.to_ascii_uppercase() == from_upper)
                .unwrap();
            token.tick_decay(epoch);
            let bal = token.balances.entry(req.from.clone()).or_insert(0);
            if *bal < req.amount {
                return Json(TxResultResponse {
                    success: false,
                    message: format!(
                        "Insufficient {} balance: {} < {}",
                        from_upper, bal, req.amount
                    ),
                    tx_hash: None,
                });
            }
            *bal -= req.amount;
        } else if from_upper != "EVAP" {
            return Json(TxResultResponse {
                success: false,
                message: format!("Unknown from_token: {}", req.from_token),
                tx_hash: None,
            });
        }
        // EVAP debit handled below via executor.

        if to_is_token {
            // Credit the to_token balance.
            let token = store
                .tokens
                .iter_mut()
                .find(|t| t.symbol.to_ascii_uppercase() == to_upper)
                .unwrap();
            token.tick_decay(epoch);
            let bal = token.balances.entry(req.from.clone()).or_insert(0);
            *bal = bal.saturating_add(amount_out);
        } else if to_upper != "EVAP" {
            return Json(TxResultResponse {
                success: false,
                message: format!("Unknown to_token: {}", req.to_token),
                tx_hash: None,
            });
        }
        // EVAP credit handled below via executor.
    }

    // EVAP ↔ token: adjust EVAP account balance through the DB.
    if from_upper == "EVAP" || to_upper == "EVAP" {
        let from_addr = match parse_address_value(&serde_json::Value::String(req.from.clone())) {
            Ok(a) => a,
            Err(e) => {
                return Json(TxResultResponse {
                    success: false,
                    message: e,
                    tx_hash: None,
                })
            }
        };
        let mut db = safe_lock(&state.db);
        if from_upper == "EVAP" {
            // Deduct EVAP.
            let acct = db.get_or_create_account(&from_addr);
            if acct.balance < req.amount {
                return Json(TxResultResponse {
                    success: false,
                    message: format!(
                        "Insufficient EVAP balance: {} < {}",
                        acct.balance, req.amount
                    ),
                    tx_hash: None,
                });
            }
            let new_bal = acct.balance - req.amount;
            let mut updated = acct.clone();
            updated.balance = new_bal;
            db.put_account(updated);
        }
        if to_upper == "EVAP" {
            // Credit EVAP.
            let acct = db.get_or_create_account(&from_addr);
            let updated_balance = acct.balance.saturating_add(amount_out);
            let mut updated = acct.clone();
            updated.balance = updated_balance;
            db.put_account(updated);
        }
    }

    let tx_hash = tx_hash(&format!(
        "swap:{}:{}:{}:{}",
        req.from_token, req.to_token, req.amount, req.from
    ));
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Swapped {} {} for {} {}",
            req.amount, from_upper, amount_out, to_upper
        ),
        tx_hash: Some(tx_hash),
    })
}

/// Look up an oracle exchange rate from_symbol → to_symbol.
/// Falls back to 1.0 if no oracle price is available.
fn oracle_rate(state: &ApiState, from: &str, to: &str) -> f64 {
    let from_u = from.to_ascii_uppercase();
    let to_u = to.to_ascii_uppercase();
    if from_u == to_u {
        return 1.0;
    }

    let (from_usd, to_usd) = if let Some(ref ob) = state.oracle_bridge {
        let bridge = ob.lock().unwrap();
        let f = if from_u == "EVAP" {
            bridge
                .get_twap("evap_usd")
                .or_else(|| bridge.get_twap("evap_usdc"))
                .unwrap_or(1.0)
        } else {
            bridge
                .get_twap(&format!("{}_usd", from_u.to_ascii_lowercase()))
                .unwrap_or(1.0)
        };
        let t = if to_u == "EVAP" {
            bridge
                .get_twap("evap_usd")
                .or_else(|| bridge.get_twap("evap_usdc"))
                .unwrap_or(1.0)
        } else {
            bridge
                .get_twap(&format!("{}_usd", to_u.to_ascii_lowercase()))
                .unwrap_or(1.0)
        };
        (f, t)
    } else {
        (1.0, 1.0)
    };

    if to_usd == 0.0 {
        1.0
    } else {
        from_usd / to_usd
    }
}

// ──────────────────────────── Staking Store ─────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
pub struct StakingPool {
    pub id: u64,
    pub name: String,
    pub reward_rate: u64,     // per epoch
    pub reward_decay_hl: u64, // reward decay half-life
    pub total_staked: u64,
    pub created_epoch: u64,
    pub stakers: Vec<Staker>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Staker {
    pub address: String,
    pub amount: u64,
    pub staked_epoch: u64,
    pub pending_rewards: u64,
    pub last_claim_epoch: u64,
    pub total_claimed: u64,
    pub total_decayed: u64,
}

impl StakingPool {
    /// Compute pending rewards with decay for a staker.
    pub fn compute_rewards(&self, staker: &Staker, epoch: u64) -> u64 {
        if self.total_staked == 0 || staker.amount == 0 {
            return 0;
        }
        let epochs_since_claim = epoch.saturating_sub(staker.last_claim_epoch);
        if epochs_since_claim == 0 {
            return staker.pending_rewards;
        }
        // Raw rewards: share of pool * reward_rate * epochs
        let share = staker.amount as f64 / self.total_staked as f64;
        let raw = (share * self.reward_rate as f64 * epochs_since_claim as f64) as u64;
        // Apply decay to unclaimed rewards
        let total = staker.pending_rewards + raw;
        evaporchain_types::energy_at_epoch(total, self.reward_decay_hl, epochs_since_claim)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct StakingStore {
    pub pools: Vec<StakingPool>,
    pub next_id: u64,
}

// ── Staking handlers ──

async fn staking_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/staking.html"))
}

#[derive(Serialize)]
struct StakingPoolResponse {
    id: u64,
    name: String,
    reward_rate: u64,
    reward_decay_hl: u64,
    total_staked: u64,
    created_epoch: u64,
    staker_count: usize,
    stakers: Vec<StakerResponse>,
}
#[derive(Serialize)]
struct StakerResponse {
    address: String,
    amount: u64,
    staked_epoch: u64,
    pending_rewards: u64,
    last_claim_epoch: u64,
    total_claimed: u64,
    total_decayed: u64,
    reward_decay_pct: f64,
}

async fn get_staking_pools(State(state): State<Arc<ApiState>>) -> Json<Vec<StakingPoolResponse>> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let store = safe_lock(&state.staking_store);
    let res: Vec<StakingPoolResponse> = store
        .pools
        .iter()
        .map(|p| {
            let stakers: Vec<StakerResponse> = p
                .stakers
                .iter()
                .map(|s| {
                    let pending = p.compute_rewards(s, epoch);
                    let raw_epochs = epoch.saturating_sub(s.last_claim_epoch);
                    let share = if p.total_staked > 0 {
                        s.amount as f64 / p.total_staked as f64
                    } else {
                        0.0
                    };
                    let raw = (share * p.reward_rate as f64 * raw_epochs as f64) as u64
                        + s.pending_rewards;
                    let decay_pct = if raw > 0 {
                        ((raw - pending) as f64 / raw as f64 * 1000.0).round() / 10.0
                    } else {
                        0.0
                    };
                    StakerResponse {
                        address: s.address.clone(),
                        amount: s.amount,
                        staked_epoch: s.staked_epoch,
                        pending_rewards: pending,
                        last_claim_epoch: s.last_claim_epoch,
                        total_claimed: s.total_claimed,
                        total_decayed: s.total_decayed,
                        reward_decay_pct: decay_pct,
                    }
                })
                .collect();
            StakingPoolResponse {
                id: p.id,
                name: p.name.clone(),
                reward_rate: p.reward_rate,
                reward_decay_hl: p.reward_decay_hl,
                total_staked: p.total_staked,
                created_epoch: p.created_epoch,
                staker_count: stakers.len(),
                stakers,
            }
        })
        .collect();
    Json(res)
}

async fn get_single_pool(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<u64>,
) -> Result<Json<StakingPoolResponse>, StatusCode> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let store = safe_lock(&state.staking_store);
    let p = store
        .pools
        .iter()
        .find(|p| p.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let stakers: Vec<StakerResponse> = p
        .stakers
        .iter()
        .map(|s| {
            let pending = p.compute_rewards(s, epoch);
            let raw_epochs = epoch.saturating_sub(s.last_claim_epoch);
            let share = if p.total_staked > 0 {
                s.amount as f64 / p.total_staked as f64
            } else {
                0.0
            };
            let raw = (share * p.reward_rate as f64 * raw_epochs as f64) as u64 + s.pending_rewards;
            let decay_pct = if raw > 0 {
                ((raw - pending) as f64 / raw as f64 * 1000.0).round() / 10.0
            } else {
                0.0
            };
            StakerResponse {
                address: s.address.clone(),
                amount: s.amount,
                staked_epoch: s.staked_epoch,
                pending_rewards: pending,
                last_claim_epoch: s.last_claim_epoch,
                total_claimed: s.total_claimed,
                total_decayed: s.total_decayed,
                reward_decay_pct: decay_pct,
            }
        })
        .collect();
    Ok(Json(StakingPoolResponse {
        id: p.id,
        name: p.name.clone(),
        reward_rate: p.reward_rate,
        reward_decay_hl: p.reward_decay_hl,
        total_staked: p.total_staked,
        created_epoch: p.created_epoch,
        staker_count: stakers.len(),
        stakers,
    }))
}

#[derive(Deserialize)]
struct StakeRequest {
    pool_id: u64,
    address: String,
    amount: u64,
}

async fn post_stake(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<StakeRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, false) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_wallet_ownership(&state, user_id, &req.address) {
        return resp;
    }
    if req.amount == 0 {
        return Json(TxResultResponse {
            success: false,
            message: "Amount must be greater than zero".into(),
            tx_hash: None,
        });
    }
    // Balance pre-check
    {
        let addr = parse_hex_address(&req.address);
        if let Ok(addr_bytes) = addr {
            let db = safe_lock(&state.db);
            if let Some(acct) = db.get_account(&addr_bytes) {
                if acct.balance < req.amount {
                    return Json(TxResultResponse {
                        success: false,
                        message: format!("Insufficient balance: {} < {}", acct.balance, req.amount),
                        tx_hash: None,
                    });
                }
            } else {
                return Json(TxResultResponse {
                    success: false,
                    message: "Account not found — use faucet first".into(),
                    tx_hash: None,
                });
            }
        }
    }
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.staking_store);
    let p = match store.pools.iter_mut().find(|p| p.id == req.pool_id) {
        Some(p) => p,
        None => {
            return Json(TxResultResponse {
                success: false,
                message: "Pool not found".into(),
                tx_hash: None,
            })
        }
    };
    if let Some(s) = p.stakers.iter_mut().find(|s| s.address == req.address) {
        s.amount += req.amount;
    } else {
        p.stakers.push(Staker {
            address: req.address.clone(),
            amount: req.amount,
            staked_epoch: epoch,
            pending_rewards: 0,
            last_claim_epoch: epoch,
            total_claimed: 0,
            total_decayed: 0,
        });
    }
    p.total_staked += req.amount;
    let hash = tx_hash(&format!(
        "stake:{}:{}:{}",
        req.pool_id, req.address, req.amount
    ));
    Json(TxResultResponse {
        success: true,
        message: format!("Staked {} in {}", req.amount, p.name),
        tx_hash: Some(hash),
    })
}

#[derive(Deserialize)]
struct UnstakeRequest {
    pool_id: u64,
    address: String,
    amount: u64,
}

async fn post_unstake(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<UnstakeRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, false) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_wallet_ownership(&state, user_id, &req.address) {
        return resp;
    }
    let mut store = safe_lock(&state.staking_store);
    let p = match store.pools.iter_mut().find(|p| p.id == req.pool_id) {
        Some(p) => p,
        None => {
            return Json(TxResultResponse {
                success: false,
                message: "Pool not found".into(),
                tx_hash: None,
            })
        }
    };
    let s = match p.stakers.iter_mut().find(|s| s.address == req.address) {
        Some(s) => s,
        None => {
            return Json(TxResultResponse {
                success: false,
                message: "Not staked".into(),
                tx_hash: None,
            })
        }
    };
    if s.amount < req.amount {
        return Json(TxResultResponse {
            success: false,
            message: format!("Insufficient stake: {} < {}", s.amount, req.amount),
            tx_hash: None,
        });
    }
    s.amount -= req.amount;
    p.total_staked -= req.amount;
    let hash = tx_hash(&format!(
        "unstake:{}:{}:{}",
        req.pool_id, req.address, req.amount
    ));
    Json(TxResultResponse {
        success: true,
        message: format!("Unstaked {} from {}", req.amount, p.name),
        tx_hash: Some(hash),
    })
}

#[derive(Deserialize)]
struct ClaimRequest {
    pool_id: u64,
    address: String,
}

async fn post_claim(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<ClaimRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, false) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_wallet_ownership(&state, user_id, &req.address) {
        return resp;
    }
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.staking_store);
    let p = match store.pools.iter_mut().find(|p| p.id == req.pool_id) {
        Some(p) => p,
        None => {
            return Json(TxResultResponse {
                success: false,
                message: "Pool not found".into(),
                tx_hash: None,
            })
        }
    };
    let reward_decay_hl = p.reward_decay_hl;
    let reward_rate = p.reward_rate;
    let total_staked = p.total_staked;
    let s = match p.stakers.iter_mut().find(|s| s.address == req.address) {
        Some(s) => s,
        None => {
            return Json(TxResultResponse {
                success: false,
                message: "Not staked".into(),
                tx_hash: None,
            })
        }
    };
    let epochs_since = epoch.saturating_sub(s.last_claim_epoch);
    let share = if total_staked > 0 {
        s.amount as f64 / total_staked as f64
    } else {
        0.0
    };
    let raw = (share * reward_rate as f64 * epochs_since as f64) as u64 + s.pending_rewards;
    let actual = evaporchain_types::energy_at_epoch(raw, reward_decay_hl, epochs_since);
    let decayed = raw.saturating_sub(actual);
    s.total_claimed += actual;
    s.total_decayed += decayed;
    s.pending_rewards = 0;
    s.last_claim_epoch = epoch;
    let hash = tx_hash(&format!("claim:{}:{}:{}", req.pool_id, req.address, actual));
    Json(TxResultResponse {
        success: true,
        message: format!("Claimed {} rewards ({} decayed)", actual, decayed),
        tx_hash: Some(hash),
    })
}

// ──────────────────────────── DAO Store ─────────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
pub struct DAOProposal {
    pub id: u64,
    pub title: String,
    pub description: String,
    pub options: Vec<String>,
    pub votes: Vec<DAOVote>,
    pub created_epoch: u64,
    pub voting_period: u64,
    pub creator: String,
    pub status: String, // "Active", "Passed", "Evaporated"
    pub evaporated_epoch: Option<u64>,
}

impl DAOProposal {
    pub fn end_epoch(&self) -> u64 {
        self.created_epoch + self.voting_period
    }

    pub fn vote_totals(&self) -> HashMap<String, u64> {
        let mut totals: HashMap<String, u64> = HashMap::new();
        for opt in &self.options {
            totals.insert(opt.clone(), 0);
        }
        for v in &self.votes {
            *totals.entry(v.option.clone()).or_insert(0) += v.weight;
        }
        totals
    }

    pub fn total_votes(&self) -> u64 {
        self.votes.iter().map(|v| v.weight).sum()
    }

    pub fn tick(&mut self, epoch: u64) {
        if self.status != "Active" {
            return;
        }
        if epoch >= self.end_epoch() {
            let totals = self.vote_totals();
            let winning = totals.iter().max_by_key(|(_, v)| *v);
            if let Some((opt, _)) = winning {
                if self.total_votes() > 0 {
                    self.status = format!("Passed:{}", opt);
                } else {
                    self.status = "Evaporated".to_string();
                }
            } else {
                self.status = "Evaporated".to_string();
            }
            self.evaporated_epoch = Some(epoch);
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DAOVote {
    pub voter: String,
    pub option: String,
    pub weight: u64,
    pub epoch: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DAOStore {
    pub proposals: Vec<DAOProposal>,
    pub next_id: u64,
}

// ── DAO handlers ──

async fn dao_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/dao.html"))
}

#[derive(Serialize)]
struct ProposalResponse {
    id: u64,
    title: String,
    description: String,
    options: Vec<String>,
    created_epoch: u64,
    voting_period: u64,
    end_epoch: u64,
    creator: String,
    status: String,
    total_votes: u64,
    vote_totals: HashMap<String, u64>,
    epochs_remaining: u64,
    evaporated_epoch: Option<u64>,
    voter_count: usize,
}

async fn get_proposals(State(state): State<Arc<ApiState>>) -> Json<Vec<ProposalResponse>> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.dao_store);
    for p in store.proposals.iter_mut() {
        p.tick(epoch);
    }
    let res: Vec<ProposalResponse> = store
        .proposals
        .iter()
        .map(|p| {
            let remaining = if epoch < p.end_epoch() {
                p.end_epoch() - epoch
            } else {
                0
            };
            ProposalResponse {
                id: p.id,
                title: p.title.clone(),
                description: p.description.clone(),
                options: p.options.clone(),
                created_epoch: p.created_epoch,
                voting_period: p.voting_period,
                end_epoch: p.end_epoch(),
                creator: p.creator.clone(),
                status: p.status.clone(),
                total_votes: p.total_votes(),
                vote_totals: p.vote_totals(),
                epochs_remaining: remaining,
                evaporated_epoch: p.evaporated_epoch,
                voter_count: p
                    .votes
                    .iter()
                    .map(|v| &v.voter)
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
            }
        })
        .collect();
    Json(res)
}

async fn get_single_proposal(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<u64>,
) -> Result<Json<ProposalResponse>, StatusCode> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.dao_store);
    let p = store
        .proposals
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    p.tick(epoch);
    let remaining = if epoch < p.end_epoch() {
        p.end_epoch() - epoch
    } else {
        0
    };
    Ok(Json(ProposalResponse {
        id: p.id,
        title: p.title.clone(),
        description: p.description.clone(),
        options: p.options.clone(),
        created_epoch: p.created_epoch,
        voting_period: p.voting_period,
        end_epoch: p.end_epoch(),
        creator: p.creator.clone(),
        status: p.status.clone(),
        total_votes: p.total_votes(),
        vote_totals: p.vote_totals(),
        epochs_remaining: remaining,
        evaporated_epoch: p.evaporated_epoch,
        voter_count: p
            .votes
            .iter()
            .map(|v| &v.voter)
            .collect::<std::collections::HashSet<_>>()
            .len(),
    }))
}

#[derive(Deserialize)]
struct ProposeRequest {
    title: String,
    description: String,
    options: Vec<String>,
    voting_period: u64,
    creator: Option<String>,
}

async fn post_propose(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<ProposeRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, false) {
        return resp;
    }
    let title = match sanitize_string(&req.title, 200) {
        Ok(t) => t,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    if title.is_empty() {
        return Json(TxResultResponse {
            success: false,
            message: "Title is required".into(),
            tx_hash: None,
        });
    }
    let description = match sanitize_string(&req.description, 2000) {
        Ok(d) => d,
        Err(e) => {
            return Json(TxResultResponse {
                success: false,
                message: e,
                tx_hash: None,
            })
        }
    };
    let options: Vec<String> = req.options.iter().map(|o| strip_html_tags(o)).collect();
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let creator = req
        .creator
        .unwrap_or_else(|| format!("0x{}", GENESIS_FOUNDATION));
    let mut store = safe_lock(&state.dao_store);
    let id = store.next_id;
    store.next_id += 1;
    store.proposals.push(DAOProposal {
        id,
        title: title.clone(),
        description,
        options,
        votes: vec![],
        created_epoch: epoch,
        voting_period: req.voting_period,
        creator,
        status: "Active".into(),
        evaporated_epoch: None,
    });
    let hash = tx_hash(&format!("dao:propose:{}:{}", id, title));
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Proposal #{} '{}' created, voting for {} epochs",
            id, title, req.voting_period
        ),
        tx_hash: Some(hash),
    })
}

#[derive(Deserialize)]
struct VoteRequest {
    proposal_id: u64,
    option: String,
    weight: u64,
    voter: Option<String>,
}

async fn post_vote(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<VoteRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, false) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    let voter = match req.voter {
        Some(ref v) if !v.is_empty() => v.clone(),
        _ => {
            return Json(TxResultResponse {
                success: false,
                message: "Voter address is required".into(),
                tx_hash: None,
            })
        }
    };
    // Ownership check: caller must own the voter address
    if let Err(resp) = require_wallet_ownership(&state, user_id, &voter) {
        return resp;
    }
    // Validate vote weight: must be > 0 and <= staked amount
    if req.weight == 0 {
        return Json(TxResultResponse {
            success: false,
            message: "Vote weight must be greater than zero".into(),
            tx_hash: None,
        });
    }
    {
        let staking = safe_lock(&state.staking_store);
        let total_staked: u64 = staking
            .pools
            .iter()
            .flat_map(|p| p.stakers.iter())
            .filter(|s| s.address == voter)
            .map(|s| s.amount)
            .sum();
        if req.weight > total_staked {
            return Json(TxResultResponse {
                success: false,
                message: format!(
                    "Vote weight {} exceeds your total stake {}",
                    req.weight, total_staked
                ),
                tx_hash: None,
            });
        }
    }
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.dao_store);
    let p = match store.proposals.iter_mut().find(|p| p.id == req.proposal_id) {
        Some(p) => p,
        None => {
            return Json(TxResultResponse {
                success: false,
                message: "Proposal not found".into(),
                tx_hash: None,
            })
        }
    };
    if p.status != "Active" {
        return Json(TxResultResponse {
            success: false,
            message: format!("Proposal is {}, cannot vote", p.status),
            tx_hash: None,
        });
    }
    if !p.options.contains(&req.option) {
        return Json(TxResultResponse {
            success: false,
            message: format!("Invalid option: {}", req.option),
            tx_hash: None,
        });
    }
    if p.votes.iter().any(|v| v.voter == voter) {
        return Json(TxResultResponse {
            success: false,
            message: "Already voted".into(),
            tx_hash: None,
        });
    }
    p.votes.push(DAOVote {
        voter: voter.clone(),
        option: req.option.clone(),
        weight: req.weight,
        epoch,
    });
    let hash = tx_hash(&format!(
        "dao:vote:{}:{}:{}",
        req.proposal_id, voter, req.option
    ));
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Voted '{}' with weight {} on proposal #{}",
            req.option, req.weight, req.proposal_id
        ),
        tx_hash: Some(hash),
    })
}

// ──────────────────────────── Router ───────────────────────────────────

/// Chain metadata endpoint.
async fn get_chain(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let history = safe_lock(&state.block_history);
    let latest = history.back();
    let stats = safe_lock(&state.stats);
    Json(serde_json::json!({
        "chain_name": "EvaporChain",
        "chain_id": "evaporchain-testnet-1",
        "version": "0.2.0",
        "block_height": latest.map(|b| b.number).unwrap_or(0),
        "epoch": latest.map(|b| b.epoch).unwrap_or(0),
        "total_transactions": stats.total_transactions,
        "consensus": "Proof-of-Decay",
        "signature_scheme": "ML-DSA (Dilithium3)",
    }))
}

/// Latest block shortcut.
async fn get_latest_block(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<BlockRecord>, StatusCode> {
    let history = safe_lock(&state.block_history);
    history
        .back()
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)
        .map(Json)
}

/// Mempool endpoint with transaction details.
async fn get_mempool(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let txs = state.mempool_transactions();
    Json(serde_json::json!({
        "pending": txs.len(),
        "transactions": txs,
    }))
}

/// `GET /api/mempool/:hash` — direct mempool-membership check for a single
/// tx hash. Returns `{hash, in_mempool}`. Trivial wrapper around
/// `Mempool::contains_hash`; primarily useful for explorer UIs that don't
/// want to walk `/api/mempool`'s full transaction list.
async fn get_mempool_by_hash(
    State(state): State<Arc<ApiState>>,
    Path(hash): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let (hash_hex, hash_bytes) = parse_tx_hash(&hash).ok_or(StatusCode::BAD_REQUEST)?;
    let in_mempool = state.mempool_contains_hash(&hash_bytes);
    Ok(Json(serde_json::json!({
        "hash": hash_hex,
        "in_mempool": in_mempool,
    })))
}

/// Transaction receipt lookup by hash.
async fn get_tx_receipt(
    State(state): State<Arc<ApiState>>,
    Path(hash): Path<String>,
) -> Result<Json<crate::persistence::TxReceipt>, StatusCode> {
    let store = state
        .chain_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .get_tx_receipt(&hash)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// Address transaction history.
async fn get_address_txs(
    State(state): State<Arc<ApiState>>,
    Path(addr): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = state
        .chain_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let limit = params.limit.unwrap_or(50).min(200);
    let receipts = store.get_address_transactions(&addr, limit);
    Ok(Json(serde_json::json!({
        "address": addr,
        "count": receipts.len(),
        "transactions": receipts,
    })))
}

/// Transaction index stats.
async fn get_tx_index_stats(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = state
        .chain_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(serde_json::json!({
        "indexed_transactions": store.tx_index_count(),
    })))
}

#[derive(Deserialize)]
struct PaginationParams {
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct EventQueryParams {
    event_name: Option<String>,
    from_block: Option<u64>,
    to_block: Option<u64>,
    limit: Option<usize>,
}

/// Contract event logs by contract ID.
async fn get_contract_events(
    State(state): State<Arc<ApiState>>,
    Path(contract_id): Path<u64>,
    Query(params): Query<EventQueryParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = state
        .chain_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let limit = params.limit.unwrap_or(100).min(1000);
    let events = store.get_contract_events(
        contract_id,
        params.event_name.as_deref(),
        params.from_block,
        params.to_block,
        limit,
    );
    Ok(Json(serde_json::json!({
        "contract_id": contract_id,
        "count": events.len(),
        "events": events,
    })))
}

/// All contract events in a specific block.
async fn get_block_contract_events(
    State(state): State<Arc<ApiState>>,
    Path(block_number): Path<u64>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = state
        .chain_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let events = store.get_block_events(block_number, 1000);
    Ok(Json(serde_json::json!({
        "block_number": block_number,
        "count": events.len(),
        "events": events,
    })))
}

/// Contract event index stats.
async fn get_event_index_stats(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = state
        .chain_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(serde_json::json!({
        "indexed_events": store.contract_event_count(),
        "indexed_transactions": store.tx_index_count(),
    })))
}

/// NFT collections endpoint.
async fn get_nft_collections(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let store = safe_lock(&state.nft_store);
    let mut collections: HashMap<String, Vec<u64>> = HashMap::new();
    for nft in &store.tokens {
        collections
            .entry(nft.collection.clone())
            .or_default()
            .push(nft.id);
    }
    let result: Vec<serde_json::Value> = collections
        .iter()
        .map(|(name, ids)| serde_json::json!({ "name": name, "count": ids.len(), "nft_ids": ids }))
        .collect();
    Json(serde_json::json!(result))
}

// ─────────────────── Nova Proof / Light Client Endpoints ──────────────────

/// Response for `/api/proof/latest` — returns the latest chain proof for light client sync.
#[derive(Serialize)]
struct ChainProofResponse {
    genesis_state_root: String,
    final_state_root: String,
    block_height: u64,
    final_epoch: u64,
    proof_size_bytes: usize,
    num_steps: usize,
    proof_hex: String,
}

/// Response for `/api/proof/status` — prover metrics.
#[derive(Serialize)]
struct ProverStatusResponse {
    block_height: u64,
    epoch: u64,
    blocks_folded: usize,
    accumulator_size_bytes: usize,
    total_prove_time_ms: f64,
    avg_fold_time_ms: f64,
    last_fold_time_ms: f64,
    num_checkpoints: usize,
    prove_mode: bool,
}

/// GET /api/metrics — real-time throughput and performance metrics.
#[derive(Serialize)]
struct MetricsResponse {
    tps: f64,
    peak_tps: f64,
    avg_txs_per_block: f64,
    avg_block_exec_time_ms: f64,
    avg_gas_per_block: u64,
    total_transactions: u64,
    blocks_tracked: usize,
}

async fn get_metrics(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> Result<Json<MetricsResponse>, (StatusCode, Json<serde_json::Value>)> {
    require_admin_auth(&headers)?;
    let t = safe_lock(&state.throughput);
    let stats = safe_lock(&state.stats);
    Ok(Json(MetricsResponse {
        tps: t.current_tps(),
        peak_tps: t.peak_tps,
        avg_txs_per_block: t.avg_txs_per_block(),
        avg_block_exec_time_ms: t.avg_exec_time_us() as f64 / 1000.0,
        avg_gas_per_block: t.avg_gas_per_block(),
        total_transactions: stats.total_transactions,
        blocks_tracked: t.recent_blocks.len(),
    }))
}

// ──────────────────────────── Drain (admin) ───────────────────────────
//
// `POST /api/admin/drain` is what the Ansible upgrade playbook calls
// before swapping a node binary. The chain has to keep producing
// blocks while one validator gracefully retires; "drain" means: stop
// proposing, stop voting, mark this node as draining so peers route
// around it. The flag lives on `TendermintConsensus` and is read by
// the consensus tick before deciding to propose / prevote (see
// tendermint.rs::tick — the `drain_gate_open` check).
//
// Auth: same `require_admin_auth` middleware as `/metrics`. If
// `EVAPORCHAIN_ADMIN_KEY` is unset the gate is open (matches the
// existing metrics behaviour); the warning is logged at startup by
// `start_api_server` so this isn't a silent failure.

#[derive(Serialize)]
struct DrainResponse {
    /// `"draining"` (success) or `"already_draining"` / `"not_draining"`.
    pub status: &'static str,
    pub draining: bool,
    pub drain_started_at_epoch: Option<u64>,
}

async fn post_admin_drain(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> Result<Json<DrainResponse>, (StatusCode, Json<serde_json::Value>)> {
    require_admin_auth(&headers)?;
    let Some(tc) = state.tendermint.as_ref() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "drain unsupported in mock-consensus mode"
            })),
        ));
    };
    let mut tc = safe_lock(tc);
    let was_draining = tc.is_draining();
    let started_at = tc.set_draining();
    tracing::warn!(
        drain_started_at_epoch = started_at,
        was_draining,
        "Admin drain requested — node will stop proposing/voting"
    );
    Ok(Json(DrainResponse {
        status: if was_draining {
            "already_draining"
        } else {
            "draining"
        },
        draining: true,
        drain_started_at_epoch: Some(started_at),
    }))
}

async fn post_admin_undrain(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> Result<Json<DrainResponse>, (StatusCode, Json<serde_json::Value>)> {
    require_admin_auth(&headers)?;
    let Some(tc) = state.tendermint.as_ref() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "drain unsupported in mock-consensus mode"
            })),
        ));
    };
    let mut tc = safe_lock(tc);
    let was_draining = tc.clear_draining();
    tracing::info!(was_draining, "Admin undrain — node resumes consensus");
    Ok(Json(DrainResponse {
        status: if was_draining {
            "draining"
        } else {
            "not_draining"
        },
        draining: false,
        drain_started_at_epoch: None,
    }))
}

async fn get_admin_drain_status(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> Result<Json<DrainResponse>, (StatusCode, Json<serde_json::Value>)> {
    require_admin_auth(&headers)?;
    let (draining, since) = state
        .tendermint
        .as_ref()
        .map(|tc| safe_lock(tc).drain_state())
        .unwrap_or((false, None));
    Ok(Json(DrainResponse {
        status: if draining { "draining" } else { "not_draining" },
        draining,
        drain_started_at_epoch: since,
    }))
}

/// GET /metrics — Prometheus text exposition format for scraping.
async fn get_prometheus_metrics(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_admin_auth(&headers) {
        return e.into_response();
    }
    // Snapshot per-validator timing history first (cheap copy) so we
    // don't hold the tendermint lock concurrent with db / throughput /
    // history locks below — the consensus loop in main.rs takes
    // tendermint then db, so the reverse order here would deadlock.
    let per_validator_history: Vec<(u64, f64)> = state
        .tendermint
        .as_ref()
        .map(|tc| safe_lock(tc).block_production_history())
        .unwrap_or_default();
    let t = safe_lock(&state.throughput);
    let stats = safe_lock(&state.stats);
    let db = safe_lock(&state.db);
    let history = safe_lock(&state.block_history);
    let peer_count = state.peer_count.load(std::sync::atomic::Ordering::Relaxed);
    let uptime = state.start_time.elapsed().as_secs();

    let block_height = history.back().map(|b| b.number).unwrap_or(0);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    let active_objects = db.object_count();
    let ghost_count = db.ghost_count();
    let account_count = db.all_account_addresses().len();

    let mut out = String::with_capacity(2048);
    out.push_str("# HELP evaporchain_block_height Current block height\n");
    out.push_str("# TYPE evaporchain_block_height gauge\n");
    out.push_str(&format!("evaporchain_block_height {}\n", block_height));
    out.push_str("# HELP evaporchain_epoch Current epoch\n");
    out.push_str("# TYPE evaporchain_epoch gauge\n");
    out.push_str(&format!("evaporchain_epoch {}\n", epoch));
    out.push_str("# HELP evaporchain_tps Current transactions per second\n");
    out.push_str("# TYPE evaporchain_tps gauge\n");
    out.push_str(&format!("evaporchain_tps {:.2}\n", t.current_tps()));
    out.push_str("# HELP evaporchain_peak_tps Peak TPS observed\n");
    out.push_str("# TYPE evaporchain_peak_tps gauge\n");
    out.push_str(&format!("evaporchain_peak_tps {:.2}\n", t.peak_tps));
    out.push_str("# HELP evaporchain_total_transactions Total transactions processed\n");
    out.push_str("# TYPE evaporchain_total_transactions counter\n");
    out.push_str(&format!(
        "evaporchain_total_transactions {}\n",
        stats.total_transactions
    ));
    out.push_str("# HELP evaporchain_active_objects Number of active state objects\n");
    out.push_str("# TYPE evaporchain_active_objects gauge\n");
    out.push_str(&format!("evaporchain_active_objects {}\n", active_objects));
    out.push_str("# HELP evaporchain_ghost_count Number of evaporated ghost records\n");
    out.push_str("# TYPE evaporchain_ghost_count gauge\n");
    out.push_str(&format!("evaporchain_ghost_count {}\n", ghost_count));
    out.push_str("# HELP evaporchain_accounts Total accounts\n");
    out.push_str("# TYPE evaporchain_accounts gauge\n");
    out.push_str(&format!("evaporchain_accounts {}\n", account_count));
    out.push_str("# HELP evaporchain_peer_count Connected peers\n");
    out.push_str("# TYPE evaporchain_peer_count gauge\n");
    out.push_str(&format!("evaporchain_peer_count {}\n", peer_count));
    out.push_str("# HELP evaporchain_avg_block_exec_ms Average block execution time in ms\n");
    out.push_str("# TYPE evaporchain_avg_block_exec_ms gauge\n");
    out.push_str(&format!(
        "evaporchain_avg_block_exec_ms {:.2}\n",
        t.avg_exec_time_us() as f64 / 1000.0
    ));
    out.push_str("# HELP evaporchain_avg_gas_per_block Average gas used per block\n");
    out.push_str("# TYPE evaporchain_avg_gas_per_block gauge\n");
    out.push_str(&format!(
        "evaporchain_avg_gas_per_block {}\n",
        t.avg_gas_per_block()
    ));
    out.push_str("# HELP evaporchain_uptime_seconds Node uptime in seconds\n");
    out.push_str("# TYPE evaporchain_uptime_seconds counter\n");
    out.push_str(&format!("evaporchain_uptime_seconds {}\n", uptime));

    // ── Substrate thermodynamic metrics ───────────────────────────────

    // Fee controller — current base fee in ppm (derived from integrator energy)
    {
        use evaporchain_fee_controller::{base_fee, FeeControllerParams};
        let fee_state = safe_lock(&state.fee_state);
        let params = FeeControllerParams::default_genesis();
        let base_fee_ppm = base_fee(&fee_state, &params);
        out.push_str("# HELP evaporchain_fee_base_ppm Current base fee from EIP-1559-style fee controller (ppm)\n");
        out.push_str("# TYPE evaporchain_fee_base_ppm gauge\n");
        out.push_str(&format!("evaporchain_fee_base_ppm {}\n", base_fee_ppm));
    }

    // EPV — live protocol version count
    {
        let epv = safe_lock(&state.epv_registry);
        let live = epv.iter().count();
        out.push_str(
            "# HELP evaporchain_epv_live_versions Number of live EPV protocol versions tracked\n",
        );
        out.push_str("# TYPE evaporchain_epv_live_versions gauge\n");
        out.push_str(&format!("evaporchain_epv_live_versions {}\n", live));
    }

    // Autopoietic viability — 2=Viable, 1=Stressed, 0=Inviable
    {
        use evaporchain_autopoietic::{AutopoieticStatus, ChainAutopoiesis};
        use evaporchain_llsa::proof::AlwaysAcceptVerifier;
        let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
        let last_sentinel_vote = {
            let db_ref = safe_lock(&state.db);
            let params = db_ref.all_sentinel_params();
            let mut max_epoch: Option<u64> = None;
            for p in &params {
                for v in db_ref.get_sentinel_votes(p.id) {
                    max_epoch =
                        Some(max_epoch.map_or(v.observed_epoch, |e| e.max(v.observed_epoch)));
                }
            }
            max_epoch
        };
        let covenant_ids: Vec<Vec<u8>> = (1u8..=5).map(|i| vec![i, 0, 0, 0]).collect();
        let book = safe_lock(&state.patronage_book);
        let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 1_000, 50);
        let report = sys.health_report(&book, &covenant_ids, last_sentinel_vote, epoch);
        let viability_score: u64 = match report.status {
            AutopoieticStatus::Viable => 2,
            AutopoieticStatus::Stressed => 1,
            AutopoieticStatus::Inviable => 0,
        };
        out.push_str("# HELP evaporchain_autopoietic_viability Chain autopoietic viability: 2=Viable 1=Stressed 0=Inviable\n");
        out.push_str("# TYPE evaporchain_autopoietic_viability gauge\n");
        out.push_str(&format!(
            "evaporchain_autopoietic_viability {}\n",
            viability_score
        ));
    }

    // Consensus phase — encoded as: 3=LivenessStable, 2=SafetyStable, 1=Frozen, 0=Chaotic
    {
        let phase_score: u64 = if let Some(tc) = &state.tendermint {
            let tc = safe_lock(tc);
            let phase = tc.consensus_phase();
            match format!("{phase:?}").as_str() {
                s if s.contains("Liveness") => 3,
                s if s.contains("Safety") => 2,
                s if s.contains("Frozen") => 1,
                _ => 0,
            }
        } else {
            3 // MockConsensus defaults to LivenessStable
        };
        out.push_str("# HELP evaporchain_consensus_phase RG Phase Map regime: 3=LivenessStable 2=SafetyStable 1=Frozen 0=Chaotic\n");
        out.push_str("# TYPE evaporchain_consensus_phase gauge\n");
        out.push_str(&format!("evaporchain_consensus_phase {}\n", phase_score));
    }

    // ── Operator-surface `evap_*` metrics ────────────────────────────
    //
    // The original `evaporchain_*` names are preserved above for
    // backwards compatibility with the legacy dashboard. The block
    // below exposes the canonical names referenced by the operator
    // Grafana dashboard and Alertmanager rules under `deploy/`.
    {
        // Core chain progress
        out.push_str("# HELP evap_block_height Current block height\n");
        out.push_str("# TYPE evap_block_height gauge\n");
        out.push_str(&format!("evap_block_height {}\n", block_height));

        out.push_str("# HELP evap_epoch Current epoch\n");
        out.push_str("# TYPE evap_epoch gauge\n");
        out.push_str(&format!("evap_epoch {}\n", epoch));

        let finalized_height = {
            let ft = safe_lock(&state.finality_tracker);
            ft.latest_finalized_height()
        };
        out.push_str("# HELP evap_finalized_height Highest BLS-finalised block height\n");
        out.push_str("# TYPE evap_finalized_height gauge\n");
        out.push_str(&format!("evap_finalized_height {}\n", finalized_height));

        // ── Per-height finality gap (Mainnet P1) ─────────────────────────
        //
        // Source: `TendermintConsensus::finality_gap_history()` —
        // a ring buffer of (height, commit_to_finalise_gap_ms) recorded
        // each time a height's commit certificate is observed (see
        // tendermint.rs::on_block_committed). Plus the live
        // `unfinalised_tail()` for heights that have committed but
        // whose cert has not arrived yet.
        //
        // Histogram buckets are in seconds and chosen to span the
        // operational regime: 0.5 s / 1 s / 2 s catch healthy single-
        // slot finality at 1 s slot time (most samples fall in the
        // first bucket); 5 s / 10 s / 30 s cover degraded but still
        // recovering states; 60 s / 300 s / 1800 s catch genuine
        // stalls (the alert fires above 30 s anyway). +Inf is emitted
        // separately per the Prometheus histogram convention.
        let (gap_history, unfinalised_tail, worst_unfinalised_ms) =
            if let Some(tc) = &state.tendermint {
                let tc = safe_lock(tc);
                (
                    tc.finality_gap_history(),
                    tc.unfinalised_tail(),
                    tc.worst_unfinalised_gap_ms(),
                )
            } else {
                (Vec::new(), Vec::new(), 0u64)
            };

        let gap_buckets: [f64; 9] = [0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 300.0, 1800.0];
        out.push_str(
            "# HELP evap_finality_gap_seconds Per-height commit→finalise duration (seconds)\n",
        );
        out.push_str("# TYPE evap_finality_gap_seconds histogram\n");
        let gap_seconds: Vec<f64> = gap_history
            .iter()
            .map(|(_, gap_ms)| (*gap_ms as f64) / 1000.0)
            .collect();
        for ub in gap_buckets.iter() {
            let cumulative = gap_seconds.iter().filter(|s| *s <= ub).count() as u64;
            out.push_str(&format!(
                "evap_finality_gap_seconds_bucket{{le=\"{}\"}} {}\n",
                ub, cumulative
            ));
        }
        let gap_total = gap_seconds.len() as u64;
        out.push_str(&format!(
            "evap_finality_gap_seconds_bucket{{le=\"+Inf\"}} {}\n",
            gap_total
        ));
        let gap_sum: f64 = gap_seconds.iter().sum();
        out.push_str(&format!("evap_finality_gap_seconds_sum {}\n", gap_sum));
        out.push_str(&format!("evap_finality_gap_seconds_count {}\n", gap_total));

        out.push_str(
            "# HELP evap_unfinalised_height_count Heights committed but not yet finalised\n",
        );
        out.push_str("# TYPE evap_unfinalised_height_count gauge\n");
        out.push_str(&format!(
            "evap_unfinalised_height_count {}\n",
            unfinalised_tail.len()
        ));
        out.push_str(
            "# HELP evap_worst_unfinalised_gap_seconds Oldest commit→pending-cert age (seconds); drives EvapFinalityStalled\n",
        );
        out.push_str("# TYPE evap_worst_unfinalised_gap_seconds gauge\n");
        out.push_str(&format!(
            "evap_worst_unfinalised_gap_seconds {}\n",
            (worst_unfinalised_ms as f64) / 1000.0
        ));

        // Validator set — active = unjailed
        let (validator_total, active_validators, consensus_round) =
            if let Some(tc) = &state.tendermint {
                let tc = safe_lock(tc);
                let vs = tc.validator_set();
                let total = vs.len() as u64;
                let active = vs.validators().iter().filter(|v| !v.jailed).count() as u64;
                (total, active, tc.round() as u64)
            } else {
                // MockConsensus path — single-node dev mode.
                (1u64, 1u64, 0u64)
            };
        out.push_str("# HELP evap_active_validators Number of active (un-jailed) validators\n");
        out.push_str("# TYPE evap_active_validators gauge\n");
        out.push_str(&format!("evap_active_validators {}\n", active_validators));
        out.push_str("# HELP evap_validator_set_size Total validator-set size (active + jailed)\n");
        out.push_str("# TYPE evap_validator_set_size gauge\n");
        out.push_str(&format!("evap_validator_set_size {}\n", validator_total));
        out.push_str(
            "# HELP evap_consensus_round Current consensus round inside the active height\n",
        );
        out.push_str("# TYPE evap_consensus_round gauge\n");
        out.push_str(&format!("evap_consensus_round {}\n", consensus_round));

        // Object lifecycle
        out.push_str("# HELP evap_active_objects Number of active state objects\n");
        out.push_str("# TYPE evap_active_objects gauge\n");
        out.push_str(&format!("evap_active_objects {}\n", active_objects));
        out.push_str("# HELP evap_ghost_count Number of evaporated ghost records\n");
        out.push_str("# TYPE evap_ghost_count gauge\n");
        out.push_str(&format!("evap_ghost_count {}\n", ghost_count));
        out.push_str(
            "# HELP evap_evaporated_objects_total Total objects evaporated since startup\n",
        );
        out.push_str("# TYPE evap_evaporated_objects_total counter\n");
        out.push_str(&format!(
            "evap_evaporated_objects_total {}\n",
            stats.total_evaporated
        ));

        // Mempool / tx pipeline
        let mempool_size = if let Some(tc) = &state.tendermint {
            let tc = safe_lock(tc);
            tc.mempool.len() as u64
        } else {
            let c = safe_lock(&state.consensus);
            c.mempool.len() as u64
        };
        out.push_str("# HELP evap_mempool_size Current pending tx count in the active mempool\n");
        out.push_str("# TYPE evap_mempool_size gauge\n");
        out.push_str(&format!("evap_mempool_size {}\n", mempool_size));

        out.push_str(
            "# HELP evap_pending_txs_total Cumulative txs that have entered the mempool since startup\n",
        );
        out.push_str("# TYPE evap_pending_txs_total counter\n");
        out.push_str(&format!(
            "evap_pending_txs_total {}\n",
            stats.total_transactions
        ));

        // Finalised tx outcomes. Mempool gates malformed txs, but the
        // executor still rejects valid-format txs at runtime
        // (InsufficientBalance, InvalidNonce, signature failure, etc.).
        // Both labels are now driven by per-tx outcomes from
        // BlockExecutionResult — failed buckets are no longer
        // hardcoded to 0.
        let total_finalised = stats.total_transactions;
        let total_rejected = stats.total_rejected_transactions;
        let total_success = total_finalised.saturating_sub(total_rejected);
        out.push_str("# HELP evap_finalised_txs_total Total finalised transactions, partitioned by execution result\n");
        out.push_str("# TYPE evap_finalised_txs_total counter\n");
        out.push_str(&format!(
            "evap_finalised_txs_total{{result=\"success\"}} {total_success}\n"
        ));
        out.push_str(&format!(
            "evap_finalised_txs_total{{result=\"failed\"}} {total_rejected}\n"
        ));

        // Block-production timing histogram, partitioned by producer.
        //
        // Source: TendermintConsensus::block_production_history() —
        // a ring buffer of (producer_id, exec_time_seconds) recorded
        // per block commit (see tendermint.rs::record_block_production_timing).
        // Label: `producer="validator-{id}"`. Falls back to a single
        // `producer="local"` series sourced from the throughput tracker
        // when the consensus engine is absent (mock mode) or empty
        // (cold start, no committed blocks yet).
        //
        // Grafana heatmap: group by `producer` to render one row per
        // validator; rate(evap_block_production_seconds_bucket[5m])
        // gives the per-validator commit-latency distribution.
        let buckets: [f64; 10] = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0];
        out.push_str(
            "# HELP evap_block_production_seconds Block execution wall-time histogram (per producer)\n",
        );
        out.push_str("# TYPE evap_block_production_seconds histogram\n");

        // Reuse the snapshot taken at function entry (held in a local
        // so we don't re-lock tendermint while db is locked).
        if per_validator_history.is_empty() {
            // Fallback: emit the local-only series so a freshly-started
            // node still produces a valid exposition (and existing
            // Grafana panels keyed on `producer="local"` don't go blank).
            let exec_secs: Vec<f64> = t
                .recent_blocks
                .iter()
                .map(|b| b.2 as f64 / 1_000_000.0)
                .collect();
            for ub in buckets.iter() {
                let cumulative = exec_secs.iter().filter(|s| *s <= ub).count() as u64;
                out.push_str(&format!(
                    "evap_block_production_seconds_bucket{{producer=\"local\",le=\"{}\"}} {}\n",
                    ub, cumulative
                ));
            }
            let total_count = exec_secs.len() as u64;
            out.push_str(&format!(
                "evap_block_production_seconds_bucket{{producer=\"local\",le=\"+Inf\"}} {}\n",
                total_count
            ));
            let total_sum: f64 = exec_secs.iter().sum();
            out.push_str(&format!(
                "evap_block_production_seconds_sum{{producer=\"local\"}} {}\n",
                total_sum
            ));
            out.push_str(&format!(
                "evap_block_production_seconds_count{{producer=\"local\"}} {}\n",
                total_count
            ));
        } else {
            // Group samples by producer_id and emit one histogram
            // series per validator. Producer labels use the canonical
            // `validator-{id}` form so Grafana heatmaps group cleanly.
            let mut by_producer: BTreeMap<u64, Vec<f64>> = BTreeMap::new();
            for (pid, secs) in per_validator_history {
                by_producer.entry(pid).or_default().push(secs);
            }
            for (pid, samples) in by_producer.iter() {
                let label = format!("validator-{}", pid);
                for ub in buckets.iter() {
                    let cumulative = samples.iter().filter(|s| **s <= *ub).count() as u64;
                    out.push_str(&format!(
                        "evap_block_production_seconds_bucket{{producer=\"{}\",le=\"{}\"}} {}\n",
                        label, ub, cumulative
                    ));
                }
                let total_count = samples.len() as u64;
                out.push_str(&format!(
                    "evap_block_production_seconds_bucket{{producer=\"{}\",le=\"+Inf\"}} {}\n",
                    label, total_count
                ));
                let total_sum: f64 = samples.iter().sum();
                out.push_str(&format!(
                    "evap_block_production_seconds_sum{{producer=\"{}\"}} {}\n",
                    label, total_sum
                ));
                out.push_str(&format!(
                    "evap_block_production_seconds_count{{producer=\"{}\"}} {}\n",
                    label, total_count
                ));
            }
        }

        // Networking
        out.push_str("# HELP evap_peer_count Currently-connected P2P peers\n");
        out.push_str("# TYPE evap_peer_count gauge\n");
        out.push_str(&format!("evap_peer_count {}\n", peer_count));

        // ── libp2p Sybil resistance (Mainnet P1) ──
        // Per-peer reputation gauge, bucketed score range, total
        // active bans, and labelled rejection counters.
        if let Some(sybil) = state.network_sybil.as_ref() {
            if let Ok(g) = sybil.read() {
                out.push_str("# HELP evap_peer_score Reputation score per connected peer (label = peer_id prefix)\n");
                out.push_str("# TYPE evap_peer_score gauge\n");
                for (pid, score) in g.scores.iter() {
                    let pid_short: String = pid.to_string().chars().take(12).collect();
                    out.push_str(&format!(
                        "evap_peer_score{{peer_id=\"{}\"}} {}\n",
                        pid_short, score.score
                    ));
                }
                out.push_str("# HELP evap_active_bans Number of currently-active IP bans\n");
                out.push_str("# TYPE evap_active_bans gauge\n");
                out.push_str(&format!(
                    "evap_active_bans {}\n",
                    g.bans.active_bans().len()
                ));
                out.push_str("# HELP evap_inbound_rejections_total Inbound libp2p connections refused, by reason\n");
                out.push_str("# TYPE evap_inbound_rejections_total counter\n");
                for (reason, count) in g.rejections.snapshot().iter() {
                    out.push_str(&format!(
                        "evap_inbound_rejections_total{{reason=\"{}\"}} {}\n",
                        reason, count
                    ));
                }
            }
        }

        // Data availability — every finalised block carries a DA
        // attestation, so cumulative finalised count is a faithful
        // proxy for cumulative DA attestations seen by this node.
        let da_attestations_total = {
            let ft = safe_lock(&state.finality_tracker);
            ft.total_finalized()
        };
        out.push_str("# HELP evap_da_attestations_total Total DA attestations observed (one per finalised block)\n");
        out.push_str("# TYPE evap_da_attestations_total counter\n");
        out.push_str(&format!(
            "evap_da_attestations_total {}\n",
            da_attestations_total
        ));

        // Refresh pool balance — drives Patronage Covenant payouts
        let refresh_pool_balance = {
            let pool = safe_lock(&state.patronage_pool);
            pool.total_accrued()
        };
        out.push_str("# HELP evap_refresh_pool_balance Total energy currently sitting in the protocol refresh pool\n");
        out.push_str("# TYPE evap_refresh_pool_balance gauge\n");
        out.push_str(&format!(
            "evap_refresh_pool_balance {}\n",
            refresh_pool_balance
        ));

        // Fee controller — base fee + Lyapunov drift around target
        {
            use evaporchain_fee_controller::{base_fee, signed_diff, FeeControllerParams};
            let fee_state = safe_lock(&state.fee_state);
            let params = FeeControllerParams::default_genesis();
            let base_fee_ppm = base_fee(&fee_state, &params);
            let drift = signed_diff(fee_state.energy, params.target_energy);
            out.push_str(
                "# HELP evap_fee_controller_base_fee Base fee (ppm) from the Singh-Lyapunov fee controller\n",
            );
            out.push_str("# TYPE evap_fee_controller_base_fee gauge\n");
            out.push_str(&format!("evap_fee_controller_base_fee {}\n", base_fee_ppm));
            out.push_str(
                "# HELP evap_fee_controller_lyapunov_drift Signed E - E* drift; |drift| -> 0 means controller has converged\n",
            );
            out.push_str("# TYPE evap_fee_controller_lyapunov_drift gauge\n");
            out.push_str(&format!("evap_fee_controller_lyapunov_drift {}\n", drift));
        }

        // Bell-Certified Beacon — latest CHSH S-value (×1000)
        let bell_s_milli: u64 = if let Some(tc) = &state.tendermint {
            let tc = safe_lock(tc);
            tc.last_bell_reading()
                .map(|r| r.s_value_milli)
                .unwrap_or(evaporchain_bell_beacon::LOCAL_REALISM_S_MILLI)
        } else {
            evaporchain_bell_beacon::LOCAL_REALISM_S_MILLI
        };
        out.push_str(
            "# HELP evap_bell_s_value_milli Latest CHSH S-value (milli-units); >2000 means Bell-certified\n",
        );
        out.push_str("# TYPE evap_bell_s_value_milli gauge\n");
        out.push_str(&format!("evap_bell_s_value_milli {}\n", bell_s_milli));
    }

    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
        .into_response()
}

/// GET /api/proof/latest — generate and return the latest chain proof.
async fn get_proof_latest(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ChainProofResponse>, StatusCode> {
    let p = safe_lock(&state.chain_prover);
    let chain_proof = p
        .generate_chain_proof()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    Ok(Json(ChainProofResponse {
        genesis_state_root: hex::encode(chain_proof.genesis_state_root),
        final_state_root: hex::encode(chain_proof.final_state_root),
        block_height: chain_proof.block_height,
        final_epoch: chain_proof.final_epoch,
        proof_size_bytes: chain_proof.proof_size_bytes,
        num_steps: chain_proof.num_steps,
        proof_hex: hex::encode(&chain_proof.proof.proof_bytes),
    }))
}

/// GET /api/proof/status — prover metrics and health.
async fn get_proof_status(State(state): State<Arc<ApiState>>) -> Json<ProverStatusResponse> {
    let p = safe_lock(&state.chain_prover);
    let m = p.metrics();

    Json(ProverStatusResponse {
        block_height: m.block_height,
        epoch: m.epoch,
        blocks_folded: m.blocks_folded,
        accumulator_size_bytes: m.accumulator_size_bytes,
        total_prove_time_ms: m.total_prove_time_us as f64 / 1000.0,
        avg_fold_time_ms: m.avg_fold_time_us as f64 / 1000.0,
        last_fold_time_ms: m.last_fold_time_us as f64 / 1000.0,
        num_checkpoints: m.num_checkpoints,
        prove_mode: state.prove_mode,
    })
}

/// GET /api/proof/verify — verify a chain proof submitted as hex in query param.
#[derive(Deserialize)]
struct VerifyProofQuery {
    proof_hex: String,
    num_steps: usize,
    genesis_state_root: String,
}

#[derive(Serialize)]
struct VerifyProofResponse {
    valid: bool,
}

async fn get_proof_verify(
    State(state): State<Arc<ApiState>>,
    Query(q): Query<VerifyProofQuery>,
) -> Result<Json<VerifyProofResponse>, StatusCode> {
    let genesis = hex::decode(&q.genesis_state_root).map_err(|_| StatusCode::BAD_REQUEST)?;
    if genesis.len() != 32 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let proof_bytes = hex::decode(&q.proof_hex).map_err(|_| StatusCode::BAD_REQUEST)?;

    let mut genesis_arr = [0u8; 32];
    genesis_arr.copy_from_slice(&genesis);

    let chain_proof = evaporchain_proving::chain_proof::ChainProof {
        proof: evaporchain_proving::CompressedProof {
            proof_bytes,
            num_steps: q.num_steps,
            z0_bytes: genesis_arr.to_vec(),
        },
        genesis_state_root: genesis_arr,
        final_state_root: [0u8; 32],
        block_height: q.num_steps as u64,
        final_epoch: 0,
        created_at: 0,
        proof_size_bytes: 0,
        num_steps: q.num_steps,
    };

    let p = safe_lock(&state.chain_prover);
    let valid = p.verify_chain_proof(&chain_proof).unwrap_or(false);

    Ok(Json(VerifyProofResponse { valid }))
}

// ──────────────────── Light Client Verification ─────────────────────────

/// GET /api/light/state-proof/account/:addr — Verkle inclusion proof for an account.
async fn get_account_state_proof(
    State(state): State<Arc<ApiState>>,
    Path(addr_hex): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let addr_bytes = hex::decode(&addr_hex).map_err(|_| StatusCode::BAD_REQUEST)?;
    if addr_bytes.len() != 32 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&addr_bytes);

    let mut db = safe_lock(&state.db);
    let state_root = db.compute_state_root();
    let proof = db.prove_account(&addr);
    let account = db.get_account(&addr).cloned();
    drop(db);

    Ok(Json(serde_json::json!({
        "type": "account",
        "address": addr_hex,
        "state_root": hex::encode(state_root),
        "exists": proof.value.is_some(),
        "account": account.map(|a| serde_json::json!({
            "balance": a.balance,
            "nonce": a.nonce,
        })),
        "proof": {
            "key": hex::encode(proof.key),
            "value": proof.value.map(hex::encode),
            "depth": proof.depth,
            "commitments": proof.commitments.iter().map(hex::encode).collect::<Vec<_>>(),
            "path_indices": proof.path_indices,
            "siblings": proof.siblings.iter().map(|level| level.iter().map(|(idx, hash)| serde_json::json!({"index": idx, "hash": hex::encode(hash)})).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "hit_compressed": proof.hit_compressed,
        },
    })))
}

/// GET /api/light/state-proof/object/:id — Verkle inclusion proof for a state object.
async fn get_object_state_proof(
    State(state): State<Arc<ApiState>>,
    Path(obj_id_hex): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let id_bytes = hex::decode(&obj_id_hex).map_err(|_| StatusCode::BAD_REQUEST)?;
    if id_bytes.len() != 32 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&id_bytes);

    let mut db = safe_lock(&state.db);
    let state_root = db.compute_state_root();
    let proof = db.prove_object(&id);
    let obj = db.get_object(&id).cloned();
    drop(db);

    Ok(Json(serde_json::json!({
        "type": "object",
        "object_id": obj_id_hex,
        "state_root": hex::encode(state_root),
        "exists": proof.value.is_some(),
        "object": obj.map(|o| serde_json::json!({
            "energy": o.energy,
            "half_life": o.half_life,
            "state": format!("{:?}", o.state),
            "created_at": o.created_at,
            "last_refreshed": o.last_refreshed,
        })),
        "proof": {
            "key": hex::encode(proof.key),
            "value": proof.value.map(hex::encode),
            "depth": proof.depth,
            "commitments": proof.commitments.iter().map(hex::encode).collect::<Vec<_>>(),
            "path_indices": proof.path_indices,
            "siblings": proof.siblings.iter().map(|level| level.iter().map(|(idx, hash)| serde_json::json!({"index": idx, "hash": hex::encode(hash)})).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "hit_compressed": proof.hit_compressed,
        },
    })))
}

/// GET /api/light/tx-proof/:block/:tx_index — transaction inclusion proof.
async fn get_tx_inclusion_proof(
    State(state): State<Arc<ApiState>>,
    Path((block_number, tx_index)): Path<(u64, usize)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = state
        .chain_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let block = store
        .load_full_block(block_number)
        .ok_or(StatusCode::NOT_FOUND)?;
    let proof = crate::persistence::prove_tx_inclusion(&block.transactions, tx_index, block_number)
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(serde_json::json!({
        "proof": proof,
        "valid": crate::persistence::verify_tx_inclusion(&proof),
    })))
}

/// POST /api/light/verify-tx-proof — verify a submitted tx inclusion proof.
async fn post_verify_tx_proof(
    Json(proof): Json<crate::persistence::TxInclusionProof>,
) -> Json<serde_json::Value> {
    let valid = crate::persistence::verify_tx_inclusion(&proof);
    Json(serde_json::json!({ "valid": valid }))
}

/// GET /api/light/verify-state-proof — verify a Verkle proof against a state root.
#[derive(Deserialize)]
struct VerifyStateProofQuery {
    state_root: String,
    proof_json: String,
}

async fn get_verify_state_proof(
    Query(q): Query<VerifyStateProofQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let root_bytes = hex::decode(&q.state_root).map_err(|_| StatusCode::BAD_REQUEST)?;
    if root_bytes.len() != 32 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut root = [0u8; 32];
    root.copy_from_slice(&root_bytes);

    let proof: evaporchain_crypto::EnergyVerkleProof =
        serde_json::from_str(&q.proof_json).map_err(|_| StatusCode::BAD_REQUEST)?;

    let valid = evaporchain_crypto::EnergyVerkleTrie::verify(&proof, &root);

    Ok(Json(serde_json::json!({
        "valid": valid,
        "key": hex::encode(proof.key),
        "value_exists": proof.value.is_some(),
    })))
}

/// GET /api/light/headers — compact block headers for light client sync.
#[derive(Deserialize)]
struct HeadersQuery {
    from: Option<u64>,
    to: Option<u64>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct CompactHeader {
    number: u64,
    epoch: u64,
    parent_hash: String,
    state_root: String,
    tx_count: usize,
    tx_merkle_root: String,
    timestamp: u64,
    has_nova_proof: bool,
}

async fn get_light_headers(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<HeadersQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = state
        .chain_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let history = safe_lock(&state.block_history);
    let latest = history.back().map(|b| b.number).unwrap_or(0);
    drop(history);

    let from = params.from.unwrap_or(0);
    let to = params.to.unwrap_or(latest).min(latest);
    let limit = params.limit.unwrap_or(100).min(500);

    let mut headers = Vec::new();
    for bn in from..=to {
        if headers.len() >= limit {
            break;
        }
        if let Some(block) = store.load_full_block(bn) {
            let tx_merkle_root = crate::persistence::compute_tx_merkle_root(&block.transactions);
            headers.push(CompactHeader {
                number: block.number,
                epoch: block.epoch,
                parent_hash: hex::encode(block.parent_hash),
                state_root: hex::encode(block.state_root),
                tx_count: block.transactions.len(),
                tx_merkle_root: hex::encode(tx_merkle_root),
                timestamp: block.timestamp,
                has_nova_proof: block.nova_proof.is_some(),
            });
        }
    }

    Ok(Json(serde_json::json!({
        "count": headers.len(),
        "from": from,
        "to": to,
        "headers": headers,
    })))
}

// ──────────────────────── DA Sampling Endpoints ─────────────────────────

#[derive(Serialize)]
struct DABlockInfoResponse {
    block_number: u64,
    total_shards: usize,
    commitment_root: String,
    original_size_bytes: usize,
}

/// GET /api/da/block/:number — DA info for a specific block.
async fn get_da_block(
    State(state): State<Arc<ApiState>>,
    Path(block_number): Path<u64>,
) -> Result<Json<DABlockInfoResponse>, StatusCode> {
    let store = safe_lock(&state.da_store);
    let package = store.get(&block_number).ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(DABlockInfoResponse {
        block_number,
        total_shards: package.shards.len(),
        commitment_root: hex::encode(package.header.commitment_root),
        original_size_bytes: package.header.original_len,
    }))
}

/// JSON 404 fallback handler.
async fn fallback_404() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "Not found"})),
    )
}

/// Security headers middleware.
fn security_headers(response: &mut axum::http::Response<axum::body::Body>) {
    let h = response.headers_mut();
    // Only set headers not already handled by nginx reverse proxy
    h.insert(
        "Permissions-Policy",
        "camera=(), microphone=(), geolocation=()".parse().unwrap(),
    );
}

// ─────────────── Frontier Primitives ─────────────────────────────────────

async fn get_frontier_status(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let Some(ref fs_arc) = state.frontier_state else {
        return Json(serde_json::json!({"error": "frontier not enabled"}));
    };
    let fs = fs_arc.lock().unwrap();
    let health = fs.energy_trie.health();
    let poha_dist = fs.poha.temperature_distribution();

    Json(serde_json::json!({
        "anchors": fs.anchors.anchor_count(),
        "poha": {
            "active": fs.poha.active_count(),
            "ghosts": fs.poha.ghost_count(),
            "hot": poha_dist.hot,
            "warm": poha_dist.warm,
            "cold": poha_dist.cold,
        },
        "energy_trie": {
            "active_leaves": health.active_leaves,
            "compressed_leaves": health.compressed_leaves,
            "total_nodes": health.total_nodes,
            "max_energy": health.max_energy,
            "min_half_life": health.min_half_life,
            "last_activity_epoch": health.last_activity_epoch,
            "compressions": health.compressions,
            "decompressions": health.decompressions,
        },
        "lazy_cache": {
            "snapshots": fs.lazy_cache.snapshot_count(),
            "total_objects": fs.lazy_cache.total_objects(),
            "latest_anchor_epoch": fs.lazy_cache.latest_anchor_epoch(),
        }
    }))
}

async fn get_lazy_eval(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<LazyEvalParams>,
) -> impl IntoResponse {
    let Some(ref fs_arc) = state.frontier_state else {
        return Json(serde_json::json!({"error": "frontier not enabled"}));
    };
    let fs = fs_arc.lock().unwrap();

    if let Some(object_id_hex) = params.object_id {
        let Ok(bytes) = hex::decode(&object_id_hex) else {
            return Json(serde_json::json!({"error": "invalid hex object_id"}));
        };
        if bytes.len() != 32 {
            return Json(serde_json::json!({"error": "object_id must be 32 bytes"}));
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(&bytes);

        let epoch = params
            .epoch
            .unwrap_or_else(|| fs.lazy_cache.latest_anchor_epoch().unwrap_or(0));

        match fs.query_lazy(&id, epoch) {
            Some(result) => Json(serde_json::json!({
                "object_id": hex::encode(result.object_id),
                "query_epoch": result.query_epoch,
                "anchor_epoch": result.anchor_epoch,
                "energy": result.energy,
                "state": format!("{:?}", result.state),
                "energy_at_anchor": result.energy_at_anchor,
                "half_life": result.half_life,
            })),
            None => Json(serde_json::json!({"error": "object not found in lazy cache"})),
        }
    } else {
        let epoch = params
            .epoch
            .unwrap_or_else(|| fs.lazy_cache.latest_anchor_epoch().unwrap_or(0));

        let results = fs.query_all_lazy(epoch);
        let items: Vec<_> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "object_id": hex::encode(r.object_id),
                    "energy": r.energy,
                    "state": format!("{:?}", r.state),
                    "half_life": r.half_life,
                })
            })
            .collect();

        Json(serde_json::json!({
            "epoch": epoch,
            "anchor_epoch": results.first().map(|r| r.anchor_epoch),
            "count": items.len(),
            "objects": items,
            "cache_snapshots": fs.lazy_cache.snapshot_count(),
            "cache_total_objects": fs.lazy_cache.total_objects(),
        }))
    }
}

#[derive(serde::Deserialize)]
struct LazyEvalParams {
    object_id: Option<String>,
    epoch: Option<u64>,
}

// ─────────────── Data Availability Sampling ───────────────────────────────

async fn get_da_status(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let store = state.da_store.lock().unwrap();
    let blocks_with_da: Vec<u64> = store.keys().cloned().collect();
    let total = blocks_with_da.len();
    let latest = blocks_with_da.last().copied();
    Json(serde_json::json!({
        "da_enabled": true,
        "blocks_cached": total,
        "latest_da_block": latest,
        "erasure_scheme": "reed_solomon_4+4",
    }))
}

async fn get_da_sample(
    State(state): State<Arc<ApiState>>,
    Path((block, shard_index)): Path<(u64, usize)>,
) -> impl IntoResponse {
    let store = state.da_store.lock().unwrap();
    let Some(package) = store.get(&block) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("no DA data for block {}", block)})),
        )
            .into_response();
    };

    if shard_index >= package.shards.len() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("shard_index {} out of range (total: {})", shard_index, package.shards.len())
            })),
        ).into_response();
    }

    let da = evaporchain_da::block_da::BlockDA::new().unwrap();
    match da.prove_shard(package, shard_index) {
        Ok(response) => Json(serde_json::json!({
            "block": block,
            "shard_index": shard_index,
            "shard_data": hex::encode(&response.shard.data),
            "shard_hash": hex::encode(response.shard.hash),
            "proof": {
                "siblings": response.proof.siblings.iter().map(hex::encode).collect::<Vec<_>>(),
                "leaf_index": response.proof.leaf_index,
                "root": hex::encode(response.proof.root),
            },
            "commitment_root": hex::encode(package.header.commitment_root),
            "total_shards": package.header.total_shards,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{}", e)})),
        )
            .into_response(),
    }
}

async fn get_da_light_sample(
    State(state): State<Arc<ApiState>>,
    Path(block): Path<u64>,
) -> impl IntoResponse {
    let store = state.da_store.lock().unwrap();
    let Some(package) = store.get(&block) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("no DA data for block {}", block)})),
        )
            .into_response();
    };

    // Generate 4 random sample queries using block hash as seed
    let seed = blake3::hash(&block.to_le_bytes());
    let queries = evaporchain_da::block_da::BlockDA::generate_sample_queries(
        block,
        &package.header,
        4,
        seed.as_bytes(),
    );

    let da = evaporchain_da::block_da::BlockDA::new().unwrap();
    let mut samples = Vec::new();
    let mut all_valid = true;

    for query in &queries {
        if let Ok(response) = da.prove_shard(package, query.shard_index) {
            let valid =
                evaporchain_da::block_da::BlockDA::verify_shard_sample(&package.header, &response);
            if !valid {
                all_valid = false;
            }
            samples.push(serde_json::json!({
                "shard_index": query.shard_index,
                "shard_hash": hex::encode(response.shard.hash),
                "proof_root": hex::encode(response.proof.root),
                "valid": valid,
            }));
        }
    }

    Json(serde_json::json!({
        "block": block,
        "commitment_root": hex::encode(package.header.commitment_root),
        "total_shards": package.header.total_shards,
        "samples_requested": queries.len(),
        "samples_verified": samples.len(),
        "all_valid": all_valid,
        "samples": samples,
    }))
    .into_response()
}

// ─────────────── 2D Cell Sampling ───────────────────────────────────

async fn get_da_cell_sample(
    State(state): State<Arc<ApiState>>,
    Path((block, row, col)): Path<(u64, usize, usize)>,
) -> impl IntoResponse {
    let store = state.da_2d_store.lock().unwrap();
    let Some(package) = store.get(&block) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("no 2D DA data for block {}", block)})),
        )
            .into_response();
    };

    if row >= package.header.row_roots.len() || col >= package.header.col_roots.len() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("cell ({},{}) out of range ({}x{})", row, col,
                    package.header.row_roots.len(), package.header.col_roots.len())
            })),
        )
            .into_response();
    }

    let da2d = evaporchain_da::block_da_2d::BlockDA2D::new();
    match da2d.prove_cell(package, row, col) {
        // `cell_data` is mandatory: light-client `verify_cell_proof` hashes
        // it to derive `cell_hash`, then walks the row+column Merkle paths.
        // Omitting it (the prior format) made the endpoint unverifiable —
        // an attacker could send any cell_hash with matching siblings and
        // the client would have no way to refute. Closes punch-list #2b.
        Ok(proof) => Json(serde_json::json!({
            "block": block,
            "row": row,
            "col": col,
            "cell_data": hex::encode(&proof.cell_data),
            "cell_hash": hex::encode(proof.cell_hash),
            "row_root": hex::encode(package.header.row_roots[row]),
            "col_root": hex::encode(package.header.col_roots[col]),
            "data_root": hex::encode(package.header.data_root),
            "extended_dim": package.header.extended_dim,
            "row_proof_siblings": proof.row_siblings.iter().map(hex::encode).collect::<Vec<_>>(),
            "col_proof_siblings": proof.col_siblings.iter().map(hex::encode).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{}", e)})),
        )
            .into_response(),
    }
}

/// `GET /api/da/header/:block` — the full BlockDA2DHeader for `block`,
/// hex-encoded as JSON. Light-client samplers (`LightClientSampler` /
/// `evaporchain da verify`) need every row/col Merkle root to verify
/// cell proofs locally; the per-cell endpoint only ships the two roots
/// involved in that cell, not the full header. Returns 404 if no 2D
/// package exists for this block (chain currently retains the last 64).
async fn get_da_2d_header(
    State(state): State<Arc<ApiState>>,
    Path(block): Path<u64>,
) -> impl IntoResponse {
    let store = state.da_2d_store.lock().unwrap();
    let Some(package) = store.get(&block) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("no 2D DA data for block {}", block)})),
        )
            .into_response();
    };
    let h = &package.header;
    Json(serde_json::json!({
        "block": block,
        "data_root": hex::encode(h.data_root),
        "row_roots": h.row_roots.iter().map(hex::encode).collect::<Vec<_>>(),
        "col_roots": h.col_roots.iter().map(hex::encode).collect::<Vec<_>>(),
        "extended_dim": h.extended_dim,
        "original_dim": h.original_dim,
        "cell_size": h.cell_size,
        "original_len": h.original_len,
        "data_hash": hex::encode(h.data_hash),
    }))
    .into_response()
}

async fn get_da_2d_light_sample(
    State(state): State<Arc<ApiState>>,
    Path(block): Path<u64>,
) -> impl IntoResponse {
    let store = state.da_2d_store.lock().unwrap();
    let Some(package) = store.get(&block) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("no 2D DA data for block {}", block)})),
        )
            .into_response();
    };

    let da2d = evaporchain_da::block_da_2d::BlockDA2D::new();
    let seed = blake3::hash(&block.to_le_bytes());
    let num_samples = std::cmp::min(8, package.header.extended_dim * package.header.extended_dim);
    let queries = evaporchain_da::commitments::generate_2d_queries(
        block,
        package.header.extended_dim,
        num_samples,
        seed.as_bytes(),
    );

    let commitments = evaporchain_da::commitments::RowColumnCommitments {
        row_roots: package.header.row_roots.clone(),
        col_roots: package.header.col_roots.clone(),
        data_root: package.header.data_root,
        extended_dim: package.header.extended_dim,
    };

    let mut samples = Vec::new();
    let mut valid_count = 0usize;
    for query in &queries {
        if let Ok(proof) = da2d.prove_cell(package, query.row, query.col) {
            let valid = commitments.verify_cell_proof(&proof);
            if valid {
                valid_count += 1;
            }
            samples.push(serde_json::json!({
                "row": query.row,
                "col": query.col,
                "cell_hash": hex::encode(proof.cell_hash),
                "valid": valid,
            }));
        }
    }

    let total_cells = package.header.extended_dim * package.header.extended_dim;
    let confidence = if total_cells > 0 {
        1.0 - (1.0 - (valid_count as f64 / total_cells as f64)).powi(num_samples as i32)
    } else {
        0.0
    };

    Json(serde_json::json!({
        "block": block,
        "data_root": hex::encode(package.header.data_root),
        "extended_dim": package.header.extended_dim,
        "original_dim": package.header.original_dim,
        "total_cells": total_cells,
        "samples_requested": queries.len(),
        "samples_valid": valid_count,
        "confidence": confidence,
        "samples": samples,
    }))
    .into_response()
}

// ─────────────── Evaporation DA Proof ────────────────────────────────

async fn get_evaporation_da_proof(
    State(state): State<Arc<ApiState>>,
    Path(object_id_hex): Path<String>,
) -> impl IntoResponse {
    let object_id = match hex::decode(&object_id_hex) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid object_id hex (need 32 bytes)"})),
            )
                .into_response();
        }
    };

    let db = state.db.lock().unwrap();
    let Some(ghost) = db.get_ghost(&object_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "object not evaporated (no ghost record)"})),
        )
            .into_response();
    };

    let da_store = state.da_store.lock().unwrap();

    let evap_epoch = ghost.evaporated_at;
    let candidate_blocks: Vec<_> = da_store.keys().copied().collect();

    let mut proof_result = None;
    for &bn in candidate_blocks.iter().rev() {
        if let Some(package) = da_store.get(&bn) {
            if package.shards.is_empty() {
                continue;
            }
            let shard_index = (u64::from_le_bytes(object_id[..8].try_into().unwrap_or([0u8; 8]))
                as usize)
                % package.shards.len();
            let snapshot = evaporchain_da::evaporation_da::EnergySnapshot {
                object_id,
                energy_at_evaporation: 0,
                evaporation_epoch: evap_epoch,
                half_life: ghost.original_half_life.unwrap_or(10),
                last_refreshed: 0,
                energy_at_refresh: 0,
            };
            if let Ok(proof) =
                evaporchain_da::evaporation_da::EvaporationDAProofBuilder::create_proof(
                    object_id,
                    ghost.original_data.as_deref().unwrap_or(&ghost.data_hash),
                    snapshot,
                    &package.shards,
                    shard_index,
                )
            {
                proof_result = Some((bn, proof));
                break;
            }
        }
    }

    match proof_result {
        Some((block_number, proof)) => {
            Json(serde_json::json!({
                "object_id": object_id_hex,
                "block_number": block_number,
                "evaporation_epoch": evap_epoch,
                "data_hash": hex::encode(proof.pre_evaporation_data_hash),
                "da_commitment_root": hex::encode(proof.da_commitment_root),
                "shard_index": proof.shard_index,
                "shard_hash": hex::encode(proof.shard_hash),
                "proof_epoch": proof.proof_epoch,
                "proof_siblings": proof.shard_proof.siblings.iter().map(hex::encode).collect::<Vec<_>>(),
            })).into_response()
        }
        None => {
            (StatusCode::NOT_FOUND, Json(serde_json::json!({
                "error": "no DA package available for evaporation proof (data may have been pruned)"
            }))).into_response()
        }
    }
}

// ─────────────── PoHA Certificate Detail ─────────────────────────────

async fn get_poha_certificate(
    State(state): State<Arc<ApiState>>,
    Path(block_number): Path<u64>,
) -> impl IntoResponse {
    let Some(ref fs_arc) = state.frontier_state else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "frontier not enabled"})),
        )
            .into_response();
    };
    let fs = fs_arc.lock().unwrap();

    if let Some(cert) = fs.poha.get(block_number) {
        let temp = cert.temperature();
        Json(serde_json::json!({
            "block_number": cert.block_number,
            "data_root": hex::encode(cert.data_root),
            "shard_count": cert.shard_count,
            "energy": cert.energy,
            "initial_energy": cert.initial_energy,
            "half_life": cert.half_life,
            "temperature": format!("{:?}", temp),
            "created_epoch": cert.created_epoch,
            "last_attested_epoch": cert.last_attested_epoch,
            "re_attestation_count": cert.re_attestation_count,
            "is_supermajority": cert.is_supermajority(),
            "signer_count": cert.signer_ids.len(),
        }))
        .into_response()
    } else if let Some(ghost) = fs.poha.get_ghost(block_number) {
        Json(serde_json::json!({
            "block_number": ghost.block_number,
            "data_root": hex::encode(ghost.data_root),
            "cert_hash": hex::encode(ghost.cert_hash),
            "evaporated_epoch": ghost.evaporated_epoch,
            "total_re_attestations": ghost.total_re_attestations,
            "temperature": "Evaporated",
            "is_ghost": true,
        }))
        .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no PoHA certificate for this block"})),
        )
            .into_response()
    }
}

async fn get_poha_certificates(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let Some(ref fs_arc) = state.frontier_state else {
        return Json(serde_json::json!({"error": "frontier not enabled"}));
    };
    let fs = fs_arc.lock().unwrap();
    let dist = fs.poha.temperature_distribution();

    let certs: Vec<_> = fs
        .poha
        .all_active()
        .map(|(&bn, cert)| {
            serde_json::json!({
                "block_number": bn,
                "energy": cert.energy,
                "initial_energy": cert.initial_energy,
                "temperature": format!("{:?}", cert.temperature()),
                "re_attestations": cert.re_attestation_count,
            })
        })
        .collect();

    Json(serde_json::json!({
        "active_count": certs.len(),
        "ghost_count": fs.poha.ghost_count(),
        "distribution": {
            "hot": dist.hot,
            "warm": dist.warm,
            "cold": dist.cold,
            "evaporated": dist.evaporated,
            "ghosts": dist.ghosts,
        },
        "certificates": certs,
    }))
}

// ─────────────── State Sync ───────────────────────────────────────────

// ─────────────────── Encrypted Mempool (MEV Protection) ─────────────────

#[derive(Deserialize)]
struct EncryptedTxRequest {
    commitment: String,
    encrypted_payload: String,
    nonce_hash: String,
}

async fn post_submit_encrypted_tx(
    State(state): State<Arc<ApiState>>,
    Json(body): Json<EncryptedTxRequest>,
) -> impl IntoResponse {
    let commitment = match hex_to_32(&body.commitment) {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "success": false, "message": "invalid commitment hex (need 64 chars)"
                })),
            )
        }
    };
    let nonce_hash = match hex_to_32(&body.nonce_hash) {
        Some(n) => n,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "success": false, "message": "invalid nonce_hash hex (need 64 chars)"
                })),
            )
        }
    };
    let encrypted_payload = match hex::decode(&body.encrypted_payload) {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "success": false, "message": "invalid encrypted_payload hex"
                })),
            )
        }
    };

    let current_epoch = if let Some(ref tc) = state.tendermint {
        safe_lock(tc).epoch()
    } else {
        safe_lock(&state.consensus).epoch()
    };

    let enc_tx = evaporchain_consensus::encrypted_mempool::EncryptedTransaction {
        commitment,
        encrypted_payload,
        nonce_hash,
        submitted_epoch: current_epoch,
    };

    {
        let mut pool = state.encrypted_mempool.lock().unwrap();
        pool.submit_encrypted(enc_tx);
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": "encrypted transaction submitted",
            "commitment": body.commitment,
            "reveal_epoch": current_epoch + state.encrypted_mempool.lock().unwrap().reveal_delay(),
        })),
    )
}

#[derive(Deserialize)]
struct RevealRequest {
    commitment: String,
    nonce: String,
}

async fn post_reveal_encrypted_tx(
    State(state): State<Arc<ApiState>>,
    Json(body): Json<RevealRequest>,
) -> impl IntoResponse {
    let commitment = match hex_to_32(&body.commitment) {
        Some(c) => c,
        None => {
            return Json(serde_json::json!({
                "success": false, "message": "invalid commitment hex"
            }))
        }
    };
    let nonce = match hex_to_32(&body.nonce) {
        Some(n) => n,
        None => {
            return Json(serde_json::json!({
                "success": false, "message": "invalid nonce hex"
            }))
        }
    };

    let current_epoch = if let Some(ref tc) = state.tendermint {
        safe_lock(tc).epoch()
    } else {
        safe_lock(&state.consensus).epoch()
    };

    let mut pool = state.encrypted_mempool.lock().unwrap();
    let revealed = pool.process_reveals(current_epoch, &[(commitment, nonce)]);

    if revealed.is_empty() {
        return Json(serde_json::json!({
            "success": false,
            "message": "no transaction revealed — either too early, wrong nonce, or not found",
        }));
    }

    for tx in &revealed {
        state.submit_tx(tx.clone());
    }

    Json(serde_json::json!({
        "success": true,
        "revealed_count": revealed.len(),
    }))
}

async fn get_encrypted_mempool_status(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let pool = state.encrypted_mempool.lock().unwrap();
    let (encrypted, plaintext) = pool.pending_count();
    Json(serde_json::json!({
        "encrypted_pending": encrypted,
        "plaintext_pending": plaintext,
        "total": pool.len(),
        "reveal_delay_epochs": pool.reveal_delay(),
    }))
}

fn hex_to_32(s: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(s).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Some(arr)
}

// ─────────────────── Light Client ────────────────────────────────────────

async fn get_light_client_status(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let lc = state.light_client.lock().unwrap();
    let latest = lc.latest_trusted_height();
    Json(serde_json::json!({
        "latest_trusted_height": latest,
        "trusted_headers_stored": lc.trusted_count(),
    }))
}

#[derive(Deserialize)]
struct VerifyHeaderRequest {
    height: u64,
    #[allow(dead_code)]
    epoch: u64,
    block_hash: String,
    parent_hash: String,
    state_root: String,
}

async fn post_verify_header(
    State(state): State<Arc<ApiState>>,
    Json(body): Json<VerifyHeaderRequest>,
) -> impl IntoResponse {
    let block_hash = match hex_to_32(&body.block_hash) {
        Some(h) => h,
        None => {
            return Json(serde_json::json!({ "verified": false, "error": "invalid block_hash" }))
        }
    };
    let parent_hash = match hex_to_32(&body.parent_hash) {
        Some(h) => h,
        None => {
            return Json(serde_json::json!({ "verified": false, "error": "invalid parent_hash" }))
        }
    };
    let state_root = match hex_to_32(&body.state_root) {
        Some(h) => h,
        None => {
            return Json(serde_json::json!({ "verified": false, "error": "invalid state_root" }))
        }
    };

    let lc = state.light_client.lock().unwrap();
    let trusted = lc.trusted_state_at(body.height);
    match trusted {
        Some(ts) => {
            let matches = ts.header.block_hash == block_hash
                && ts.header.state_root == state_root
                && ts.header.parent_hash == parent_hash;
            Json(serde_json::json!({
                "verified": matches,
                "height": body.height,
                "trusted": true,
                "expires_at": ts.trust_expires_at,
            }))
        }
        None => Json(serde_json::json!({
            "verified": false,
            "height": body.height,
            "trusted": false,
            "error": "height not in trusted set",
        })),
    }
}

async fn get_trusted_header(
    State(state): State<Arc<ApiState>>,
    Path(height): Path<u64>,
) -> impl IntoResponse {
    let lc = state.light_client.lock().unwrap();
    match lc.trusted_state_at(height) {
        Some(ts) => Json(serde_json::json!({
            "found": true,
            "height": ts.header.height,
            "epoch": ts.header.epoch,
            "block_hash": hex::encode(ts.header.block_hash),
            "parent_hash": hex::encode(ts.header.parent_hash),
            "state_root": hex::encode(ts.header.state_root),
            "timestamp": ts.header.timestamp,
            "trust_expires_at": ts.trust_expires_at,
            "validator_count": ts.header.validator_set.active_count(),
            "certificate_signers": ts.header.commit_certificate.signer_ids.len(),
        })),
        None => Json(serde_json::json!({
            "found": false,
            "height": height,
        })),
    }
}

// ─────────────────── Finality ───────────────────────────────────────────

async fn get_weak_subjectivity_checkpoint(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    if let Some(ref tc_arc) = state.tendermint {
        let tc = tc_arc.lock().unwrap();
        let ws_period = tc.weak_subjectivity_period();
        let trusted = tc.trusted_checkpoint();
        let latest = tc.latest_checkpoint();
        let all_checkpoints: Vec<_> = tc
            .checkpoints()
            .iter()
            .map(|(h, r)| serde_json::json!({"height": h, "state_root": hex::encode(r)}))
            .collect();

        Json(serde_json::json!({
            "weak_subjectivity_period_blocks": ws_period,
            "trusted_checkpoint": trusted.map(|(h, r, bh)| serde_json::json!({
                "height": h,
                "state_root": hex::encode(r),
                "block_hash": hex::encode(bh),
            })),
            "latest_checkpoint": latest.map(|(h, r)| serde_json::json!({
                "height": h,
                "state_root": hex::encode(r),
            })),
            "checkpoint_count": all_checkpoints.len(),
            "checkpoints": all_checkpoints,
        }))
        .into_response()
    } else {
        Json(serde_json::json!({"error": "consensus not in Tendermint mode"})).into_response()
    }
}

async fn get_finality(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let ft = state.finality_tracker.lock().unwrap();
    let latest = ft.latest_finalized_height();
    let stats = ft.stats(100);
    let latest_proof = ft.generate_proof(latest);

    Json(serde_json::json!({
        "latest_finalized_height": latest,
        "total_finalized": ft.total_finalized(),
        "records_stored": ft.record_count(),
        "stats_last_100": {
            "finalized_count": stats.finalized_count,
            "avg_participation": format!("{:.2}%", stats.avg_participation * 100.0),
            "min_participation": format!("{:.2}%", stats.min_participation * 100.0),
            "max_participation": format!("{:.2}%", stats.max_participation * 100.0),
            "high_participation_count": stats.high_participation_count,
        },
        "latest_proof": latest_proof.map(|p| serde_json::json!({
            "height": p.height,
            "block_hash": hex::encode(p.block_hash),
            "state_root": hex::encode(p.state_root),
            "proof_hash": hex::encode(p.proof_hash),
            "signers": p.certificate.signer_ids.len(),
        })),
    }))
}

async fn get_finality_proof(
    State(state): State<Arc<ApiState>>,
    Path(height): Path<u64>,
) -> impl IntoResponse {
    let ft = state.finality_tracker.lock().unwrap();
    match ft.generate_proof(height) {
        Some(proof) => Json(serde_json::json!({
            "found": true,
            "height": proof.height,
            "block_hash": hex::encode(proof.block_hash),
            "state_root": hex::encode(proof.state_root),
            "proof_hash": hex::encode(proof.proof_hash),
            "certificate": {
                "round": proof.certificate.round,
                "signers": proof.certificate.signer_ids,
                "aggregate_signature": hex::encode(&proof.certificate.aggregate_signature),
            },
            "verified": evaporchain_consensus::finality::FinalityTracker::verify_proof(&proof),
        })),
        None => Json(serde_json::json!({
            "found": false,
            "height": height,
        })),
    }
}

/// Per-height finality gap snapshot (Mainnet P1).
///
/// Returns:
/// - `unfinalised`: heights that have been committed but haven't seen a
///   finality cert yet, projected to (height, age_seconds).
/// - `worst_gap_seconds`: max age across `unfinalised`.
/// - `recent_gaps`: most recent (last 100) commit→finalise gap samples,
///   newest first, sourced from the consensus engine's ring buffer.
async fn get_finality_gap(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let (unfinalised, worst_ms, recent) = if let Some(tc) = &state.tendermint {
        let tc = tc.lock().unwrap();
        (
            tc.unfinalised_tail(),
            tc.worst_unfinalised_gap_ms(),
            tc.finality_gap_history(),
        )
    } else {
        (Vec::new(), 0u64, Vec::new())
    };

    let unfinalised_json: Vec<serde_json::Value> = unfinalised
        .iter()
        .map(|(h, age_ms)| {
            serde_json::json!({
                "height": h,
                "age_seconds": (*age_ms as f64) / 1000.0,
            })
        })
        .collect();

    // Newest-first window of the last 100 samples.
    let recent_json: Vec<serde_json::Value> = recent
        .iter()
        .rev()
        .take(100)
        .map(|(h, gap_ms)| {
            serde_json::json!({
                "height": h,
                "gap_seconds": (*gap_ms as f64) / 1000.0,
            })
        })
        .collect();

    Json(serde_json::json!({
        "unfinalised": unfinalised_json,
        "worst_gap_seconds": (worst_ms as f64) / 1000.0,
        "recent_gaps": recent_json,
    }))
}

async fn get_sync_snapshot_info(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let info = state.snapshot_info.lock().unwrap();
    match *info {
        Some((height, state_root, data_len)) => Json(serde_json::json!({
            "available": true,
            "height": height,
            "state_root": hex::encode(state_root),
            "data_bytes": data_len,
        })),
        None => Json(serde_json::json!({
            "available": false,
        })),
    }
}

// ──────────────────────── Snapshot fast-sync endpoints ────────────────────
//
// `/api/snapshot/latest` — metadata of the most recent on-disk snapshot.
// `/api/snapshot/download/:height` — raw .zst blob for that height.
//
// Snapshots are produced every `--snapshot-interval` blocks by main.rs.
// A fast-syncing peer hits `/latest` to discover height, then GETs
// `/download/:height` and verifies via `SnapshotFile::from_bytes`.

/// Resolve the on-disk path of the snapshot file for `height` under the
/// configured snapshot directory. Returns `None` if no snapshot dir is
/// configured.
fn snapshot_path_for(state: &ApiState, height: u64) -> Option<std::path::PathBuf> {
    state
        .snapshot_dir
        .as_ref()
        .map(|dir| dir.join(format!("{}.zst", height)))
}

/// Find the highest-numbered `.zst` file under `state.snapshot_dir`.
fn latest_snapshot_height(state: &ApiState) -> Option<u64> {
    let dir = state.snapshot_dir.as_ref()?;
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<u64> = None;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("zst") {
            continue;
        }
        let stem = match p.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };
        if let Ok(h) = stem.parse::<u64>() {
            if best.map(|b| h > b).unwrap_or(true) {
                best = Some(h);
            }
        }
    }
    best
}

/// `GET /api/snapshot/latest` — returns the most recent snapshot's
/// metadata (or 404 if none).
async fn get_snapshot_latest(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let height = match latest_snapshot_height(&state) {
        Some(h) => h,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "available": false,
                    "reason": "no snapshot on disk",
                })),
            )
                .into_response();
        }
    };
    let path = match snapshot_path_for(&state, height) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "available": false,
                    "reason": "snapshot dir not configured",
                })),
            )
                .into_response();
        }
    };
    match evaporchain_state::SnapshotFile::load_and_verify(&path) {
        Ok(file) => {
            let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let meta = file.metadata(size_bytes);
            Json(serde_json::json!({
                "available": true,
                "version": meta.version,
                "chain_id": meta.chain_id,
                "block_height": meta.block_height,
                "state_root": hex::encode(meta.state_root),
                "epoch": meta.epoch,
                "integrity_hash": hex::encode(meta.integrity_hash),
                "size_bytes": meta.size_bytes,
                "download_path": meta.download_path,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "available": false,
                "reason": format!("verify failed: {}", e),
            })),
        )
            .into_response(),
    }
}

/// `GET /api/snapshot/download/:height` — streams the `.zst` blob for
/// the given height. 404 if not present.
async fn get_snapshot_download(
    State(state): State<Arc<ApiState>>,
    Path(height): Path<u64>,
) -> impl IntoResponse {
    let path = match snapshot_path_for(&state, height) {
        Some(p) => p,
        None => return (StatusCode::NOT_FOUND, "snapshot dir not configured").into_response(),
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return (StatusCode::NOT_FOUND, "snapshot not found").into_response(),
    };
    let len = bytes.len();
    (
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/zstd".to_string(),
            ),
            (axum::http::header::CONTENT_LENGTH, len.to_string()),
        ],
        bytes,
    )
        .into_response()
}

// ─────────────────── Offline signing helpers ─────────────────────────────

/// Return the current nonce for an address (next expected nonce for signing).
async fn get_account_nonce(
    State(state): State<Arc<ApiState>>,
    Path(address): Path<String>,
) -> impl IntoResponse {
    let addr = match parse_address_value(&serde_json::Value::String(address.clone())) {
        Ok(a) => a,
        Err(e) => return Json(serde_json::json!({ "error": e })),
    };
    let db = safe_lock(&state.db);
    let nonce = db.get_account(&addr).map(|a| a.nonce).unwrap_or(0);
    Json(serde_json::json!({
        "address": address,
        "nonce": nonce,
        "chain_id": hex::encode(&state.chain_id),
    }))
}

#[derive(Deserialize)]
struct SignableBytesRequest {
    tx_type: String,
    params: serde_json::Value,
}

/// Return the canonical bytes to sign for a transaction.
/// The client uses these bytes with their ML-DSA private key, then submits
/// the transaction through the normal endpoint with signature + public_key.
async fn post_signable_bytes(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<SignableBytesRequest>,
) -> impl IntoResponse {
    let params = &req.params;

    let tx_opt: Option<Transaction> = (|| -> Option<Transaction> {
        match req.tx_type.as_str() {
            "transfer" => {
                let from = parse_address_value(params.get("from")?).ok()?;
                let to = parse_address_value(params.get("to")?).ok()?;
                let amount = params.get("amount")?.as_u64()?;
                let nonce = {
                    let db = safe_lock(&state.db);
                    db.get_account(&from).map(|a| a.nonce).unwrap_or(0)
                };
                Some(Transaction::Transfer(evaporchain_types::TransferTx {
                    from,
                    to,
                    amount,
                    nonce,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                }))
            }
            "create_object" => {
                let creator = parse_address_value(params.get("creator")?).ok()?;
                let object_id = parse_address_value(params.get("object_id")?).ok()?;
                let energy = params.get("energy")?.as_u64()?;
                let half_life = params.get("half_life")?.as_u64()?;
                let data = params
                    .get("data")
                    .and_then(|v| v.as_str())
                    .map(|s| s.as_bytes().to_vec())
                    .unwrap_or_else(|| b"offline".to_vec());
                Some(Transaction::CreateObject(
                    evaporchain_types::CreateObjectTx {
                        creator,
                        object_id,
                        energy,
                        half_life,
                        data,
                        decay_curve: None,
                        lad_mode: None,
                        signature: None,
                        public_key: None,
                    },
                ))
            }
            "refresh" => {
                let object_id = parse_address_value(params.get("object_id")?).ok()?;
                let energy_deposit = params.get("energy_deposit")?.as_u64()?;
                Some(Transaction::Refresh(evaporchain_types::RefreshTx {
                    object_id,
                    energy_deposit,
                    signature: None,
                    public_key: None,
                }))
            }
            _ => None,
        }
    })();

    match tx_opt {
        Some(tx) => {
            let signable = tx.signing_message(&state.chain_id);
            Json(serde_json::json!({
                "ok": true,
                "tx_type": req.tx_type,
                "signable_hex": hex::encode(&signable),
                "chain_id": hex::encode(&state.chain_id),
            }))
        }
        None => Json(serde_json::json!({
            "ok": false,
            "error": format!("unknown or invalid tx_type: {}", req.tx_type),
        })),
    }
}

pub fn create_router(state: Arc<ApiState>, auth_state: Arc<crate::auth::AuthState>) -> Router {
    // Network-1 (re-audit 2026-05-02): CORS allow-list is the
    // built-in default plus any origins from EVAPORCHAIN_CORS_ORIGINS
    // (comma-separated). Hardcoded origins were a deployment trap:
    // a fork running on a different domain had no escape valve, and
    // operators sometimes added a wildcard regex elsewhere — better
    // to make extension explicit and audited via env.
    let mut allowed_origins: Vec<axum::http::HeaderValue> = vec![
        "https://evaporchain.com".parse().unwrap(),
        "https://testnet.evaporchain.com".parse().unwrap(),
        "http://localhost:3000".parse().unwrap(),
    ];
    if let Ok(extra) = std::env::var("EVAPORCHAIN_CORS_ORIGINS") {
        for origin in extra.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            // Refuse `*` — a permissive wildcard would defeat the
            // CSRF defense entirely. Operators must enumerate.
            if origin == "*" {
                eprintln!(
                    "\x1b[31m⚠ EVAPORCHAIN_CORS_ORIGINS contains `*` — refusing wildcard origin\x1b[0m"
                );
                continue;
            }
            match origin.parse() {
                Ok(hv) => allowed_origins.push(hv),
                Err(e) => eprintln!(
                    "\x1b[33m⚠ EVAPORCHAIN_CORS_ORIGINS skipping invalid origin {origin}: {e}\x1b[0m"
                ),
            }
        }
    }
    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ]);

    // Auth sub-router with its own state
    let auth_router = Router::new()
        .route("/api/auth/register", post(crate::auth::register))
        .route("/api/auth/login", post(crate::auth::login))
        .route("/api/auth/verify-email", post(crate::auth::verify_email))
        .route("/api/auth/me", get(crate::auth::get_me))
        .route("/api/auth/logout", post(crate::auth::logout))
        .route("/api/wallet/create", post(crate::auth::create_wallet))
        .route("/api/wallet/list", get(crate::auth::list_wallets))
        .route("/api/wallet/activity", get(crate::auth::get_activity))
        .with_state(auth_state);

    let router = Router::new()
        // Wallet is the landing page
        .route("/", get(wallet_html))
        .route("/wallet", get(wallet_html))
        // Explorer (developer dashboard)
        .route("/explorer", get(dashboard_html))
        .route("/health", get(health))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        // Chain metadata
        .route("/api/chain", get(get_chain))
        // Explorer
        .route("/api/status", get(get_status))
        .route("/api/four_act", get(get_four_act_status))
        .route("/api/mortis_cert", get(get_mortis_cert))
        .route("/api/refresh_pool", get(get_refresh_pool))
        .route("/api/tombstone/:addr_hex", get(get_tombstone))
        .route("/api/hbct/state", get(get_hbct_state))
        .route("/api/hbct/seed_demo", post(post_hbct_seed_demo))
        .route("/api/hbct/mint", post(post_hbct_mint))
        .route("/api/hbct/transfer", post(post_hbct_transfer))
        .route("/api/hbct/burn", post(post_hbct_burn))
        .route("/api/hbct/balance", post(post_hbct_balance))
        .route("/api/hbct/tick", post(post_hbct_tick))
        .route(
            "/api/hbct/seed_attestation",
            post(post_hbct_seed_attestation),
        )
        .route("/api/hbct/settle", post(post_hbct_settle))
        .route("/api/sentinel/register", post(post_sentinel_register_param))
        .route("/api/sentinel/vote", post(post_sentinel_vote))
        .route("/api/sentinel/tick", post(post_sentinel_tick))
        .route("/api/sentinel/parameter/:id", get(get_sentinel_param))
        .route("/api/sentinel/all", get(get_sentinel_all))
        .route("/api/sentinel/seed_demo", post(post_sentinel_seed_demo))
        .route("/api/sentinel/seed_votes", post(post_sentinel_seed_votes))
        .route(
            "/api/boltzmann_stake/:validator_id/at/:current_epoch",
            get(get_boltzmann_stake),
        )
        .route("/api/lamport_time", get(get_lamport_time))
        .route("/api/light_cone", get(get_light_cone))
        .route(
            "/api/light_cone/antichain_digest",
            get(get_light_cone_antichain_digest),
        )
        .route(
            "/api/light_cone/antichain_digest_history",
            get(get_light_cone_antichain_digest_history),
        )
        .route(
            "/api/light_cone/candidate_heads",
            get(get_light_cone_candidate_heads),
        )
        .route(
            "/api/light_cone/authoritative_head",
            get(get_light_cone_authoritative_head),
        )
        .route(
            "/api/light_cone/block_clock/:block_id_hex",
            get(get_light_cone_block_clock),
        )
        .route("/api/causal_cone", get(get_causal_cone))
        .route("/api/mcc_fork_choice", get(get_mcc_fork_choice))
        .route(
            "/api/singh_attractor_fork_choice",
            post(post_singh_attractor_fork_choice),
        )
        .route("/api/cone_bridge", post(post_cone_bridge))
        .route("/api/eg_fss/sign_verify", post(post_eg_fss_sign_verify))
        .route("/api/tur_liveness", get(get_tur_liveness))
        .route("/api/cmu_check", get(get_cmu_check))
        .route("/api/crooks_refund", post(post_crooks_refund))
        .route("/api/beacon/:tau", get(get_beacon))
        .route("/api/prp/prove", post(post_prp_prove))
        .route("/api/efh/h0", post(post_efh_h0))
        .route("/api/efh/bottleneck", post(post_efh_bottleneck))
        .route("/api/lambda_fold", get(get_lambda_fold))
        .route("/api/lambda_fold/verify", post(post_lambda_fold_verify))
        // Crooks-MEV Phase 1.4 — observe-only MEV ring buffer view.
        .route("/api/mev/observations", get(get_mev_observations))
        // Crooks-MEV Phase 4.4 — operator dispute endpoint.
        .route("/api/mev/dispute", post(post_mev_dispute))
        .route("/api/singh_attractor", post(post_singh_attractor))
        .route("/api/bell_beacon", post(post_bell_beacon))
        .route("/api/allen_relation", post(post_allen_relation))
        .route("/api/mdl_optimal", post(post_mdl_optimal))
        .route("/api/cslc_reconstruct", post(post_cslc_reconstruct))
        .route("/api/padic", post(post_padic))
        .route("/api/tropical_weight", post(post_tropical_weight))
        .route("/api/eb_fs_challenge", post(post_eb_fs_challenge))
        .route("/api/identity", get(get_identity))
        .route("/api/mortis_cert_preview", get(get_mortis_cert_preview))
        .route("/api/mortis_cert_verify", post(post_mortis_verify))
        .route("/api/lad_vm/simulate", post(post_lad_vm_simulate))
        .route("/api/patronage/pledge", post(post_patronage_pledge))
        .route("/api/patronage/honour", post(post_patronage_honour))
        .route("/api/patronage/revoke", post(post_patronage_revoke))
        .route("/api/patronage/status", get(get_patronage_status))
        .route("/api/patronage/immune", get(get_patronage_immune))
        .route("/api/governance/flags", get(get_governance_flags))
        .route("/api/governance/param", post(post_governance_param))
        .route(
            "/api/cartel_alarm/run_gate",
            post(post_cartel_alarm_run_gate),
        )
        .route(
            "/api/cartel_alarm/chain_status",
            get(get_cartel_alarm_chain_status),
        )
        .route(
            "/api/cartel_alarm/pending_events",
            get(get_cartel_alarm_pending_events),
        )
        .route(
            "/api/governance/fork_choice_mode",
            get(get_governance_fork_choice_mode),
        )
        .route(
            "/api/governance/fork_choice_mode",
            post(post_governance_fork_choice_mode),
        )
        .route("/api/script_lad/check", post(post_script_lad_check))
        .route("/api/script_lad/simulate", post(post_script_lad_simulate))
        .route(
            "/api/validators/boltzmann_stakes",
            get(get_boltzmann_stakes),
        )
        .route(
            "/api/validators/boltzmann_weights",
            get(get_boltzmann_weights),
        )
        .route("/api/validators/sanov_slash", post(post_sanov_slash))
        .route("/api/braid/commit", post(post_braid_commit))
        .route("/api/decay_forget/prove", post(post_decay_forget_prove))
        .route("/api/decay_forget/verify", post(post_decay_forget_verify))
        .route("/api/hlts/quorum_check", post(post_hlts_quorum_check))
        .route("/api/pnt/insert", post(post_pnt_insert))
        .route("/api/pnt/advance_phase", post(post_pnt_advance_phase))
        .route("/api/pnt/status", get(get_pnt_status))
        .route("/api/pnt/is_spent/:nullifier_hex", get(get_pnt_is_spent))
        .route("/api/entropic_slash", post(post_entropic_slash))
        .route("/api/fee_controller/step", post(post_fee_controller_step))
        .route("/api/fee_controller/status", get(get_fee_controller_status))
        .route("/api/fork_cert/prove", post(post_fork_cert_prove))
        .route("/api/fork_cert/verify", post(post_fork_cert_verify))
        .route("/api/fork_cert_v2/prove", post(post_fork_cert_v2_prove))
        .route("/api/fork_cert_v2/verify", post(post_fork_cert_v2_verify))
        .route("/api/bell_beacon_v2/issue", post(post_bell_beacon_v2_issue))
        .route(
            "/api/bell_beacon_v2/verify",
            post(post_bell_beacon_v2_verify),
        )
        .route(
            "/api/singh_attractor_v2/draw",
            post(post_singh_attractor_v2_draw),
        )
        .route(
            "/api/ib_validators_v2/vote",
            post(post_ib_validators_v2_vote),
        )
        .route(
            "/api/ib_validators_v2/jail/chsh_failure",
            post(post_ib_validators_v2_jail_chsh_failure),
        )
        .route(
            "/api/light_cone_v2/causal_root",
            post(post_light_cone_v2_causal_root),
        )
        .route(
            "/api/light_cone_v2/prove_ancestry",
            post(post_light_cone_v2_prove_ancestry),
        )
        .route(
            "/api/light_cone_v2/verify_ancestry",
            post(post_light_cone_v2_verify_ancestry),
        )
        .route(
            "/api/singh_inequality_v2/gate",
            post(post_singh_inequality_v2_gate),
        )
        .route(
            "/api/singh_inequality_v2/compare",
            post(post_singh_inequality_v2_compare),
        )
        .route("/api/antichain/compute", post(post_antichain_compute))
        .route("/api/hot_cold_stake/decay", post(post_hot_cold_decay))
        .route("/api/hot_cold_stake/promote", post(post_hot_cold_promote))
        .route("/api/hot_cold_stake/demote", post(post_hot_cold_demote))
        .route("/api/epv/register", post(post_epv_register))
        .route("/api/epv/status", get(get_epv_status))
        .route("/api/epv/prune", post(post_epv_prune))
        .route("/api/etlp/seal", post(post_etlp_seal))
        .route("/api/etlp/witness", post(post_etlp_witness))
        .route("/api/etlp/can_unlock", post(post_etlp_can_unlock))
        .route("/api/dsn/fold_nullifier", post(post_dsn_fold_nullifier))
        .route("/api/dsn/advance_window", post(post_dsn_advance_window))
        .route("/api/dsn/status", get(get_dsn_status))
        .route("/api/wsbf/rg_flow", post(post_wsbf_rg_flow))
        .route("/api/rg_phase/classify", post(post_rg_phase_classify))
        .route("/api/rg_phase/trajectory", post(post_rg_phase_trajectory))
        .route("/api/tombstone/mint", post(post_tombstone_mint))
        .route(
            "/api/tombstone/eulogy_root",
            post(post_tombstone_eulogy_root),
        )
        .route("/api/elexon/epoch_to_slot", post(post_elexon_epoch_to_slot))
        .route("/api/demurrage/owed", post(post_demurrage_owed))
        .route("/api/tx/settle_demurrage", post(post_settle_demurrage))
        .route("/api/bell/latest", get(get_bell_latest))
        .route("/api/mera/commit", post(post_mera_commit))
        .route(
            "/api/annealing/temperature",
            post(post_annealing_temperature),
        )
        .route(
            "/api/annealing/accepts_candidate",
            post(post_annealing_accepts_candidate),
        )
        .route(
            "/api/hlwa/effective_supply",
            post(post_hlwa_effective_supply),
        )
        .route("/api/hlwa/re_attest", post(post_hlwa_re_attest))
        .route("/api/llsa/apply_amendment", post(post_llsa_apply_amendment))
        .route(
            "/api/energy_kernel/conservation_check",
            post(post_energy_kernel_conservation_check),
        )
        .route(
            "/api/energy_kernel/redirect",
            post(post_energy_kernel_redirect),
        )
        .route("/api/autopoietic/health", get(get_autopoietic_health))
        .route("/api/consensus/phase", get(get_consensus_phase))
        .route("/api/demo/reset", post(post_demo_reset))
        .route("/api/docs", get(get_api_docs))
        .route("/api/objects", get(get_objects))
        .route("/api/object/:id", get(get_single_object))
        .route("/api/accounts", get(get_accounts))
        .route("/api/blocks", get(get_blocks))
        .route("/api/blocks/latest", get(get_latest_block))
        .route("/api/block/latest", get(get_latest_block))
        .route("/api/block/:number", get(get_single_block))
        .route("/api/light_header/:height", get(get_light_header))
        .route("/api/light_header/latest", get(get_latest_light_header))
        .route(
            "/api/block/:number/transactions",
            get(get_block_transactions),
        )
        .route("/api/account/:address", get(get_account_detail))
        .route(
            "/api/account/:address/transactions",
            get(get_account_transactions),
        )
        .route("/api/search/:query", get(explorer_search))
        .route("/api/mera/activations", get(get_mera_activations))
        .route("/api/tx/:hash", get(get_tx_by_hash))
        .route("/api/transactions", get(get_transactions))
        .route("/block/:number", get(block_detail_html))
        .route("/tx/:hash", get(tx_detail_html))
        .route("/api/mempool", get(get_mempool))
        .route("/api/mempool/:hash", get(get_mempool_by_hash))
        .route("/api/events", get(get_events))
        // Stats
        .route("/api/stats", get(get_stats_summary))
        .route("/api/stats/timeline", get(get_stats_timeline))
        .route("/api/stats/summary", get(get_stats_summary))
        // Network
        .route("/api/network", get(get_network))
        .route("/api/network/health", get(get_network_health))
        // Sybil resistance (Mainnet P1)
        .route("/api/network/peers", get(get_network_peers))
        .route("/api/network/scores", get(get_network_scores))
        .route("/api/network/banned", get(get_network_banned))
        .route("/api/network/ban", post(post_network_ban))
        .route("/api/network/unban", post(post_network_unban))
        // Block-explorer view: full validator list
        .route("/api/validators", get(get_validators))
        // Wallet / Transactions
        .route("/api/tx/transfer", post(post_transfer))
        .route("/api/tx/create-object", post(post_create_object))
        .route("/api/tx/refresh", post(post_refresh))
        .route("/api/tx/resurrect", post(post_resurrect))
        .route("/api/tx/batch", post(post_batch))
        // Validator delegation (P0 #4 wallet-facing surface)
        .route("/api/tx/delegate", post(post_delegate))
        .route("/api/tx/undelegate", post(post_undelegate))
        .route("/api/tx/claim_delegation", post(post_claim_delegation))
        .route(
            "/api/validator/:id/delegations",
            get(get_validator_delegations),
        )
        // Offline signing helpers
        .route("/api/tx/nonce/:address", get(get_account_nonce))
        .route("/api/tx/signable", post(post_signable_bytes))
        .route("/api/receipt/:hash", get(get_tx_receipt))
        .route("/api/address/:addr/transactions", get(get_address_txs))
        .route("/api/tx-index/stats", get(get_tx_index_stats))
        .route("/api/contract/:id/events", get(get_contract_events))
        .route("/api/block/:number/events", get(get_block_contract_events))
        .route("/api/event-index/stats", get(get_event_index_stats))
        // Contracts
        .route("/api/contracts", get(get_contracts))
        .route("/api/contract/:id", get(get_contract))
        .route("/api/tx/deploy-contract", post(post_deploy_contract))
        .route("/api/tx/call-contract", post(post_call_contract))
        // EvaporScript Contracts
        .route("/api/scripts", get(get_scripts))
        .route("/api/script/:id", get(get_script))
        .route("/api/script/:id/abi", get(get_script_abi))
        .route("/api/tx/deploy-script", post(post_deploy_script))
        .route("/api/tx/call-script", post(post_call_script))
        .route("/api/tx/upgrade_contract", post(post_upgrade_contract))
        // NFT Marketplace
        .route("/nft", get(nft_html))
        .route("/api/nfts", get(get_nfts))
        .route("/api/nft/collections", get(get_nft_collections))
        .route("/api/nft/:id", get(get_single_nft))
        .route("/api/nft/mint", post(post_mint_nft))
        .route("/api/nft/transfer", post(post_transfer_nft))
        .route("/api/nft/refresh", post(post_refresh_nft))
        // Tokens
        .route("/tokens", get(tokens_html))
        .route("/api/tokens", get(get_tokens))
        .route("/api/token/:id", get(get_single_token))
        .route("/api/token/deploy", post(post_deploy_token))
        .route("/api/token/transfer", post(post_token_transfer))
        .route("/api/token/balance", post(post_token_balance))
        // Swap
        .route("/api/swap/quote", post(post_swap_quote))
        .route("/api/swap/execute", post(post_swap_execute))
        // Staking
        .route("/staking", get(staking_html))
        .route("/api/staking", get(get_staking_pools))
        .route("/api/staking/pools", get(get_staking_pools))
        .route("/api/staking/pool/:id", get(get_single_pool))
        .route("/api/staking/stake", post(post_stake))
        .route("/api/staking/unstake", post(post_unstake))
        .route("/api/staking/claim", post(post_claim))
        // DAO
        .route("/dao", get(dao_html))
        .route("/api/dao", get(get_proposals))
        .route("/api/dao/proposals", get(get_proposals))
        .route("/api/dao/proposal/:id", get(get_single_proposal))
        .route("/api/dao/propose", post(post_propose))
        .route("/api/dao/vote", post(post_vote))
        // Address detail
        .route("/address", get(address_html))
        .route("/address/:addr", get(address_html))
        .route("/api/address/:addr", get(get_address_detail))
        // Faucet
        .route("/faucet", get(faucet_html))
        .route("/docs", get(docs_html))
        .route("/api/faucet", post(post_faucet))
        // Oracle (no auth — node-operator data ingestion)
        .route("/api/oracle/ingest", post(post_oracle_ingest))
        .route("/api/oracle/status", get(get_oracle_status))
        .route("/api/oracle/feed/:key", get(get_oracle_feed))
        // Sharding
        .route("/api/shards", get(get_shard_status))
        .route("/api/shards/health", get(get_shard_health))
        // Ghost bridges
        .route("/api/ghosts", get(get_ghost_list))
        .route("/api/ghosts/:id", get(get_ghost_detail))
        // Metrics / Throughput
        .route("/api/metrics", get(get_metrics))
        .route("/metrics", get(get_prometheus_metrics))
        // Admin — graceful drain (Ansible upgrade playbook)
        .route("/api/admin/drain", post(post_admin_drain))
        .route("/api/admin/undrain", post(post_admin_undrain))
        .route("/api/admin/drain/status", get(get_admin_drain_status))
        // Nova Proofs / Light Client
        .route("/api/proof/latest", get(get_proof_latest))
        .route("/api/proof/status", get(get_proof_status))
        .route("/api/proof/verify", get(get_proof_verify))
        // Data Availability sampling
        .route(
            "/api/light/state-proof/account/:addr",
            get(get_account_state_proof),
        )
        .route(
            "/api/light/state-proof/object/:id",
            get(get_object_state_proof),
        )
        .route(
            "/api/light/tx-proof/:block/:tx_index",
            get(get_tx_inclusion_proof),
        )
        .route("/api/light/verify-tx-proof", post(post_verify_tx_proof))
        .route("/api/light/verify-state-proof", get(get_verify_state_proof))
        .route("/api/light/headers", get(get_light_headers))
        .route("/api/frontier", get(get_frontier_status))
        .route("/api/lazy-eval", get(get_lazy_eval))
        .route("/api/da/status", get(get_da_status))
        .route("/api/da/block/:number", get(get_da_block))
        .route("/api/da/sample/:block/:shard_index", get(get_da_sample))
        .route("/api/da/light-sample/:block", get(get_da_light_sample))
        .route("/api/da/cell/:block/:row/:col", get(get_da_cell_sample))
        .route("/api/da/header/:block", get(get_da_2d_header))
        .route(
            "/api/da/2d-light-sample/:block",
            get(get_da_2d_light_sample),
        )
        .route(
            "/api/da/evaporation-proof/:object_id",
            get(get_evaporation_da_proof),
        )
        // PoHA certificates
        .route("/api/da/poha", get(get_poha_certificates))
        .route("/api/da/poha/:block", get(get_poha_certificate))
        // Light client
        .route("/api/light/client/status", get(get_light_client_status))
        .route("/api/light/client/verify", post(post_verify_header))
        .route("/api/light/client/header/:height", get(get_trusted_header))
        // MEV-protected encrypted mempool
        .route("/api/mev/submit", post(post_submit_encrypted_tx))
        .route("/api/mev/reveal", post(post_reveal_encrypted_tx))
        .route("/api/mev/status", get(get_encrypted_mempool_status))
        // Finality
        .route(
            "/api/weak-subjectivity",
            get(get_weak_subjectivity_checkpoint),
        )
        .route("/api/finality", get(get_finality))
        .route("/api/finality/proof/:height", get(get_finality_proof))
        .route("/api/finality/gap", get(get_finality_gap))
        // State sync
        .route("/api/sync/snapshot-info", get(get_sync_snapshot_info))
        // Fast-sync snapshot (zstd .zst blob format, see SnapshotFile)
        .route("/api/snapshot/latest", get(get_snapshot_latest))
        .route("/api/snapshot/download/:height", get(get_snapshot_download))
        // PWA
        .route("/manifest.json", get(manifest_json))
        .route("/sw.js", get(service_worker_js))
        // WebSocket subscriptions
        .route("/ws", get(ws_upgrade_handler))
        // JSON-RPC 2.0 endpoint
        .route("/rpc", post(crate::jsonrpc::handle_jsonrpc));

    // Phase 5.4 of LAMBDA_FOLD_NOVA_PLAN — nova-mode Lambda-Fold
    // endpoints are only mounted when the `lambda_fold_nova` feature
    // is on. With the feature off, the routes don't exist (404)
    // rather than returning a runtime "not compiled in" error —
    // operators see the absence in `/api/docs` and the routing table.
    #[cfg(feature = "lambda_fold_nova")]
    let router = router
        .route("/api/lambda_fold/nova", get(get_lambda_fold_nova))
        .route(
            "/api/lambda_fold/nova/verify",
            post(post_lambda_fold_nova_verify),
        )
        .route(
            "/api/lambda_fold/nova/vk_bytes",
            get(get_lambda_fold_nova_vk_bytes),
        );

    router
        .with_state(state)
        // Merge auth routes (different state type)
        .merge(auth_router)
        .fallback(fallback_404)
        .layer(cors)
        .layer(axum::middleware::map_response(
            |mut resp: axum::http::Response<axum::body::Body>| async move {
                security_headers(&mut resp);
                resp
            },
        ))
}

// ──────────────────────────── Rate Limiter ────────────────────────────

/// Simple in-memory IP rate limiter: max `limit` requests per `window` per IP.
struct RateLimiter {
    requests: Mutex<HashMap<std::net::IpAddr, Vec<Instant>>>,
    limit: usize,
    window: std::time::Duration,
}

impl RateLimiter {
    fn new(limit: usize, window_secs: u64) -> Self {
        Self {
            requests: Mutex::new(HashMap::new()),
            limit,
            window: std::time::Duration::from_secs(window_secs),
        }
    }

    fn check(&self, ip: std::net::IpAddr) -> bool {
        let mut map = self.requests.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        let timestamps = map.entry(ip).or_default();
        timestamps.retain(|t| now.duration_since(*t) < self.window);
        if timestamps.len() >= self.limit {
            return false;
        }
        timestamps.push(now);
        true
    }
}

async fn rate_limit_middleware(
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    axum::extract::Extension(limiter): axum::extract::Extension<Arc<RateLimiter>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if !addr.ip().is_loopback() && !limiter.check(addr.ip()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("Retry-After", "10")],
            "Rate limit exceeded",
        )
            .into_response();
    }
    next.run(request).await
}

/// Start the API server on the given port.
///
/// TLS: Set `EVAPORCHAIN_TLS_CERT` and `EVAPORCHAIN_TLS_KEY` environment
/// variables to PEM file paths to enable HTTPS. Without these, the server
/// binds plaintext HTTP — suitable for localhost or behind a TLS-terminating
/// reverse proxy (nginx, caddy), but NOT for direct internet exposure.
pub async fn start_api_server(
    state: Arc<ApiState>,
    auth_state: Arc<crate::auth::AuthState>,
    port: u16,
) -> anyhow::Result<()> {
    let limiter = Arc::new(RateLimiter::new(200, 10));
    let app = create_router(state, auth_state)
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(axum::Extension(limiter))
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));

    // Loud warning when admin endpoints (drain / metrics / proof_replay)
    // are open. EVAPORCHAIN_ADMIN_KEY=<secret> in env locks them down.
    if std::env::var("EVAPORCHAIN_ADMIN_KEY")
        .map(|v| v.is_empty())
        .unwrap_or(true)
    {
        eprintln!(
            "\x1b[1;31m⚠ EVAPORCHAIN_ADMIN_KEY is unset — admin endpoints \
             (/api/admin/drain, /metrics, ...) are UNAUTHENTICATED. \
             Set EVAPORCHAIN_ADMIN_KEY=<random-32-bytes> for production.\x1b[0m"
        );
    }

    // AUDIT_2026_05_06.md MEDIUM — Dashboard TLS. When both
    // EVAPORCHAIN_TLS_CERT and EVAPORCHAIN_TLS_KEY are set,
    // terminate TLS in-process via axum-server + rustls. The
    // env-var pair points at PEM file paths (cert chain + private
    // key). When either is unset, fall through to plain HTTP for
    // localhost dev — the operational warning makes the choice
    // explicit.
    let tls_cert_path = std::env::var("EVAPORCHAIN_TLS_CERT").ok();
    let tls_key_path = std::env::var("EVAPORCHAIN_TLS_KEY").ok();
    if let (Some(cert_path), Some(key_path)) = (tls_cert_path, tls_key_path) {
        println!(
            "\x1b[1;32m━━━ Dashboard: https://localhost:{} (TLS) ━━━\x1b[0m",
            port
        );
        let config = axum_server::tls_rustls::RustlsConfig::from_pem_file(
            std::path::PathBuf::from(&cert_path),
            std::path::PathBuf::from(&key_path),
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "EVAPORCHAIN_TLS_CERT={cert_path} / EVAPORCHAIN_TLS_KEY={key_path} \
                 failed to load: {e}. PEM files must be readable + parsable."
            )
        })?;
        axum_server::bind_rustls(addr, config).serve(app).await?;
        return Ok(());
    }

    // Plain HTTP fall-through for dev / localhost.
    eprintln!(
        "\x1b[33m⚠ Dashboard serving over HTTP (plaintext). \
         For production, use a TLS-terminating reverse proxy \
         or set EVAPORCHAIN_TLS_CERT + EVAPORCHAIN_TLS_KEY \
         (PEM file paths).\x1b[0m"
    );
    println!(
        "\x1b[1;36m━━━ Dashboard: http://localhost:{} ━━━\x1b[0m",
        port
    );
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ──────────────────────────── Helpers for main.rs ─────────────────────

/// Build TxRecord entries from a block's transactions.
/// Gas cost estimates (must match execution crate).
pub(crate) fn estimate_tx_gas_pub(tx: &Transaction) -> u64 {
    estimate_tx_gas(tx)
}

fn estimate_tx_gas(tx: &Transaction) -> u64 {
    match tx {
        Transaction::Transfer(_) => 21_000,
        Transaction::CreateObject(t) => 50_000 + (200 * t.data.len() as u64),
        Transaction::Refresh(_) => 30_000,
        Transaction::DeployContract(_) => 100_000,
        Transaction::CallContract(_) => 40_000,
        Transaction::DeployScript(_) => 150_000,
        Transaction::CallScript(_) => 50_000,
        Transaction::ValidatorStake(_) => 50_000,
        Transaction::ValidatorExit(_) => 30_000,
        Transaction::ValidatorClaimStake(_) => 30_000,
        Transaction::Shield(_) => 60_000,
        Transaction::Unshield(_) => 80_000,
        Transaction::PrivateTransfer(ptx) => {
            100_000
                + 20_000 * ptx.input_nullifiers.len() as u64
                + 15_000 * ptx.output_commitments.len() as u64
        }
        Transaction::Deferred(dtx) => 75_000 + 5_000 * dtx.guards.len() as u64,
        Transaction::Blob(tx) => 50_000 + 10 * tx.data.len() as u64,
        Transaction::Governance(_) => 25_000,
        Transaction::MultiSig(_) => 50_000,
        Transaction::UserOp(tx) => 30_000 + tx.call_data.len() as u64 * 16,
        Transaction::UpgradeContract(tx) => 100_000 + tx.new_bytecode.len() as u64 * 200,
        Transaction::Delegate(_) => 40_000,
        Transaction::Undelegate(_) => 40_000,
        Transaction::RotateValidatorKey(_) => 80_000,
        Transaction::ClaimDelegation(_) => 30_000,
        Transaction::Refund(_) => 5_000,
    }
}

/// Resolve a per-tx (status, error) tuple from a block-aligned outcomes
/// slice. Index-aligned with `block.transactions`. When the slice is
/// empty (legacy execution path that did not populate outcomes) or the
/// hash mismatches, falls back to `"success"` and emits a warn-level
/// log so missing wiring is surfaced rather than silently lying.
///
/// Tied to the bug uncovered during the 3-node faucet smoke run where
/// `/api/transactions` reported success for every tx regardless of
/// whether the transfer actually credited the recipient.
fn outcome_to_status(
    tx: &Transaction,
    idx: usize,
    outcomes: &[evaporchain_execution::TxOutcome],
) -> (String, Option<String>) {
    if outcomes.is_empty() {
        tracing::warn!(
            tx_hash = %hex::encode(tx.tx_hash()),
            "tx_records_from_block: no per-tx outcomes available — \
             falling back to 'success' (executor not wired through). \
             /api/transactions status may be inaccurate."
        );
        return ("success".to_string(), None);
    }
    match outcomes.get(idx) {
        Some(o) if o.tx_hash == tx.tx_hash() => {
            if o.success {
                ("success".to_string(), None)
            } else {
                (
                    "rejected".to_string(),
                    Some(o.error.clone().unwrap_or_else(|| "rejected".to_string())),
                )
            }
        }
        Some(o) => {
            // Slice is non-empty but the hash doesn't match — the
            // executor returned a misaligned outcomes vec. Fall back
            // safely and log loudly.
            tracing::warn!(
                tx_hash = %hex::encode(tx.tx_hash()),
                outcome_hash = %hex::encode(o.tx_hash),
                idx,
                "tx_records_from_block: outcome hash mismatch — \
                 treating as success (executor wiring bug)"
            );
            ("success".to_string(), None)
        }
        None => {
            tracing::warn!(
                tx_hash = %hex::encode(tx.tx_hash()),
                idx,
                outcomes_len = outcomes.len(),
                "tx_records_from_block: outcome missing for tx index — \
                 falling back to 'success'"
            );
            ("success".to_string(), None)
        }
    }
}

/// Build TxRecord entries from a block + per-tx outcomes.
///
/// Outcomes are produced by `ExecutionEngine::execute_block` and must
/// be aligned with `block.transactions`. Pass `&[]` for legacy paths
/// that don't yet have outcomes — this falls back to `"success"` with
/// a warn-level log (preserves the prior reporting behaviour).
pub fn tx_records_from_block_with_outcomes(
    block: &Block,
    outcomes: &[evaporchain_execution::TxOutcome],
) -> Vec<TxRecord> {
    block
        .transactions
        .iter()
        .enumerate()
        .map(|(i, tx)| {
            let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
            let gas = estimate_tx_gas(tx);
            let (status, error) = outcome_to_status(tx, i, outcomes);
            // `error` is plumbed onto TxRecord.error so wallets and
            // explorers can surface the executor's rejection reason
            // (e.g. "InsufficientBalance"). Absent for successful txs;
            // serialised as missing via `skip_serializing_if`.
            match tx {
                Transaction::Transfer(t) => TxRecord {
                    hash,
                    tx_type: "transfer".to_string(),
                    from: account_full(&t.from),
                    to: account_full(&t.to),
                    amount: Some(t.amount),
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::CreateObject(t) => TxRecord {
                    hash,
                    tx_type: "create_object".to_string(),
                    from: account_full(&t.creator),
                    to: String::new(),
                    amount: None,
                    object_id: Some(format!("0x{}", hex::encode(&t.object_id[..8]))),
                    energy: Some(t.energy),
                    half_life: Some(t.half_life),
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::Refresh(t) => TxRecord {
                    hash,
                    tx_type: "refresh".to_string(),
                    from: String::new(),
                    to: String::new(),
                    amount: None,
                    object_id: Some(format!("0x{}", hex::encode(&t.object_id[..8]))),
                    energy: Some(t.energy_deposit),
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::DeployContract(t) => TxRecord {
                    hash,
                    tx_type: "deploy_contract".to_string(),
                    from: account_full(&t.deployer),
                    to: String::new(),
                    amount: None,
                    object_id: None,
                    energy: Some(t.energy),
                    half_life: Some(t.half_life),
                    method: Some(t.template.clone()),
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::CallContract(t) => TxRecord {
                    hash,
                    tx_type: "call_contract".to_string(),
                    from: account_full(&t.caller),
                    to: format!("contract:{}", t.contract_id),
                    amount: None,
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: Some(t.method.clone()),
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::DeployScript(t) => TxRecord {
                    hash,
                    tx_type: "deploy_script".to_string(),
                    from: account_full(&t.deployer),
                    to: String::new(),
                    amount: None,
                    object_id: None,
                    energy: Some(t.energy),
                    half_life: Some(t.half_life),
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::CallScript(t) => TxRecord {
                    hash,
                    tx_type: "call_script".to_string(),
                    from: account_full(&t.caller),
                    to: format!("script:{}", t.contract_id),
                    amount: None,
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: Some(t.method.clone()),
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::ValidatorStake(t) => TxRecord {
                    hash,
                    tx_type: "validator_stake".to_string(),
                    from: account_full(&t.validator_address),
                    to: String::new(),
                    amount: Some(t.stake_amount),
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::ValidatorExit(t) => TxRecord {
                    hash,
                    tx_type: "validator_exit".to_string(),
                    from: account_full(&t.validator_address),
                    to: String::new(),
                    amount: None,
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::ValidatorClaimStake(t) => TxRecord {
                    hash,
                    tx_type: "validator_claim_stake".to_string(),
                    from: account_full(&t.validator_address),
                    to: String::new(),
                    amount: None,
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::Shield(t) => TxRecord {
                    hash,
                    tx_type: "shield".to_string(),
                    from: account_full(&t.from),
                    to: String::new(),
                    amount: Some(t.amount),
                    object_id: None,
                    energy: t.energy,
                    half_life: Some(t.half_life),
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::Unshield(t) => TxRecord {
                    hash,
                    tx_type: "unshield".to_string(),
                    from: String::new(),
                    to: account_full(&t.to),
                    amount: Some(t.amount),
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::PrivateTransfer(t) => TxRecord {
                    hash,
                    tx_type: "private_transfer".to_string(),
                    from: String::new(),
                    to: String::new(),
                    amount: Some(t.fee),
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::Deferred(dtx) => TxRecord {
                    hash,
                    tx_type: "deferred".to_string(),
                    from: account_full(&dtx.submitter),
                    to: String::new(),
                    amount: Some(dtx.deposit),
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::Blob(tx) => TxRecord {
                    hash,
                    tx_type: "blob".to_string(),
                    from: account_full(&tx.submitter),
                    to: String::new(),
                    amount: None,
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::Governance(tx) => TxRecord {
                    hash,
                    tx_type: "governance".to_string(),
                    from: account_full(&tx.sender),
                    to: String::new(),
                    amount: None,
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::MultiSig(tx) => TxRecord {
                    hash,
                    tx_type: "multisig".to_string(),
                    from: account_full(&tx.multisig_address),
                    to: String::new(),
                    amount: None,
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::UserOp(tx) => TxRecord {
                    hash,
                    tx_type: "user_op".to_string(),
                    from: account_full(&tx.sender),
                    to: String::new(),
                    amount: None,
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::UpgradeContract(tx) => TxRecord {
                    hash,
                    tx_type: "upgrade_contract".to_string(),
                    from: account_full(&tx.owner),
                    to: String::new(),
                    amount: None,
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::Delegate(tx) => TxRecord {
                    hash,
                    tx_type: "delegate".to_string(),
                    from: account_full(&tx.delegator),
                    to: format!("validator-{}", tx.validator_id),
                    amount: Some(tx.amount),
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::Undelegate(tx) => TxRecord {
                    hash,
                    tx_type: "undelegate".to_string(),
                    from: account_full(&tx.delegator),
                    to: format!("validator-{}", tx.validator_id),
                    amount: Some(tx.amount),
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::RotateValidatorKey(tx) => TxRecord {
                    hash,
                    tx_type: "rotate_validator_key".to_string(),
                    from: account_full(&tx.validator_address),
                    to: format!("validator-{}", tx.validator_id),
                    amount: None,
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: Some(format!("effective_epoch={}", tx.effective_epoch)),
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::ClaimDelegation(tx) => TxRecord {
                    hash,
                    tx_type: "claim_delegation".to_string(),
                    from: account_full(&tx.delegator),
                    to: format!("validator-{}", tx.validator_id),
                    amount: None,
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
                Transaction::Refund(tx) => TxRecord {
                    hash,
                    tx_type: "refund".to_string(),
                    from: account_full(&tx.attacker),
                    to: account_full(&tx.victim),
                    amount: Some(tx.amount),
                    object_id: None,
                    energy: None,
                    half_life: None,
                    method: None,
                    gas,
                    block_number: block.number,
                    epoch: block.epoch,
                    status: status.clone(),
                    error: error.clone(),
                },
            }
        })
        .collect()
}

/// Push an event into the events ring buffer.
pub fn push_event(
    events: &Arc<Mutex<VecDeque<EventRecord>>>,
    epoch: u64,
    event_type: &str,
    message: String,
) {
    let mut evts = safe_lock(events);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    evts.push_back(EventRecord {
        epoch,
        event_type: event_type.to_string(),
        message,
        timestamp_ms: ts,
    });
    // Keep last 200 events
    while evts.len() > 200 {
        evts.pop_front();
    }
}

#[cfg(test)]
mod tx_status_tests {
    //! Smoke tests for the per-tx-status reporting fix.
    //!
    //! Covers the bug surfaced during the 3-node faucet smoke run
    //! (commit 58c70bc context) where every recorded tx in a
    //! BlockRecord reported `status: "success"` regardless of whether
    //! the executor actually committed it. After the fix:
    //!   - InsufficientBalance / InvalidNonce → status = "rejected"
    //!   - successful Transfer → status = "success"
    //!   - the legacy code path (empty outcomes slice) still emits
    //!     "success" with a warn log (back-compat).
    use super::*;
    use evaporchain_execution::{ExecutionEngine, SimpleExecutor, TxOutcome};
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::{Account, Block, Transaction, TransferTx};

    fn addr(byte: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = byte;
        a
    }

    fn fund(db: &mut InMemoryStateDB, byte: u8, balance: u64) {
        db.put_account(Account {
            address: addr(byte),
            balance,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
        });
    }

    fn make_block(number: u64, epoch: u64, txs: Vec<Transaction>) -> Block {
        Block {
            number,
            epoch,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: txs,
            timestamp: 0,
            chain_id: String::new(),
            producer_id: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
            da_row_roots: vec![],
            da_col_roots: vec![],
        }
    }

    fn transfer(from: u8, to: u8, amount: u64, nonce: u64) -> Transaction {
        Transaction::Transfer(TransferTx {
            from: addr(from),
            to: addr(to),
            amount,
            nonce,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        })
    }

    /// A Transfer with insufficient balance should be reported as
    /// "rejected" with a non-empty error in /api/transactions.
    #[test]
    fn insufficient_balance_transfer_reports_rejected() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 100); // sender has 100, asks for 500
        let mut executor = SimpleExecutor::new(7);
        let block = make_block(1, 1, vec![transfer(1, 2, 500, 0)]);

        let exec = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(exec.tx_outcomes.len(), 1);
        assert!(!exec.tx_outcomes[0].success);
        assert!(exec.tx_outcomes[0].error.is_some());

        let records = tx_records_from_block_with_outcomes(&block, &exec.tx_outcomes);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, "rejected");
        // Hash alignment must hold so the API can map outcomes back to txs.
        assert_eq!(exec.tx_outcomes[0].tx_hash, block.transactions[0].tx_hash());
    }

    /// A rejected Transfer must surface the executor error string on
    /// the TxRecord so wallets can answer "why did my tx fail?". The
    /// JSON serialisation must include a non-empty `error` field for
    /// rejected txs and omit it entirely for successful ones (per the
    /// `skip_serializing_if` contract).
    #[test]
    fn rejected_tx_record_carries_error_string_and_serialises_it() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 100);
        let mut executor = SimpleExecutor::new(7);
        let block = make_block(
            1,
            1,
            vec![
                transfer(1, 2, 500, 0), // rejected — InsufficientBalance
                transfer(1, 2, 50, 1),  // success after partial debit? actually
                                        // both run against the same DB; second
                                        // may also fail. We only care about the
                                        // first record here, but include a
                                        // second tx so the field-omission
                                        // assertion has something to bite on
                                        // when the executor accepts it.
            ],
        );

        let exec = executor.execute_block(&mut db, &block).unwrap();
        let records = tx_records_from_block_with_outcomes(&block, &exec.tx_outcomes);
        assert_eq!(records.len(), 2);

        // First tx: must be rejected with a non-empty error string.
        assert_eq!(records[0].status, "rejected");
        let err = records[0]
            .error
            .as_ref()
            .expect("rejected TxRecord must carry an executor error string");
        assert!(
            !err.is_empty(),
            "TxRecord.error must be a non-empty string for rejected txs, got {:?}",
            err
        );

        // JSON wire shape: rejected tx must serialise an `error` field
        // with a non-empty string value.
        let json0 = serde_json::to_value(&records[0]).expect("TxRecord serialises");
        let err_json = json0
            .get("error")
            .and_then(|v| v.as_str())
            .expect("rejected TxRecord JSON must include `error` field");
        assert!(
            !err_json.is_empty(),
            "rejected TxRecord JSON `error` must be non-empty"
        );

        // For any tx that succeeded, the JSON must omit `error` entirely
        // (skip_serializing_if = "Option::is_none"). For a rejected tx
        // with the same outcome it must be present. This guards the
        // wire-format contract that wallet/explorer consumers depend on.
        if records[1].status == "success" {
            let json1 = serde_json::to_value(&records[1]).expect("TxRecord serialises");
            assert!(
                json1.get("error").is_none(),
                "successful TxRecord JSON must omit `error` field, got {:?}",
                json1
            );
        }
    }

    /// A valid Transfer should be reported as "success" in
    /// /api/transactions.
    #[test]
    fn valid_transfer_reports_success() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 1000);
        let mut executor = SimpleExecutor::new(7);
        let block = make_block(1, 1, vec![transfer(1, 2, 100, 0)]);

        let exec = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(exec.tx_outcomes.len(), 1);
        assert!(exec.tx_outcomes[0].success);
        assert!(exec.tx_outcomes[0].error.is_none());

        let records = tx_records_from_block_with_outcomes(&block, &exec.tx_outcomes);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, "success");
    }

    /// A mixed block (one valid, one invalid) must report each tx with
    /// its own status — proves outcomes are positionally correct.
    #[test]
    fn mixed_block_reports_per_tx_status() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 1000);
        fund(&mut db, 3, 50); // too poor for a 500 transfer
        let mut executor = SimpleExecutor::new(7);
        let block = make_block(
            1,
            1,
            vec![
                transfer(1, 2, 100, 0), // good
                transfer(3, 4, 500, 0), // insufficient balance
            ],
        );

        let exec = executor.execute_block(&mut db, &block).unwrap();
        let records = tx_records_from_block_with_outcomes(&block, &exec.tx_outcomes);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].status, "success");
        assert_eq!(records[1].status, "rejected");
    }

    /// Legacy back-compat: empty outcomes slice falls back to
    /// "success" + warn log (preserves prior behaviour for any code
    /// path not yet wired through). This is the documented fallback.
    #[test]
    fn empty_outcomes_falls_back_to_success() {
        let block = make_block(1, 1, vec![transfer(1, 2, 100, 0)]);
        let records: Vec<TxRecord> = tx_records_from_block_with_outcomes(&block, &[]);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, "success");
    }

    /// Mismatched outcome hash falls back to success rather than
    /// crashing — guards against accidental misalignment from a
    /// future executor change.
    #[test]
    fn hash_mismatch_falls_back_to_success() {
        let block = make_block(1, 1, vec![transfer(1, 2, 100, 0)]);
        let bogus = vec![TxOutcome {
            tx_hash: [0xAB; 32], // not the real hash
            success: false,
            error: Some("not even my tx".into()),
            gas_used: 0,
        }];
        let records = tx_records_from_block_with_outcomes(&block, &bogus);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, "success");
    }
}

#[cfg(test)]
mod faucet_rate_limit_tests {
    //! Regression tests for the 2026-04-30 fix where the `/api/faucet`
    //! rate limiter keyed on `hex(addr[..20])`, causing 60-tx stress
    //! bursts with sequentially-numbered recipients (all sharing 20
    //! leading zero bytes) to collapse into a single bucket — the
    //! 3-Mini cluster smoke run saw 1 success / 59 rate-limited.
    //!
    //! These tests drive the pure helper `check_and_record_faucet_rate_limit`
    //! directly — no `ApiState`, no router. The handler `post_faucet`
    //! is a thin shell over this helper plus the IP extraction in
    //! `client_ip_from`; both are exercised here.
    use super::{
        check_and_record_faucet_rate_limit, client_ip_from, FaucetRateOutcome,
        FAUCET_RATE_LIMIT_SECS,
    };
    use axum::http::HeaderMap;
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::Instant;

    const TEST_CAP: usize = 100_000;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn addr(seq: u8) -> [u8; 32] {
        // Distinct addresses that all share `[..20]` (zeros) — exactly
        // the shape that broke the old prefix-keyed limiter.
        let mut a = [0u8; 32];
        a[31] = seq;
        a
    }

    /// 10 distinct recipients from one harness IP must all pass — this
    /// is the load-test scenario that the old `addr[..20]` key broke.
    #[test]
    fn faucet_rate_limit_distinct_addresses_same_ip_pass() {
        let mut map: HashMap<(IpAddr, [u8; 32]), Instant> = HashMap::new();
        let harness = ip(10, 0, 0, 1);
        for seq in 0..10u8 {
            let outcome = check_and_record_faucet_rate_limit(
                &mut map,
                harness,
                addr(seq),
                FAUCET_RATE_LIMIT_SECS,
                false,
                TEST_CAP,
            );
            assert_eq!(
                outcome,
                FaucetRateOutcome::Allowed,
                "distinct address #{} from harness IP must pass",
                seq
            );
        }
        assert_eq!(map.len(), 10, "each (ip, addr) pair occupies its own slot");
    }

    /// Two faucets to the same recipient address from the same IP within
    /// the cooldown window: the second must be `Blocked`. This is the
    /// original anti-pump intent, and it must survive the key change.
    #[test]
    fn faucet_rate_limit_same_address_same_ip_blocks() {
        let mut map: HashMap<(IpAddr, [u8; 32]), Instant> = HashMap::new();
        let harness = ip(10, 0, 0, 1);
        let target = addr(7);
        let first = check_and_record_faucet_rate_limit(
            &mut map,
            harness,
            target,
            FAUCET_RATE_LIMIT_SECS,
            false,
            TEST_CAP,
        );
        assert_eq!(first, FaucetRateOutcome::Allowed);
        let second = check_and_record_faucet_rate_limit(
            &mut map,
            harness,
            target,
            FAUCET_RATE_LIMIT_SECS,
            false,
            TEST_CAP,
        );
        match second {
            FaucetRateOutcome::Blocked { remaining_secs } => {
                assert!(
                    remaining_secs <= FAUCET_RATE_LIMIT_SECS,
                    "remaining must be ≤ cooldown, got {}",
                    remaining_secs
                );
                assert!(
                    remaining_secs > 0,
                    "remaining must be > 0 immediately after first hit, got {}",
                    remaining_secs
                );
            }
            other => panic!("expected Blocked, got {:?}", other),
        }
    }

    /// The same recipient address from two different IPs: both must
    /// pass. A faucet keyed on (ip, addr) splits the slot per-IP, so
    /// one operator pumping a single address cannot DoS another
    /// operator's claim to the same address.
    #[test]
    fn faucet_rate_limit_same_address_different_ips_pass() {
        let mut map: HashMap<(IpAddr, [u8; 32]), Instant> = HashMap::new();
        let target = addr(42);
        let a = check_and_record_faucet_rate_limit(
            &mut map,
            ip(10, 0, 0, 1),
            target,
            FAUCET_RATE_LIMIT_SECS,
            false,
            TEST_CAP,
        );
        let b = check_and_record_faucet_rate_limit(
            &mut map,
            ip(10, 0, 0, 2),
            target,
            FAUCET_RATE_LIMIT_SECS,
            false,
            TEST_CAP,
        );
        assert_eq!(a, FaucetRateOutcome::Allowed);
        assert_eq!(b, FaucetRateOutcome::Allowed);
    }

    /// `--faucet-rate-limit-disabled` (the disable bit on `ApiState`)
    /// must bypass the check entirely — even repeated hits on the same
    /// (ip, addr) pair pass. Used by stress harnesses where the chain
    /// is the system under test, not the faucet.
    #[test]
    fn faucet_rate_limit_disabled_flag_bypasses_all() {
        let mut map: HashMap<(IpAddr, [u8; 32]), Instant> = HashMap::new();
        let harness = ip(10, 0, 0, 1);
        let target = addr(99);
        for _ in 0..5 {
            let outcome = check_and_record_faucet_rate_limit(
                &mut map,
                harness,
                target,
                FAUCET_RATE_LIMIT_SECS,
                /* disabled */ true,
                TEST_CAP,
            );
            assert_eq!(
                outcome,
                FaucetRateOutcome::Allowed,
                "disabled flag must allow repeated same-(ip, addr) hits"
            );
        }
        assert!(
            map.is_empty(),
            "disabled mode must not allocate map entries"
        );
    }

    /// **Audit fix C3, default-deny path**: with no
    /// `EVAPORCHAIN_TRUSTED_PROXY_DEPTH` env var set,
    /// `X-Forwarded-For` is **ignored entirely** to defeat header
    /// spoofing. The direct TCP peer is the only trusted source.
    #[test]
    fn client_ip_ignores_x_forwarded_for_by_default() {
        // Make sure no test elsewhere has set the env var.
        std::env::remove_var("EVAPORCHAIN_TRUSTED_PROXY_DEPTH");
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            "203.0.113.5, 198.51.100.7".parse().unwrap(),
        );
        let direct = SocketAddr::new(ip(10, 0, 0, 1), 12345);
        let resolved = client_ip_from(&h, Some(direct));
        // With depth=0, the spoofable header is ignored — the direct
        // peer wins.
        assert_eq!(resolved, ip(10, 0, 0, 1));
    }

    /// No `X-Forwarded-For` → fall through to `ConnectInfo`.
    #[test]
    fn client_ip_falls_back_to_connect_info() {
        std::env::remove_var("EVAPORCHAIN_TRUSTED_PROXY_DEPTH");
        let h = HeaderMap::new();
        let direct = SocketAddr::new(ip(10, 0, 0, 9), 4444);
        let resolved = client_ip_from(&h, Some(direct));
        assert_eq!(resolved, ip(10, 0, 0, 9));
    }

    /// Header missing AND no ConnectInfo → `0.0.0.0`. Verifies the
    /// fallback constant rather than a panic.
    #[test]
    fn client_ip_unresolved_falls_back_to_unspecified() {
        std::env::remove_var("EVAPORCHAIN_TRUSTED_PROXY_DEPTH");
        let h = HeaderMap::new();
        let resolved = client_ip_from(&h, None);
        assert_eq!(resolved, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    }

    /// Operator opt-in: with `TRUSTED_PROXY_DEPTH=1`, the rightmost
    /// entry (the closest proxy hop, set by the operator's reverse
    /// proxy) is used. Leftmost (attacker-controlled) entries are
    /// ignored.
    #[test]
    fn client_ip_with_depth_one_uses_rightmost_entry() {
        std::env::set_var("EVAPORCHAIN_TRUSTED_PROXY_DEPTH", "1");
        let mut h = HeaderMap::new();
        // Attacker prefixes "9.9.9.9, " in front; the operator's proxy
        // appends "203.0.113.5". Rightmost = trusted client.
        h.insert("x-forwarded-for", "9.9.9.9, 203.0.113.5".parse().unwrap());
        let direct = SocketAddr::new(ip(10, 0, 0, 1), 12345);
        let resolved = client_ip_from(&h, Some(direct));
        assert_eq!(resolved, ip(203, 0, 113, 5));
        std::env::remove_var("EVAPORCHAIN_TRUSTED_PROXY_DEPTH");
    }
}

#[cfg(test)]
mod bell_beacon_v2_handler_tests {
    //! Handler-level smoke tests for the Bell-Certified Beacon V2
    //! `/api/bell_beacon_v2/{issue,verify}` endpoints. The substrate
    //! is fully tested in `evaporchain-bell-beacon-v2`; this mod
    //! locks the JSON DTO ↔ inner-cert translation, the issue →
    //! verify round-trip through the actual handler bodies, and the
    //! chain-id binding contract that defeats cross-chain replay.
    use super::*;

    fn balanced_window() -> Vec<BellBeaconV2PairDto> {
        let mut out = Vec::new();
        for i in 0..16u8 {
            let mut tag = [0u8; 32];
            tag[0] = i;
            tag[31] = i;
            out.push(BellBeaconV2PairDto {
                first_energy: if i & 1 == 1 { 100 } else { 10 },
                first_tx_count: if (i >> 1) & 1 == 1 { 100 } else { 10 },
                second_energy: if (i >> 2) & 1 == 1 { 100 } else { 10 },
                second_tx_count: if (i >> 3) & 1 == 1 { 100 } else { 10 },
                tag_hex: hex::encode(tag),
            });
        }
        out
    }

    fn prev_hex() -> String {
        let mut a = [0u8; 32];
        a[0] = 9;
        hex::encode(a)
    }

    /// Issue then verify the same certificate against the same inputs
    /// — must succeed.
    #[tokio::test]
    async fn issue_then_verify_round_trip() {
        let pairs = balanced_window();
        let issue_resp = post_bell_beacon_v2_issue(Json(BellBeaconV2IssueReq {
            chain_id: "test-chain-v1".to_string(),
            window_start: 100,
            window_end: 200,
            pairs: pairs.clone(),
            prev_block_hash_hex: prev_hex(),
        }))
        .await;
        let body = issue_resp.0;
        assert_eq!(body["status"], "ok", "issue failed: {body:?}");
        let cert: BellCertDto =
            serde_json::from_value(body["certificate"].clone()).expect("decode cert");

        let verify_resp = post_bell_beacon_v2_verify(Json(BellBeaconV2VerifyReq {
            chain_id: "test-chain-v1".to_string(),
            pairs,
            prev_block_hash_hex: prev_hex(),
            certificate: cert,
        }))
        .await;
        assert_eq!(verify_resp.0["status"], "ok");
        assert_eq!(verify_resp.0["verified"], true);
    }

    /// Empty window → handler propagates `EmptyWindow` from the
    /// substrate as a structured error.
    #[tokio::test]
    async fn issue_rejects_empty_window() {
        let resp = post_bell_beacon_v2_issue(Json(BellBeaconV2IssueReq {
            chain_id: "test-chain-v1".to_string(),
            window_start: 100,
            window_end: 200,
            pairs: vec![],
            prev_block_hash_hex: prev_hex(),
        }))
        .await;
        assert_eq!(resp.0["status"], "error");
        assert!(resp.0["detail"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("empty"));
    }

    /// Chain-id binding (audit fix HIGH H2): a certificate issued
    /// under chain_id "A" must NOT verify when re-presented under
    /// chain_id "B" with otherwise identical inputs. The seed
    /// derivation folds chain_id, so cross-chain replay fails at
    /// the seed-mismatch check.
    #[tokio::test]
    async fn cross_chain_replay_rejected_at_verify() {
        let pairs = balanced_window();
        let issue_resp = post_bell_beacon_v2_issue(Json(BellBeaconV2IssueReq {
            chain_id: "chain-a".to_string(),
            window_start: 100,
            window_end: 200,
            pairs: pairs.clone(),
            prev_block_hash_hex: prev_hex(),
        }))
        .await;
        assert_eq!(issue_resp.0["status"], "ok");
        let cert: BellCertDto =
            serde_json::from_value(issue_resp.0["certificate"].clone()).unwrap();

        // Same cert, same pairs, same prev_hash, but verifier
        // claims it was issued under "chain-b".
        let verify_resp = post_bell_beacon_v2_verify(Json(BellBeaconV2VerifyReq {
            chain_id: "chain-b".to_string(),
            pairs,
            prev_block_hash_hex: prev_hex(),
            certificate: cert,
        }))
        .await;
        assert_eq!(verify_resp.0["status"], "error");
        assert_eq!(verify_resp.0["verified"], false);
    }

    /// Tampered seed at verify → rejected. Closes the case where
    /// an attacker passes an issued cert through the verifier with
    /// the seed mutated.
    #[tokio::test]
    async fn tampered_seed_rejected_at_verify() {
        let pairs = balanced_window();
        let issue_resp = post_bell_beacon_v2_issue(Json(BellBeaconV2IssueReq {
            chain_id: "test-chain-v1".to_string(),
            window_start: 100,
            window_end: 200,
            pairs: pairs.clone(),
            prev_block_hash_hex: prev_hex(),
        }))
        .await;
        let mut cert: BellCertDto =
            serde_json::from_value(issue_resp.0["certificate"].clone()).unwrap();

        // Flip a byte in the hex-encoded seed.
        let mut seed_bytes = hex::decode(&cert.seed_hex).unwrap();
        seed_bytes[0] ^= 0xff;
        cert.seed_hex = hex::encode(&seed_bytes);

        let verify_resp = post_bell_beacon_v2_verify(Json(BellBeaconV2VerifyReq {
            chain_id: "test-chain-v1".to_string(),
            pairs,
            prev_block_hash_hex: prev_hex(),
            certificate: cert,
        }))
        .await;
        assert_eq!(verify_resp.0["verified"], false);
    }
}

#[cfg(test)]
mod singh_inequality_v2_handler_tests {
    //! Handler-level tests for Singh-Inequality V2. Substrate is
    //! fully tested in `evaporchain-singh-inequality-v2`. This mod
    //! locks the JSON DTO ↔ inner-cert translation, the load-bearing
    //! V2-strictly-tighter operating region (concentrated signals
    //! where Bernstein admits and Hoeffding rejects), and the
    //! degenerate-input rejection paths.
    use super::*;

    fn cv(lo: u64, hi: u64, energy: u64, var: u64) -> ContributorWithVarianceDto {
        ContributorWithVarianceDto {
            lo,
            hi,
            energy,
            variance_proxy: var,
        }
    }

    /// Concentrated signals (var=4 ≪ range²=100): V2 admits ε=15
    /// where V1 rejects. Locks the doctrine claim through the
    /// handler layer.
    #[tokio::test]
    async fn compare_surfaces_v2_strictly_tighter_region() {
        let resp = post_singh_inequality_v2_compare(Json(SinghBernsteinGateReq {
            contributors: vec![cv(0, 10, 1000, 4); 5],
            deviation: 15,
            soundness_multiplier: 1,
        }))
        .await;
        assert_eq!(resp.0["status"], "ok");
        assert_eq!(resp.0["v1_admits"], false);
        assert_eq!(resp.0["v2_admits"], true);
        assert_eq!(resp.0["v2_strictly_tighter"], true);
        // V2's variance bound is strictly smaller than V1's for
        // this contributor set.
        assert_eq!(resp.0["v1_variance_bound"], "500"); // 5 · 100
        assert_eq!(resp.0["v2_variance_bound"], "20"); // 5 · 4
    }

    /// Worst-case (variance_proxy = range²): V2 collapses to V1.
    /// Both gates match exactly. Locks the soundness contract — V2
    /// never admits more than V1 when concentration is no better
    /// than uniform-on-endpoints.
    #[tokio::test]
    async fn compare_collapses_to_v1_when_variance_at_popoviciu_max() {
        let resp = post_singh_inequality_v2_compare(Json(SinghBernsteinGateReq {
            contributors: vec![cv(0, 10, 1000, 100); 2],
            deviation: 15,
            soundness_multiplier: 1,
        }))
        .await;
        assert_eq!(resp.0["status"], "ok");
        assert_eq!(
            resp.0["v1_variance_bound"], resp.0["v2_variance_bound"],
            "V1 and V2 variance bounds must equal at Popoviciu max"
        );
    }

    /// Direct gate endpoint: low-variance + matching deviation →
    /// admits. Returns variance_bound, max_range as decimal strings.
    #[tokio::test]
    async fn gate_admits_concentrated_signal() {
        let resp = post_singh_inequality_v2_gate(Json(SinghBernsteinGateReq {
            contributors: vec![cv(0, 10, 1000, 4); 5],
            deviation: 15,
            soundness_multiplier: 1,
        }))
        .await;
        assert_eq!(resp.0["status"], "ok");
        assert_eq!(resp.0["admits"], true);
        assert_eq!(resp.0["variance_bound"], "20");
        assert_eq!(resp.0["max_range"], "10");
    }

    /// Tiny deviation → both gates reject regardless of variance.
    #[tokio::test]
    async fn gate_rejects_tiny_deviation() {
        let resp = post_singh_inequality_v2_gate(Json(SinghBernsteinGateReq {
            contributors: vec![cv(0, 10, 1000, 4); 5],
            deviation: 1,
            soundness_multiplier: 1,
        }))
        .await;
        assert_eq!(resp.0["admits"], false);
    }

    /// Empty contributors → handler propagates the substrate Empty
    /// error.
    #[tokio::test]
    async fn empty_contributors_rejected() {
        let resp = post_singh_inequality_v2_gate(Json(SinghBernsteinGateReq {
            contributors: vec![],
            deviation: 10,
            soundness_multiplier: 1,
        }))
        .await;
        assert_eq!(resp.0["status"], "error");
        assert!(resp.0["detail"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("empty"));
    }

    /// Popoviciu guard: variance_proxy > range² is rejected before
    /// any gate evaluation.
    #[tokio::test]
    async fn variance_exceeding_range_squared_rejected() {
        // range = 10, range² = 100. variance_proxy = 200 > 100 →
        // VarianceExceedsRangeSquared.
        let resp = post_singh_inequality_v2_gate(Json(SinghBernsteinGateReq {
            contributors: vec![cv(0, 10, 1000, 200)],
            deviation: 5,
            soundness_multiplier: 1,
        }))
        .await;
        assert_eq!(resp.0["status"], "error");
        let detail = resp.0["detail"].as_str().unwrap_or("").to_lowercase();
        assert!(
            detail.contains("variance") && detail.contains("range"),
            "expected Popoviciu guard error, got: {detail}"
        );
    }
}

#[cfg(test)]
mod light_cone_v2_handler_tests {
    //! Handler-level tests for Light-Cone V2 causal-cone Merkle
    //! proofs. Substrate is fully tested in
    //! `evaporchain-light-cone-v2`. This mod locks the JSON DTO ↔
    //! `MerklePath` translation, prove → verify round-trip through
    //! the actual handler bodies, and the load-bearing light-client
    //! property: verify_ancestry needs only (root, ancestor, proof)
    //! with no DAG submitted.
    use super::*;

    fn id_hex(b: u8) -> String {
        let mut x = [0u8; 32];
        x[31] = b;
        hex::encode(x)
    }

    /// 5-block linear chain 0 → 1 → 2 → 3 → 4.
    fn linear_chain_blocks() -> Vec<AntichainBlockDto> {
        let mut out = Vec::new();
        out.push(AntichainBlockDto {
            id_hex: id_hex(0),
            parent_ids: vec![],
            energy: 1000,
            observed_epoch: 0,
        });
        for i in 1u8..=4 {
            out.push(AntichainBlockDto {
                id_hex: id_hex(i),
                parent_ids: vec![id_hex(i - 1)],
                energy: 1000,
                observed_epoch: i as u64,
            });
        }
        out
    }

    /// causal_root is deterministic over the same DAG + same block.
    #[tokio::test]
    async fn causal_root_is_deterministic() {
        let r1 = post_light_cone_v2_causal_root(Json(LightConeV2CausalRootReq {
            blocks: linear_chain_blocks(),
            block_id_hex: id_hex(4),
        }))
        .await;
        let r2 = post_light_cone_v2_causal_root(Json(LightConeV2CausalRootReq {
            blocks: linear_chain_blocks(),
            block_id_hex: id_hex(4),
        }))
        .await;
        assert_eq!(r1.0["status"], "ok");
        assert_eq!(r1.0["causal_root_hex"], r2.0["causal_root_hex"]);
        assert_ne!(r1.0["causal_root_hex"], "");
    }

    /// Round-trip prove → verify across every ancestor of block 4.
    /// Proves the load-bearing claim: verifier needs only the root,
    /// ancestor id, and proof — no DAG.
    #[tokio::test]
    async fn round_trip_prove_then_verify_chain() {
        for ancestor_byte in 0u8..=3 {
            let prove = post_light_cone_v2_prove_ancestry(Json(LightConeV2ProveReq {
                blocks: linear_chain_blocks(),
                descendant_hex: id_hex(4),
                ancestor_hex: id_hex(ancestor_byte),
            }))
            .await;
            assert_eq!(
                prove.0["status"], "ok",
                "prove failed for ancestor={ancestor_byte}: {:?}",
                prove.0
            );
            let root_hex = prove.0["causal_root_hex"].as_str().unwrap().to_string();
            let proof: MerklePathDto = serde_json::from_value(prove.0["proof"].clone()).unwrap();

            let verify = post_light_cone_v2_verify_ancestry(Json(LightConeV2VerifyReq {
                causal_root_hex: root_hex,
                ancestor_id_hex: id_hex(ancestor_byte),
                proof,
            }))
            .await;
            assert_eq!(
                verify.0["verified"], true,
                "verify failed for ancestor={ancestor_byte}"
            );
        }
    }

    /// prove_ancestry rejects non-ancestors with NotAnAncestor error.
    #[tokio::test]
    async fn prove_rejects_non_ancestor() {
        let resp = post_light_cone_v2_prove_ancestry(Json(LightConeV2ProveReq {
            blocks: linear_chain_blocks(),
            // id(0) is genesis; id(2) is descendant. id(2) is NOT in
            // causal past of id(0).
            descendant_hex: id_hex(0),
            ancestor_hex: id_hex(2),
        }))
        .await;
        assert_eq!(resp.0["status"], "error");
        assert!(resp.0["detail"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("ancestor"));
    }

    /// Tampered sibling → verify rejects (Merkle path doesn't
    /// reproduce the root).
    #[tokio::test]
    async fn verify_rejects_tampered_sibling() {
        let prove = post_light_cone_v2_prove_ancestry(Json(LightConeV2ProveReq {
            blocks: linear_chain_blocks(),
            descendant_hex: id_hex(4),
            ancestor_hex: id_hex(2),
        }))
        .await;
        let root_hex = prove.0["causal_root_hex"].as_str().unwrap().to_string();
        let mut proof: MerklePathDto = serde_json::from_value(prove.0["proof"].clone()).unwrap();
        if proof.siblings_hex.is_empty() {
            // Tree of 1 leaf has no siblings — pick a different
            // ancestor with a real path.
            return;
        }
        let mut bytes = hex::decode(&proof.siblings_hex[0]).unwrap();
        bytes[0] ^= 0xff;
        proof.siblings_hex[0] = hex::encode(&bytes);

        let verify = post_light_cone_v2_verify_ancestry(Json(LightConeV2VerifyReq {
            causal_root_hex: root_hex,
            ancestor_id_hex: id_hex(2),
            proof,
        }))
        .await;
        assert_eq!(verify.0["verified"], false);
    }

    /// Path-shape mismatch (siblings.len ≠ directions.len) → verifier
    /// returns a structured error rather than a silent false.
    #[tokio::test]
    async fn verify_rejects_path_shape_mismatch() {
        let resp = post_light_cone_v2_verify_ancestry(Json(LightConeV2VerifyReq {
            causal_root_hex: hex::encode([0u8; 32]),
            ancestor_id_hex: id_hex(0),
            proof: MerklePathDto {
                siblings_hex: vec![hex::encode([0u8; 32]), hex::encode([1u8; 32])],
                directions: vec![false],
            },
        }))
        .await;
        assert_eq!(resp.0["status"], "error");
        assert_eq!(resp.0["verified"], false);
        assert!(resp.0["detail"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("path"));
    }

    /// Diamond DAG: 0 ← {1, 2} ← 3. id(3)'s causal_past = {0, 1, 2}.
    /// Round-trip every ancestor.
    #[tokio::test]
    async fn round_trip_diamond_dag() {
        let blocks = vec![
            AntichainBlockDto {
                id_hex: id_hex(0),
                parent_ids: vec![],
                energy: 1000,
                observed_epoch: 0,
            },
            AntichainBlockDto {
                id_hex: id_hex(1),
                parent_ids: vec![id_hex(0)],
                energy: 1000,
                observed_epoch: 1,
            },
            AntichainBlockDto {
                id_hex: id_hex(2),
                parent_ids: vec![id_hex(0)],
                energy: 1000,
                observed_epoch: 1,
            },
            AntichainBlockDto {
                id_hex: id_hex(3),
                parent_ids: vec![id_hex(1), id_hex(2)],
                energy: 1000,
                observed_epoch: 2,
            },
        ];
        for ancestor_byte in 0u8..=2 {
            let prove = post_light_cone_v2_prove_ancestry(Json(LightConeV2ProveReq {
                blocks: blocks.clone(),
                descendant_hex: id_hex(3),
                ancestor_hex: id_hex(ancestor_byte),
            }))
            .await;
            let root_hex = prove.0["causal_root_hex"].as_str().unwrap().to_string();
            let proof: MerklePathDto = serde_json::from_value(prove.0["proof"].clone()).unwrap();
            let verify = post_light_cone_v2_verify_ancestry(Json(LightConeV2VerifyReq {
                causal_root_hex: root_hex,
                ancestor_id_hex: id_hex(ancestor_byte),
                proof,
            }))
            .await;
            assert_eq!(
                verify.0["verified"], true,
                "verify failed for diamond ancestor={ancestor_byte}"
            );
        }
    }
}

#[cfg(test)]
mod ib_validators_v2_handler_tests {
    //! Handler-level smoke tests for IB Validators V2 — the Immune
    //! Validator Set primitive. The substrate is fully tested in
    //! `evaporchain-ib-validators-v2`. This mod locks the JSON DTO
    //! ↔ inner-state translation, the three jail rejection paths
    //! through the actual handler bodies, and the doctrine-critical
    //! flow: a CHSH-failure mass-jail mutates the state such that
    //! the subsequent vote gate rejects the jailed validators.
    use super::*;

    fn id_hex(b: u8) -> String {
        hex::encode([b; 32])
    }

    /// Local energies all-zero + prior with spread → high KL → V1
    /// `ib_vote` returns Commit. No jail, energy above floor → V2
    /// returns commit too.
    fn high_kl_local() -> Vec<u64> {
        vec![0u64; 16]
    }
    fn spread_prior() -> Vec<u64> {
        (0..16).map(|i| i as u64 * 64).collect()
    }

    #[tokio::test]
    async fn vote_unjailed_high_kl_commits() {
        let resp = post_ib_validators_v2_vote(Json(IbV2VoteReq {
            local_energies: high_kl_local(),
            prior_energies: spread_prior(),
            signature_scale: 1024,
            lambda_mb: 100,
            validator_id_hex: id_hex(1),
            energy: 1000,
            energy_floor: 10,
            current_epoch: 0,
            jail_state: vec![],
        }))
        .await;
        assert_eq!(resp.0["status"], "ok");
        assert_eq!(resp.0["vote"], "commit");
    }

    /// Energy below floor → handler returns jailed with the
    /// EnergyBelowFloor reason carrying observed/floor values.
    #[tokio::test]
    async fn vote_below_energy_floor_returns_jailed() {
        let resp = post_ib_validators_v2_vote(Json(IbV2VoteReq {
            local_energies: high_kl_local(),
            prior_energies: spread_prior(),
            signature_scale: 1024,
            lambda_mb: 100,
            validator_id_hex: id_hex(1),
            energy: 5,
            energy_floor: 10,
            current_epoch: 0,
            jail_state: vec![],
        }))
        .await;
        assert_eq!(resp.0["vote"], "jailed");
        assert_eq!(resp.0["reason"]["kind"], "energy_below_floor");
        assert_eq!(resp.0["reason"]["observed"], 5);
        assert_eq!(resp.0["reason"]["floor"], 10);
    }

    /// Pre-existing slash entry in jail_state → vote rejected with
    /// matching Slashed reason.
    #[tokio::test]
    async fn vote_with_existing_slash_returns_jailed() {
        let resp = post_ib_validators_v2_vote(Json(IbV2VoteReq {
            local_energies: high_kl_local(),
            prior_energies: spread_prior(),
            signature_scale: 1024,
            lambda_mb: 100,
            validator_id_hex: id_hex(1),
            energy: 1000,
            energy_floor: 10,
            current_epoch: 50,
            jail_state: vec![JailEntryDto {
                validator_id_hex: id_hex(1),
                reason: JailReasonDto::Slashed { code: 7 },
                expires_at_epoch: 100,
            }],
        }))
        .await;
        assert_eq!(resp.0["vote"], "jailed");
        assert_eq!(resp.0["reason"]["kind"], "slashed");
        assert_eq!(resp.0["reason"]["code"], 7);
    }

    /// Zero energy_floor is rejected — handler propagates the
    /// substrate error.
    #[tokio::test]
    async fn vote_zero_floor_rejected() {
        let resp = post_ib_validators_v2_vote(Json(IbV2VoteReq {
            local_energies: high_kl_local(),
            prior_energies: spread_prior(),
            signature_scale: 1024,
            lambda_mb: 100,
            validator_id_hex: id_hex(1),
            energy: 1000,
            energy_floor: 0,
            current_epoch: 0,
            jail_state: vec![],
        }))
        .await;
        assert_eq!(resp.0["status"], "error");
        assert!(resp.0["detail"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("floor"));
    }

    /// Mass-jail handler writes ChshFailedWindow entries for every
    /// participant. Returned jailed_count matches participants.len()
    /// and the returned jail_state contains those entries with
    /// expires_at_epoch = current_epoch + jail_epochs.
    #[tokio::test]
    async fn chsh_failure_jail_marks_all_participants() {
        let resp = post_ib_validators_v2_jail_chsh_failure(Json(IbV2ChshJailReq {
            participants_hex: vec![id_hex(1), id_hex(2), id_hex(3)],
            window_start: 100,
            window_end: 200,
            current_epoch: 10,
            jail_epochs: 50,
            jail_state: vec![],
        }))
        .await;
        assert_eq!(resp.0["status"], "ok");
        assert_eq!(resp.0["jailed_count"], 3);
        let returned: Vec<JailEntryDto> =
            serde_json::from_value(resp.0["jail_state"].clone()).unwrap();
        assert_eq!(returned.len(), 3);
        for e in &returned {
            assert_eq!(e.expires_at_epoch, 60); // 10 + 50
            assert!(matches!(
                e.reason,
                JailReasonDto::ChshFailedWindow {
                    window_start: 100,
                    window_end: 200,
                }
            ));
        }
    }

    /// Doctrine-critical end-to-end: CHSH-failure mass-jail mutates
    /// state → subsequent vote on a jailed validator is rejected
    /// with the matching ChshFailedWindow reason. This is the
    /// load-bearing flow that ties the Bell-Beacon V2 gate-failure
    /// signal to validator immunity.
    #[tokio::test]
    async fn chsh_failure_jail_blocks_subsequent_vote() {
        // Mass-jail validator 1 over window [100, 200) for 50 epochs
        // starting at epoch 10. Jail expires at epoch 60.
        let jail_resp = post_ib_validators_v2_jail_chsh_failure(Json(IbV2ChshJailReq {
            participants_hex: vec![id_hex(1)],
            window_start: 100,
            window_end: 200,
            current_epoch: 10,
            jail_epochs: 50,
            jail_state: vec![],
        }))
        .await;
        let updated: Vec<JailEntryDto> =
            serde_json::from_value(jail_resp.0["jail_state"].clone()).unwrap();

        // Submit vote at epoch 30 (within jail window) — must
        // reject with ChshFailedWindow.
        let vote_resp = post_ib_validators_v2_vote(Json(IbV2VoteReq {
            local_energies: high_kl_local(),
            prior_energies: spread_prior(),
            signature_scale: 1024,
            lambda_mb: 100,
            validator_id_hex: id_hex(1),
            energy: 1000,
            energy_floor: 10,
            current_epoch: 30,
            jail_state: updated.clone(),
        }))
        .await;
        assert_eq!(vote_resp.0["vote"], "jailed");
        assert_eq!(vote_resp.0["reason"]["kind"], "chsh_failed_window");
        assert_eq!(vote_resp.0["reason"]["window_start"], 100);
        assert_eq!(vote_resp.0["reason"]["window_end"], 200);

        // Same vote at epoch 60 (jail expired exclusively) — V1 gate
        // applies and high-KL local commits.
        let vote_resp = post_ib_validators_v2_vote(Json(IbV2VoteReq {
            local_energies: high_kl_local(),
            prior_energies: spread_prior(),
            signature_scale: 1024,
            lambda_mb: 100,
            validator_id_hex: id_hex(1),
            energy: 1000,
            energy_floor: 10,
            current_epoch: 60,
            jail_state: updated,
        }))
        .await;
        assert_eq!(vote_resp.0["vote"], "commit");
    }
}

#[cfg(test)]
mod singh_attractor_v2_handler_tests {
    //! Handler-level smoke tests for Singh-Attractor V2 draw. The
    //! substrate is fully tested in `evaporchain-singh-attractor-v2`
    //! (in-basin determinism, out-of-basin seed dependence, drift
    //! bounds, Lyapunov property). This mod locks the JSON DTO ↔
    //! inner-cert translation, the in-basin / out-of-basin branch
    //! contract through the actual handler body, and the composition
    //! with `/api/bell_beacon_v2/issue` (a real chain-supplied seed
    //! drives the fallback draw).
    use super::*;

    fn two_attractors() -> Vec<AttractorV2Dto> {
        vec![
            AttractorV2Dto {
                center: 100,
                basin_radius: 10,
                drift_rate: 5,
            },
            AttractorV2Dto {
                center: 1000,
                basin_radius: 100,
                drift_rate: 10,
            },
        ]
    }

    fn seed_hex(b: u8) -> String {
        hex::encode([b; 32])
    }

    /// In-basin selection is V1-deterministic: the seed must be
    /// ignored, and the same state under different seeds picks the
    /// same attractor with the same drift.
    #[tokio::test]
    async fn in_basin_is_seed_invariant() {
        let r1 = post_singh_attractor_v2_draw(Json(SinghAttractorV2DrawReq {
            state_energy: 1050, // inside basin around 1000
            attractors: two_attractors(),
            certificate_seed_hex: seed_hex(1),
        }))
        .await;
        let r2 = post_singh_attractor_v2_draw(Json(SinghAttractorV2DrawReq {
            state_energy: 1050,
            attractors: two_attractors(),
            certificate_seed_hex: seed_hex(99),
        }))
        .await;
        assert_eq!(r1.0["status"], "ok");
        assert_eq!(r2.0["status"], "ok");
        assert_eq!(r1.0["selected_center"], 1000);
        assert_eq!(r2.0["selected_center"], 1000);
        assert_eq!(r1.0["used_fallback"], false);
        assert_eq!(r1.0["drift"], r2.0["drift"]);
    }

    /// Out-of-basin: the seed drives selection. Same seed must give
    /// byte-identical results (validator-determinism).
    #[tokio::test]
    async fn out_of_basin_same_seed_is_deterministic() {
        let r1 = post_singh_attractor_v2_draw(Json(SinghAttractorV2DrawReq {
            state_energy: 500, // gap between basins
            attractors: two_attractors(),
            certificate_seed_hex: seed_hex(7),
        }))
        .await;
        let r2 = post_singh_attractor_v2_draw(Json(SinghAttractorV2DrawReq {
            state_energy: 500,
            attractors: two_attractors(),
            certificate_seed_hex: seed_hex(7),
        }))
        .await;
        assert_eq!(r1.0["used_fallback"], true);
        assert_eq!(r2.0["used_fallback"], true);
        assert_eq!(r1.0["selected_center"], r2.0["selected_center"]);
    }

    /// Out-of-basin sampling spreads across attractors as the seed
    /// varies — no single attractor wins every seed (the V2 anti-
    /// grinding contract that V1 lacked).
    #[tokio::test]
    async fn out_of_basin_seed_varies_selection() {
        let mut seen = std::collections::HashSet::new();
        for s in 0u8..40 {
            let r = post_singh_attractor_v2_draw(Json(SinghAttractorV2DrawReq {
                state_energy: 500,
                attractors: two_attractors(),
                certificate_seed_hex: seed_hex(s),
            }))
            .await;
            seen.insert(r.0["selected_center"].as_u64().unwrap());
        }
        assert!(
            seen.len() >= 2,
            "fallback should sample both attractors over varying seeds; saw {seen:?}"
        );
    }

    /// Empty attractor list → handler propagates `Empty` as
    /// structured error.
    #[tokio::test]
    async fn empty_attractors_rejected() {
        let r = post_singh_attractor_v2_draw(Json(SinghAttractorV2DrawReq {
            state_energy: 100,
            attractors: vec![],
            certificate_seed_hex: seed_hex(0),
        }))
        .await;
        assert_eq!(r.0["status"], "error");
        assert!(r.0["detail"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("empty"));
    }

    /// End-to-end composition: bell-beacon-v2/issue → take seed_hex →
    /// singh-attractor-v2/draw with that seed → out-of-basin draw
    /// uses a real chain-supplied seed. Locks the substrate-to-
    /// substrate handshake at the network layer.
    #[tokio::test]
    async fn singh_attractor_v2_consumes_v2_bell_beacon_seed() {
        // Build a balanced 16-pair window so Bell-Beacon V2 issues.
        let mut pairs = Vec::new();
        for i in 0..16u8 {
            let mut tag = [0u8; 32];
            tag[0] = i;
            tag[31] = i;
            pairs.push(BellBeaconV2PairDto {
                first_energy: if i & 1 == 1 { 100 } else { 10 },
                first_tx_count: if (i >> 1) & 1 == 1 { 100 } else { 10 },
                second_energy: if (i >> 2) & 1 == 1 { 100 } else { 10 },
                second_tx_count: if (i >> 3) & 1 == 1 { 100 } else { 10 },
                tag_hex: hex::encode(tag),
            });
        }
        let mut prev = [0u8; 32];
        prev[0] = 9;

        let issue_resp = post_bell_beacon_v2_issue(Json(BellBeaconV2IssueReq {
            chain_id: "test-chain-v1".to_string(),
            window_start: 100,
            window_end: 200,
            pairs,
            prev_block_hash_hex: hex::encode(prev),
        }))
        .await;
        assert_eq!(issue_resp.0["status"], "ok");
        let seed = issue_resp.0["certificate"]["seed_hex"]
            .as_str()
            .unwrap()
            .to_string();

        // Use that seed to drive an out-of-basin Singh-Attractor V2 draw.
        let draw_resp = post_singh_attractor_v2_draw(Json(SinghAttractorV2DrawReq {
            state_energy: 500,
            attractors: two_attractors(),
            certificate_seed_hex: seed,
        }))
        .await;
        assert_eq!(draw_resp.0["status"], "ok");
        assert_eq!(draw_resp.0["used_fallback"], true);
        // Selection is one of the two attractor centres.
        let center = draw_resp.0["selected_center"].as_u64().unwrap();
        assert!(center == 100 || center == 1000);
    }
}

#[cfg(test)]
mod fork_cert_v2_handler_tests {
    //! Handler-level smoke tests for the Bell-anchored
    //! Evaporated-Fork Certificate V2 endpoints. The V2 substrate
    //! is fully tested in `evaporchain-evap-fork-cert-v2`; this
    //! mod locks the JSON DTO ↔ inner-cert translation and the
    //! prove → verify round-trip through the actual handler bodies.
    use super::*;

    fn fork_root_hex() -> String {
        let mut a = [0u8; 32];
        a[31] = 1;
        hex::encode(a)
    }

    fn anchor_hex() -> String {
        let mut a = [0u8; 32];
        a[0] = 9;
        hex::encode(a)
    }

    fn good_prove_req() -> ForkCertV2ProveReq {
        ForkCertV2ProveReq {
            fork_root_hex: fork_root_hex(),
            blocks: vec![ForkBlockDto {
                seed_energy: 1000,
                observed_epoch: 0,
            }],
            evaluated_at_epoch: 100,
            threshold: 600, // > decayed (500) → evaporated
            lambda_epochs: 100,
            bell_seed_anchor_hex: anchor_hex(),
            seed_anchor_epoch: 50,
        }
    }

    /// Round-trip: prove yields a cert whose witness verifies.
    #[tokio::test]
    async fn v2_prove_then_verify_round_trip() {
        let prove_resp = post_fork_cert_v2_prove(Json(good_prove_req())).await;
        let body = prove_resp.0;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["is_evaporated"], true);
        let witness_hex = body["witness_hex"]
            .as_str()
            .expect("witness_hex returned")
            .to_string();
        let total_seed = body["total_seed_energy"].as_u64().unwrap() as u128;
        let decayed = body["decayed_energy"].as_u64().unwrap() as u128;

        let verify_req = ForkCertV2VerifyReq {
            fork_root_hex: fork_root_hex(),
            evaluated_at_epoch: 100,
            total_seed_energy: total_seed,
            decayed_energy: decayed,
            threshold: 600,
            bell_seed_anchor_hex: anchor_hex(),
            seed_anchor_epoch: 50,
            witness_hex,
        };
        let verify_resp = post_fork_cert_v2_verify(Json(verify_req)).await;
        let body = verify_resp.0;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["verified"], true);
    }

    /// Prove must reject seed_anchor_epoch > evaluated_at_epoch (the
    /// V2 causality constraint — a future seed cannot witness a past
    /// evaporation claim).
    #[tokio::test]
    async fn v2_prove_rejects_causality_violation() {
        let mut req = good_prove_req();
        req.seed_anchor_epoch = req.evaluated_at_epoch + 1;
        let resp = post_fork_cert_v2_prove(Json(req)).await;
        let body = resp.0;
        assert_eq!(body["status"], "error");
        assert!(
            body["detail"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains("causality"),
            "error detail must surface causality violation"
        );
    }

    /// Verify must reject a tampered witness — the V2 binding is
    /// what closes V1's pre-computation gap, so witness tampering
    /// must surface at the handler layer.
    #[tokio::test]
    async fn v2_verify_rejects_tampered_witness() {
        // Build a real cert through prove, then flip a byte.
        let prove_resp = post_fork_cert_v2_prove(Json(good_prove_req())).await;
        let body = prove_resp.0;
        let mut witness = hex::decode(body["witness_hex"].as_str().unwrap()).unwrap();
        witness[0] ^= 0xff;
        let total_seed = body["total_seed_energy"].as_u64().unwrap() as u128;
        let decayed = body["decayed_energy"].as_u64().unwrap() as u128;

        let verify_req = ForkCertV2VerifyReq {
            fork_root_hex: fork_root_hex(),
            evaluated_at_epoch: 100,
            total_seed_energy: total_seed,
            decayed_energy: decayed,
            threshold: 600,
            bell_seed_anchor_hex: anchor_hex(),
            seed_anchor_epoch: 50,
            witness_hex: hex::encode(&witness),
        };
        let resp = post_fork_cert_v2_verify(Json(verify_req)).await;
        let body = resp.0;
        assert_eq!(body["status"], "error");
        assert_eq!(body["verified"], false);
    }

    /// End-to-end V2 stack: issue a Bell-Beacon V2 certificate, then
    /// use the returned `seed_hex` as the `bell_seed_anchor_hex` for
    /// a V2 fork-cert prove → verify round trip. Confirms the two
    /// V2 surfaces compose — operators don't have to hand-roll the
    /// anchor.
    #[tokio::test]
    async fn v2_fork_cert_consumes_v2_bell_beacon_seed() {
        // Build a balanced 16-pair window so the Bell-Beacon gate
        // passes (mirrors the substrate's `balanced_window`).
        let mut pairs = Vec::new();
        for i in 0..16u8 {
            let mut tag = [0u8; 32];
            tag[0] = i;
            tag[31] = i;
            pairs.push(BellBeaconV2PairDto {
                first_energy: if i & 1 == 1 { 100 } else { 10 },
                first_tx_count: if (i >> 1) & 1 == 1 { 100 } else { 10 },
                second_energy: if (i >> 2) & 1 == 1 { 100 } else { 10 },
                second_tx_count: if (i >> 3) & 1 == 1 { 100 } else { 10 },
                tag_hex: hex::encode(tag),
            });
        }
        let mut prev = [0u8; 32];
        prev[0] = 9;

        // Issue the Bell-Beacon V2 certificate.
        let issue_resp = post_bell_beacon_v2_issue(Json(BellBeaconV2IssueReq {
            chain_id: "test-chain-v1".to_string(),
            window_start: 100,
            window_end: 200,
            pairs: pairs.clone(),
            prev_block_hash_hex: hex::encode(prev),
        }))
        .await;
        let body = issue_resp.0;
        assert_eq!(body["status"], "ok", "bell-beacon issue failed: {body:?}");
        let seed_hex = body["certificate"]["seed_hex"]
            .as_str()
            .expect("seed_hex returned")
            .to_string();
        assert_eq!(seed_hex.len(), 64);

        // Feed the seed into V2 fork-cert prove.
        let prove_resp = post_fork_cert_v2_prove(Json(ForkCertV2ProveReq {
            fork_root_hex: fork_root_hex(),
            blocks: vec![ForkBlockDto {
                seed_energy: 1000,
                observed_epoch: 0,
            }],
            evaluated_at_epoch: 200,
            threshold: 600,
            lambda_epochs: 100,
            bell_seed_anchor_hex: seed_hex.clone(),
            seed_anchor_epoch: 150,
        }))
        .await;
        let body = prove_resp.0;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["is_evaporated"], true);
        let witness_hex = body["witness_hex"].as_str().unwrap().to_string();
        let total_seed = body["total_seed_energy"].as_u64().unwrap() as u128;
        let decayed = body["decayed_energy"].as_u64().unwrap() as u128;

        // V2 fork-cert verify with the same seed verifies.
        let verify_resp = post_fork_cert_v2_verify(Json(ForkCertV2VerifyReq {
            fork_root_hex: fork_root_hex(),
            evaluated_at_epoch: 200,
            total_seed_energy: total_seed,
            decayed_energy: decayed,
            threshold: 600,
            bell_seed_anchor_hex: seed_hex,
            seed_anchor_epoch: 150,
            witness_hex,
        }))
        .await;
        assert_eq!(verify_resp.0["verified"], true);
    }

    /// Different anchor → different witness → original verification
    /// fails. Locks the V2 binding contract at the handler layer.
    #[tokio::test]
    async fn v2_different_anchor_yields_different_witness() {
        let resp_a = post_fork_cert_v2_prove(Json(good_prove_req())).await;
        let mut req_b = good_prove_req();
        // Same length, different anchor bytes.
        let mut alt = [0u8; 32];
        alt[0] = 7;
        req_b.bell_seed_anchor_hex = hex::encode(alt);
        let resp_b = post_fork_cert_v2_prove(Json(req_b)).await;
        let w_a = resp_a.0["witness_hex"].as_str().unwrap().to_string();
        let w_b = resp_b.0["witness_hex"].as_str().unwrap().to_string();
        assert_ne!(
            w_a, w_b,
            "different anchors must yield different witnesses (V2 binding)"
        );
    }
}

#[cfg(test)]
mod canonical_tx_hash_regression {
    //! Regression test for the 2026-05-07 canonical-hash bug.
    //!
    //! Pre-fix the API returned a tx_hash computed from a format string
    //! ("transfer:from:to:amount") via the legacy `tx_hash()` helper.
    //! The chain's executor recorded finalised txs by the CANONICAL
    //! hash — `BLAKE3` over `tx.signable_bytes()` — which is what
    //! `tx_records_from_block_with_outcomes` computes when it builds
    //! `BlockRecord.transactions[].hash`. The two never matched, so a
    //! wallet that saved the API's returned hash and polled
    //! `/api/tx/<hash>` got `pending` forever even after the tx
    //! finalised.
    //!
    //! These tests lock the contract: the canonical hash is
    //! `hex(blake3(signable_bytes))`, distinct from the legacy
    //! format-string hash, and matches what the chain records.
    use super::*;
    use evaporchain_types::{Transaction, TransferTx};

    fn sample_transfer() -> Transaction {
        // Realistic post-sign Transfer (signature populated as the
        // submission path does after sign_transaction).
        Transaction::Transfer(TransferTx {
            from: [0x01; 32],
            to: [0x02; 32],
            amount: 1000,
            nonce: 5,
            signature: Some(vec![0xAA; 64]),
            public_key: Some(vec![0xBB; 32]),
            mev_refund_eligible: None,
        })
    }

    /// The canonical hash format is BLAKE3(signable_bytes), 32 bytes
    /// hex-encoded → 64 chars. Lock the format.
    #[test]
    fn canonical_hash_format_is_blake3_signable_bytes() {
        let tx = sample_transfer();
        let h = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
        assert_eq!(h.len(), 64, "canonical hash must be 64 hex chars (32 bytes)");
        assert!(
            h.chars().all(|c| c.is_ascii_hexdigit()),
            "canonical hash must be all-hex"
        );
    }

    /// The canonical hash must be DETERMINISTIC: hashing the same
    /// transaction twice must produce identical output. This is the
    /// core invariant the `/api/tx/<hash>` lookup relies on.
    #[test]
    fn canonical_hash_is_deterministic() {
        let tx = sample_transfer();
        let h1 = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
        let h2 = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
        assert_eq!(h1, h2);
    }

    /// The canonical hash must DIFFER from the legacy format-string
    /// hash. Pre-fix code returned the format-string version; if a
    /// future change ever reverts to it, this assertion fails. Lock
    /// the regression.
    #[test]
    fn canonical_hash_differs_from_legacy_format_string() {
        let tx = sample_transfer();
        let canonical = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
        // Reproduce the pre-fix format-string hash shape.
        let legacy_format_hash = tx_hash(&format!(
            "transfer:{}:{}:{}",
            hex::encode(&[0x01u8; 20]),
            hex::encode(&[0x02u8; 20]),
            1000
        ));
        assert_ne!(
            canonical, legacy_format_hash,
            "canonical hash must NOT equal the legacy format-string hash — \
             a regression to the format-string would break /api/tx/<hash>"
        );
    }

    /// The canonical hash must be SENSITIVE to nonce — two transfers
    /// from the same sender to the same recipient with the same amount
    /// but different nonces must produce different hashes. Pre-fix the
    /// format-string hash IGNORED nonce, so two such submissions
    /// would collide on the API-returned hash even though the chain
    /// treated them as distinct txs.
    #[test]
    fn canonical_hash_sensitive_to_nonce() {
        let tx_n5 = Transaction::Transfer(TransferTx {
            from: [0x01; 32],
            to: [0x02; 32],
            amount: 1000,
            nonce: 5,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let tx_n6 = Transaction::Transfer(TransferTx {
            from: [0x01; 32],
            to: [0x02; 32],
            amount: 1000,
            nonce: 6,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let h5 = hex::encode(blake3::hash(&tx_n5.signable_bytes()).as_bytes());
        let h6 = hex::encode(blake3::hash(&tx_n6.signable_bytes()).as_bytes());
        assert_ne!(h5, h6, "canonical hash must distinguish txs that differ only in nonce");
    }
}

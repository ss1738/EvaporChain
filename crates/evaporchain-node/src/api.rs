use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use axum::http::HeaderMap;
use evaporchain_consensus::MockConsensus;
use evaporchain_consensus::tendermint::TendermintConsensus;
use evaporchain_crypto::signatures::{MlDsaKeypair, Signer};
use evaporchain_da::block_da::BlockDAPackage;
use evaporchain_da::block_da_2d::BlockDA2DPackage;
use evaporchain_state::db::StateDB;
use evaporchain_state::RocksDBStateDB;
use evaporchain_types::{
    Block, CallContractTx, CreateObjectTx, DeployContractTx, ObjectState, RefreshTx, Transaction,
    TransferTx,
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
    /// Faucet rate limiter: address hex string -> last request timestamp.
    pub faucet_rate_limit: Mutex<HashMap<String, Instant>>,
    /// Pending-nonce cache. Concurrent /api/faucet (and other tx) hits
    /// would otherwise all read the same db.nonce and submit txs with
    /// identical nonce, of which only one can execute (the others hit
    /// InvalidNonce silently). The cache returns max(db.nonce, cached)
    /// and increments locally. When pending txs commit and db.nonce
    /// catches up, max takes over and the cache resyncs implicitly.
    pub pending_nonces: Mutex<HashMap<[u8; 32], u64>>,
    /// When true, the faucet skips its per-address cooldown entirely.
    /// Set by `--devnet-no-rate-limit` for stress / load testing only;
    /// `--mainnet` strict mode rejects this combination at startup.
    pub faucet_rate_limit_disabled: bool,
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
        let db_nonce = {
            let db = safe_lock(&self.db);
            db.get_account(addr).map(|a| a.nonce).unwrap_or(0)
        };
        let mut cache = safe_lock(&self.pending_nonces);
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
#[derive(Clone, Serialize, Deserialize)]
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
}

/// Accumulated chain statistics.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChainStats {
    pub total_objects_created: u64,
    pub total_evaporated: u64,
    pub total_resurrected: u64,
    pub total_refreshed: u64,
    pub total_transactions: u64,
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
    pub fn record_block(&mut self, timestamp_ms: u64, tx_count: usize, exec_time_us: u64, gas_used: u64) {
        self.recent_blocks.push_back((timestamp_ms, tx_count, exec_time_us, gas_used));
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
        let in_window: Vec<_> = self.recent_blocks.iter()
            .filter(|(ts, _, _, _)| *ts >= cutoff)
            .collect();
        if in_window.len() < 2 {
            return 0.0;
        }
        let total_txs: usize = in_window.iter().map(|(_, tc, _, _)| tc).sum();
        let span_ms = in_window.last().unwrap().0 - in_window.first().unwrap().0;
        if span_ms == 0 { return 0.0; }
        total_txs as f64 / (span_ms as f64 / 1000.0)
    }

    /// Average block execution time (microseconds) over recent blocks.
    pub fn avg_exec_time_us(&self) -> u64 {
        if self.recent_blocks.is_empty() { return 0; }
        let total: u64 = self.recent_blocks.iter().map(|(_, _, t, _)| t).sum();
        total / self.recent_blocks.len() as u64
    }

    /// Average gas used per block.
    pub fn avg_gas_per_block(&self) -> u64 {
        if self.recent_blocks.is_empty() { return 0; }
        let total: u64 = self.recent_blocks.iter().map(|(_, _, _, g)| g).sum();
        total / self.recent_blocks.len() as u64
    }

    /// Average txs per block.
    pub fn avg_txs_per_block(&self) -> f64 {
        if self.recent_blocks.is_empty() { return 0.0; }
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
pub const GENESIS_CORE_DEV:   &str = "2b91f50d68a37ce214b65903d74a8ef1c5263b90";
pub const GENESIS_VALIDATOR1:  &str = "91e5c8f23d7b4a061f9c82e640d53a17b8f26f47";
pub const GENESIS_VALIDATOR2:  &str = "4d02a7e91c3f86b5d24e0f738c915ba6e0d7a8b6";
pub const GENESIS_ECOSYSTEM:   &str = "a3f71b5e928d4c063e7a50f81d9c26b34e8a1c5e";
pub const GENESIS_COMMUNITY:   &str = "e8b12d7f94c6a35081e4f29b6d3c8a57f1e07d94";

// ──────────────────────────── Name Helpers ─────────────────────────────

fn addr_from_byte(b: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    a[0] = b;
    a
}

/// Parse a hex address string into a 32-byte array.
pub fn parse_hex_address(s: &str) -> Result<[u8; 32], String> {
    let clean = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(clean).map_err(|_| "invalid hex address".to_string())?;
    if bytes.is_empty() || bytes.len() > 32 {
        return Err("address must be 1-32 bytes".to_string());
    }
    let mut addr = [0u8; 32];
    addr[..bytes.len()].copy_from_slice(&bytes);
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
        Transaction::Delegate(t) => serde_json::json!({ "type": "delegate", "validator_id": t.validator_id, "amount": t.amount }),
        Transaction::Undelegate(t) => serde_json::json!({ "type": "undelegate", "validator_id": t.validator_id, "amount": t.amount }),
        Transaction::RotateValidatorKey(t) => serde_json::json!({
            "type": "rotate_validator_key",
            "validator_id": t.validator_id,
            "effective_epoch": t.effective_epoch,
        }),
        Transaction::ClaimDelegation(t) => serde_json::json!({
            "type": "claim_delegation",
            "validator_id": t.validator_id,
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
    Transfer { from: serde_json::Value, to: serde_json::Value, amount: u64, nonce: u64 },
    #[serde(rename = "create_object")]
    CreateObject { creator: serde_json::Value, object_id: serde_json::Value, energy: u64, half_life: u64 },
    #[serde(rename = "refresh")]
    Refresh { object_id: serde_json::Value, energy_deposit: u64 },
    #[serde(rename = "resurrect")]
    Resurrect { object_id: serde_json::Value, energy_deposit: u64 },
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
fn require_tx_auth(headers: &HeaderMap, state: &ApiState, has_signature: bool) -> Result<Option<i64>, Json<TxResultResponse>> {
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
fn require_wallet_ownership(state: &ApiState, user_id: Option<i64>, addr_hex: &str) -> Result<(), Json<TxResultResponse>> {
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

/// Sign a transaction using the appropriate keypair.
fn require_admin_auth(headers: &HeaderMap) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let expected = match std::env::var("EVAPORCHAIN_ADMIN_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => return Ok(()),
    };
    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if provided != expected {
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
        Transaction::Transfer(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
        Transaction::Refresh(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
        Transaction::CreateObject(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
        Transaction::DeployContract(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
        Transaction::CallContract(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
        Transaction::DeployScript(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
        Transaction::CallScript(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
        Transaction::ValidatorStake(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
        Transaction::ValidatorExit(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
        Transaction::ValidatorClaimStake(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
        Transaction::Shield(t) => { t.signature = Some(sig); t.public_key = Some(pk); }
        Transaction::Unshield(_) | Transaction::PrivateTransfer(_) => {} // ZK-authenticated
        Transaction::Deferred(d) => { d.signature = Some(sig); d.public_key = Some(pk); }
        Transaction::Blob(b) => { b.signature = Some(sig); b.public_key = Some(pk); }
        Transaction::Governance(g) => { g.signature = Some(sig); g.public_key = Some(pk); }
        Transaction::MultiSig(_) => {}
        Transaction::UserOp(u) => { u.signature = Some(sig); u.public_key = Some(pk); }
        Transaction::UpgradeContract(u) => { u.signature = Some(sig); u.public_key = Some(pk); }
        Transaction::Delegate(d) => { d.signature = Some(sig); d.public_key = Some(pk); }
        Transaction::Undelegate(u) => { u.signature = Some(sig); u.public_key = Some(pk); }
        Transaction::RotateValidatorKey(r) => { r.signature = Some(sig); r.public_key = Some(pk); }
        Transaction::ClaimDelegation(c) => { c.signature = Some(sig); c.public_key = Some(pk); }
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
    if parts.len() != 2 { return false; }
    let local = parts[0];
    let domain = parts[1];
    if local.is_empty() || domain.is_empty() { return false; }
    if !domain.contains('.') { return false; }
    // Reject obvious non-email characters
    let valid_chars = |c: char| c.is_alphanumeric() || "._+-".contains(c);
    local.chars().all(valid_chars) && domain.chars().all(|c| c.is_alphanumeric() || ".-".contains(c))
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

#[derive(Debug, Serialize)]
pub struct HbctStateResp {
    pub entry_count: usize,
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
        Err(e) => return Json(HbctActionResp { status: "error", detail: format!("bad from: {e}") }),
    };
    let to = match parse_hex32(&req.to_hex) {
        Ok(a) => a,
        Err(e) => return Json(HbctActionResp { status: "error", detail: format!("bad to: {e}") }),
    };
    let mut book = safe_lock(&state.hbct_book);
    match book.transfer(
        &req.delivery_location.into_bytes(),
        req.hour_slot,
        from,
        to,
        req.amount,
    ) {
        Ok(()) => Json(HbctActionResp { status: "ok", detail: "transferred".into() }),
        Err(e) => Json(HbctActionResp { status: "error", detail: format!("{e}") }),
    }
}

async fn post_hbct_burn(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<HbctBurnReq>,
) -> Json<HbctActionResp> {
    let holder = match parse_hex32(&req.holder_hex) {
        Ok(a) => a,
        Err(e) => return Json(HbctActionResp { status: "error", detail: format!("bad holder: {e}") }),
    };
    let mut book = safe_lock(&state.hbct_book);
    match book.burn(&req.delivery_location.into_bytes(), req.hour_slot, holder, req.amount) {
        Ok(()) => Json(HbctActionResp { status: "ok", detail: "burnt".into() }),
        Err(e) => Json(HbctActionResp { status: "error", detail: format!("{e}") }),
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

async fn get_mortis_cert(
    State(state): State<Arc<ApiState>>,
) -> Json<Option<MortisCertDetail>> {
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
        None => return Json(RefreshPoolResp {
            total_accrued: 0,
            credits: vec![],
        }),
    };
    let tc = safe_lock(tc);
    let raw = tc.refresh_pool_credits();
    let total: u64 = raw.iter().map(|(_, a, _)| *a).fold(0u64, |a, b| a.saturating_add(b));
    Json(RefreshPoolResp {
        total_accrued: total,
        credits: raw
            .into_iter()
            .map(|(namespace_hex, accrued, last_touched_epoch)| RefreshPoolCredit {
                namespace_hex,
                accrued,
                last_touched_epoch,
            })
            .collect(),
    })
}

async fn get_hbct_state(State(state): State<Arc<ApiState>>) -> Json<HbctStateResp> {
    let book = safe_lock(&state.hbct_book);
    Json(HbctStateResp {
        entry_count: book.len(),
    })
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
        Err(e) => return Json(HbctActionResp { status: "error", detail: format!("bad holder: {e}") }),
    };
    let mut oracle = safe_lock(&state.hbct_oracle);
    oracle.attestations.push(evaporchain_hbct::OracleAttestation {
        delivery_location: req.delivery_location.into_bytes(),
        hour_slot: req.hour_slot,
        holder,
        mwh_delivered: req.mwh_delivered,
        attested_at_epoch: req.attested_at_epoch,
    });
    Json(HbctActionResp { status: "ok", detail: "attestation recorded".into() })
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
    Json(HbctActionResp { status: "ok", detail: "registered".into() })
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
    Json(HbctActionResp { status: "ok", detail: "vote recorded".into() })
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
    let registry = evaporchain_execution::boltzmann_stake_integration::BoltzmannStakeRegistry::new();
    let decayed =
        evaporchain_execution::boltzmann_stake_integration::decayed_voting_power(
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
    let half_life = q.chain_lambda_half_life_epochs.unwrap_or(
        evaporchain_energy_kernel::ChainLambda::default_genesis().half_life(),
    );
    let observation_epoch = q.observation_epoch.unwrap_or_else(|| {
        let tc = safe_lock(tc);
        tc.height()
    });
    let tc = safe_lock(tc);
    let summary = tc.causal_cone_summary(head, half_life, observation_epoch)?;
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

// ─────────────── Light-Cone DAG observability ───────────────────────

#[derive(Debug, Serialize)]
pub struct LightConeResp {
    pub block_count: usize,
    pub running_alongside_tendermint: bool,
}

async fn get_light_cone(State(state): State<Arc<ApiState>>) -> Json<LightConeResp> {
    let tc = match state.tendermint.as_ref() {
        Some(tc) => tc,
        None => return Json(LightConeResp {
            block_count: 0,
            running_alongside_tendermint: false,
        }),
    };
    let tc = safe_lock(tc);
    Json(LightConeResp {
        block_count: tc.light_cone_block_count(),
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

async fn get_tx_by_hash(
    State(state): State<Arc<ApiState>>,
    Path(hash): Path<String>,
) -> Result<Json<TxRecord>, StatusCode> {
    let history = safe_lock(&state.block_history);
    for block in history.iter().rev() {
        for tx in &block.transactions {
            if tx.hash == hash {
                return Ok(Json(tx.clone()));
            }
        }
    }
    Err(StatusCode::NOT_FOUND)
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

async fn get_events(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<EventsQuery>,
) -> Json<EventsResponse> {
    let events = safe_lock(&state.events);
    let limit = params.limit.unwrap_or(50).min(200);
    let evts: Vec<EventRecord> = events.iter().rev().take(limit).cloned().collect();
    Json(EventsResponse { events: evts })
}

// ── Transaction submission handlers ──

async fn post_transfer(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<TransferRequest>,
) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, req.signature.is_some()) {
        Ok(uid) => uid, Err(resp) => return resp,
    };
    if req.amount == 0 {
        return Json(TxResultResponse { success: false, message: "Amount must be greater than zero".into(), tx_hash: None });
    }
    let from = match parse_address_value(&req.from) {
        Ok(a) => a, Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
    };
    let to = match parse_address_value(&req.to) {
        Ok(a) => a, Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
    };
    if from == to {
        return Json(TxResultResponse { success: false, message: "Cannot transfer to yourself".into(), tx_hash: None });
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
                return Json(TxResultResponse { success: false, message: format!("Insufficient balance: {} < {}", acct.balance, req.amount), tx_hash: None });
            }
            if req.nonce != acct.nonce {
                return Json(TxResultResponse { success: false, message: format!("Invalid nonce: expected {}, got {}", acct.nonce, req.nonce), tx_hash: None });
            }
        } else if req.amount > 0 {
            return Json(TxResultResponse { success: false, message: "Account not found — use faucet first".into(), tx_hash: None });
        }
    }
    let hash = tx_hash(&format!("transfer:{}:{}:{}", hex::encode(&from[..20]), hex::encode(&to[..20]), req.amount));
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
            return Json(TxResultResponse { success: false, message: "Duplicate transaction already in mempool".into(), tx_hash: None });
        }
    }
    {
        let mut tx = Transaction::Transfer(TransferTx {
            from, to, amount: req.amount, nonce: req.nonce,
            signature: req.signature.and_then(|s| hex::decode(s).ok()),
            public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
        });
        let sender_addr = format!("0x{}", hex::encode(from));
        sign_transaction(&mut tx, &state, Some(&sender_addr));
        state.submit_tx(tx);
    }
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Transfer queued: {} -> {} amount={}",
            account_name(&from), account_name(&to), req.amount
        ),
        tx_hash: Some(hash),
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
        Ok(a) => a, Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
    };
    let obj_id_val = match parse_address_value(&req.object_id) {
        Ok(a) => a, Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
    };
    let hash = tx_hash(&format!("create:{}:{}:{}", hex::encode(&obj_id_val[..8]), req.energy, req.half_life));
    let obj_label = hex::encode(&obj_id_val[..4]);
    let data = req.data.map(|d| d.into_bytes())
        .unwrap_or_else(|| format!("obj-0x{}", &obj_label).into_bytes());
    let mut tx = Transaction::CreateObject(CreateObjectTx {
        creator, object_id: obj_id_val, energy: req.energy, half_life: req.half_life,
        data,
        decay_curve: req.decay_curve,
        signature: req.signature.and_then(|s| hex::decode(s).ok()),
        public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
    });
    let creator_addr = format!("0x{}", hex::encode(creator));
    sign_transaction(&mut tx, &state, Some(&creator_addr));
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
        Ok(a) => a, Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
    };
    let hash = tx_hash(&format!("refresh:{}:{}", hex::encode(&obj_id_val[..8]), req.energy_deposit));
    let mut tx = Transaction::Refresh(RefreshTx {
        object_id: obj_id_val, energy_deposit: req.energy_deposit,
        signature: req.signature.and_then(|s| hex::decode(s).ok()),
        public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
    });
    sign_transaction(&mut tx, &state, None);
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!("Refresh queued: obj=0x{} energy_deposit={}", hex::encode(&obj_id_val[..4]), req.energy_deposit),
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
        Ok(a) => a, Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
    };
    let hash = tx_hash(&format!("resurrect:{}:{}", hex::encode(&obj_id_val[..8]), req.energy_deposit));
    let mut tx = Transaction::Refresh(RefreshTx {
        object_id: obj_id_val, energy_deposit: req.energy_deposit,
        signature: req.signature.and_then(|s| hex::decode(s).ok()),
        public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
    });
    sign_transaction(&mut tx, &state, None);
    state.submit_tx(tx);
    Json(TxResultResponse {
        success: true,
        message: format!("Resurrect queued: obj=0x{} energy_deposit={}", hex::encode(&obj_id_val[..4]), req.energy_deposit),
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
        return Json(BatchResponse { submitted: 0, failed: 0, results: vec![] });
    }
    if req.transactions.len() > 100 {
        return Json(BatchResponse {
            submitted: 0,
            failed: 1,
            results: vec![BatchItemResult {
                index: 0, success: false,
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
                index: 0, success: false,
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
            BatchTxItem::Transfer { from, to, amount, nonce } => {
                match (parse_address_value(&from), parse_address_value(&to)) {
                    (Ok(f), Ok(t)) if f != t && amount > 0 => {
                        let hash = tx_hash(&format!("transfer:{}:{}:{}", hex::encode(&f[..20]), hex::encode(&t[..20]), amount));
                        let mut tx = Transaction::Transfer(TransferTx {
                            from: f, to: t, amount, nonce,
                            signature: None, public_key: None,
                        });
                        sign_transaction(&mut tx, &state, None);
                        state.submit_tx(tx);
                        BatchItemResult { index: i, success: true, message: "Transfer queued".into(), tx_hash: Some(hash) }
                    }
                    (Err(e), _) | (_, Err(e)) => BatchItemResult { index: i, success: false, message: e, tx_hash: None },
                    _ => BatchItemResult { index: i, success: false, message: "Invalid transfer parameters".into(), tx_hash: None },
                }
            }
            BatchTxItem::CreateObject { creator, object_id, energy, half_life } => {
                match (parse_address_value(&creator), parse_address_value(&object_id)) {
                    (Ok(c), Ok(oid)) => {
                        let hash = tx_hash(&format!("create:{}:{}:{}", hex::encode(&oid[..8]), energy, half_life));
                        let mut tx = Transaction::CreateObject(CreateObjectTx {
                            creator: c, object_id: oid, energy, half_life,
                            data: Vec::new(), decay_curve: None,
                            signature: None, public_key: None,
                        });
                        sign_transaction(&mut tx, &state, None);
                        state.submit_tx(tx);
                        BatchItemResult { index: i, success: true, message: "CreateObject queued".into(), tx_hash: Some(hash) }
                    }
                    (Err(e), _) | (_, Err(e)) => BatchItemResult { index: i, success: false, message: e, tx_hash: None },
                }
            }
            BatchTxItem::Refresh { object_id, energy_deposit } => {
                match parse_address_value(&object_id) {
                    Ok(oid) => {
                        let hash = tx_hash(&format!("refresh:{}:{}", hex::encode(&oid[..8]), energy_deposit));
                        let mut tx = Transaction::Refresh(RefreshTx {
                            object_id: oid, energy_deposit,
                            signature: None, public_key: None,
                        });
                        sign_transaction(&mut tx, &state, None);
                        state.submit_tx(tx);
                        BatchItemResult { index: i, success: true, message: "Refresh queued".into(), tx_hash: Some(hash) }
                    }
                    Err(e) => BatchItemResult { index: i, success: false, message: e, tx_hash: None },
                }
            }
            BatchTxItem::Resurrect { object_id, energy_deposit } => {
                match parse_address_value(&object_id) {
                    Ok(oid) => {
                        let hash = tx_hash(&format!("resurrect:{}:{}", hex::encode(&oid[..8]), energy_deposit));
                        let mut tx = Transaction::Refresh(RefreshTx {
                            object_id: oid, energy_deposit,
                            signature: None, public_key: None,
                        });
                        sign_transaction(&mut tx, &state, None);
                        state.submit_tx(tx);
                        BatchItemResult { index: i, success: true, message: "Resurrect queued".into(), tx_hash: Some(hash) }
                    }
                    Err(e) => BatchItemResult { index: i, success: false, message: e, tx_hash: None },
                }
            }
        };

        if result.success { submitted += 1; } else { failed += 1; }
        results.push(result);
    }

    Json(BatchResponse { submitted, failed, results })
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
    if let Err(resp) = require_tx_auth(&headers, &state, false) { return resp; }
    let mut tx = Transaction::DeployContract(DeployContractTx {
        deployer: addr_from_byte(req.deployer),
        template: req.template.clone(),
        init_args: serde_json::to_string(&req.init_args).unwrap_or_default(),
        energy: req.energy,
        half_life: req.half_life,
        rules: req.rules.map(|r| serde_json::to_string(&r).unwrap_or_default()),
        signature: None,
        public_key: None,
    });
    sign_transaction(&mut tx, &state, None);
    state.submit_tx(tx);
    let hash = tx_hash(&format!("deploy:{}:{}:{}", req.template, req.energy, req.half_life));
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Deploy queued: template={} energy={} hl={} (mempool={})",
            req.template, req.energy, req.half_life, state.mempool_len()
        ),
        tx_hash: Some(hash),
    })
}

async fn post_call_contract(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<CallContractRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, false) { return resp; }
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
    state.submit_tx(tx);
    let hash = tx_hash(&format!("call:{}:{}:{}", req.contract_id, req.method, state.mempool_len()));
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Call queued: contract={} method={} (mempool={})",
            req.contract_id, req.method, state.mempool_len()
        ),
        tx_hash: Some(hash),
    })
}

async fn get_contracts(
    State(state): State<Arc<ApiState>>,
) -> Json<serde_json::Value> {
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
    if let Err(resp) = require_tx_auth(&headers, &state, false) { return resp; }
    let mut tx = Transaction::DeployScript(evaporchain_types::DeployScriptTx {
        deployer: addr_from_byte(req.deployer),
        source_code: req.source_code.clone(),
        energy: req.energy,
        half_life: req.half_life,
        signature: None,
        public_key: None,
    });
    sign_transaction(&mut tx, &state, None);
    state.submit_tx(tx);
    let hash = tx_hash(&format!("deploy-script:{}:{}", req.energy, req.half_life));
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Script deploy queued: energy={} hl={} source={}B (mempool={})",
            req.energy, req.half_life, req.source_code.len(), state.mempool_len()
        ),
        tx_hash: Some(hash),
    })
}

async fn post_call_script(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(req): Json<CallScriptRequest>,
) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, false) { return resp; }
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
    state.submit_tx(tx);
    let hash = tx_hash(&format!("call-script:{}:{}:{}", req.contract_id, req.method, state.mempool_len()));
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Script call queued: contract={} method={} (mempool={})",
            req.contract_id, req.method, state.mempool_len()
        ),
        tx_hash: Some(hash),
    })
}

async fn get_scripts(
    State(state): State<Arc<ApiState>>,
) -> Json<serde_json::Value> {
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

async fn get_script(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
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
        Some(sc) => {
            (StatusCode::OK, Json(serde_json::to_value(&sc.abi).unwrap())).into_response()
        }
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
    let topics = params.subscribe.clone().unwrap_or_else(|| "all".to_string());
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
    let status = if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };

    (status, Json(serde_json::json!({
        "ready": ready,
        "block_height": tip_height,
        "peers": peer_count,
        "uptime_secs": uptime_secs,
    })))
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

    let (balance, nonce) = if let Some(acct) = db.get_account(&addr_bytes) {
        (acct.balance, acct.nonce)
    } else {
        (0, 0)
    };

    // Objects owned by this address
    let objects: Vec<ObjectResponse> = db.all_object_ids().iter().filter_map(|id| {
        let obj = db.get_object(id)?;
        if obj.owner != addr_bytes { return None; }
        let current_energy = obj.energy_at(epoch);
        let decay_pct = if obj.energy > 0 {
            ((obj.energy - current_energy) as f64 / obj.energy as f64) * 100.0
        } else { 100.0 };
        let state_str = match obj.state {
            ObjectState::Active => "Active",
            ObjectState::Grace => "Grace",
            ObjectState::Ghost => "Ghost",
            ObjectState::Resurrected => "Risen",
        };
        Some(ObjectResponse {
            id: hex::encode(id), name: object_name(id, &obj.data),
            owner: hex::encode(obj.owner), owner_name: account_name(&obj.owner),
            energy: obj.energy, max_energy: obj.energy, half_life: obj.half_life,
            state: state_str.to_string(), created_epoch: obj.created_at,
            last_refreshed: obj.last_refreshed, grace_epoch: obj.grace_epoch,
            current_energy, decay_percentage: (decay_pct * 10.0).round() / 10.0,
            decay_curve: obj.decay_curve.clone(),
        })
    }).collect();
    drop(history);
    drop(db);

    // NFTs owned by this address
    let nft_store = safe_lock(&state.nft_store);
    let nfts: Vec<NftResponse> = nft_store.tokens.iter()
        .filter(|n| n.owner == full_hex || n.owner.contains(&addr_hex))
        .map(|n| nft_to_response(n, epoch))
        .collect();
    drop(nft_store);

    // Token balances
    let token_store = safe_lock(&state.token_store);
    let tokens: Vec<serde_json::Value> = token_store.tokens.iter().filter_map(|t| {
        let bal = t.balances.get(&full_hex).or_else(|| {
            t.balances.keys().find(|k| k.contains(&addr_hex)).and_then(|k| t.balances.get(k))
        }).copied().unwrap_or(0);
        if bal == 0 { return None; }
        Some(serde_json::json!({
            "token_id": t.id, "symbol": t.symbol, "name": t.name, "balance": bal
        }))
    }).collect();

    Ok(Json(AddressDetailResponse { address: full_hex, balance, nonce, objects, nfts, tokens }))
}

// ──────────────────────────── Faucet ───────────────────────────────────

const FAUCET_AMOUNT: u64 = 10_000;
const FAUCET_RATE_LIMIT_SECS: u64 = 3600; // 1 hour

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
        [(axum::http::header::CONTENT_TYPE, "application/manifest+json")],
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
    headers: HeaderMap,
    Json(req): Json<FaucetRequest>,
) -> impl IntoResponse {
    if let Err(_e) = require_admin_auth(&headers) {
        return (StatusCode::UNAUTHORIZED, Json(FaucetResponse {
            success: false, balance: 0,
            message: Some("unauthorized: invalid admin key".into()),
        }));
    }
    let addr = match parse_address_value(&req.address) {
        Ok(a) => a,
        Err(e) => return (StatusCode::OK, Json(FaucetResponse { success: false, balance: 0, message: Some(format!("Invalid address: {}", e)) })),
    };
    let addr_key = hex::encode(&addr[..20]);

    // Rate limit check (skipped entirely in --devnet-no-rate-limit mode)
    if !state.faucet_rate_limit_disabled {
        let mut limits = safe_lock(&state.faucet_rate_limit);
        if let Some(last) = limits.get(&addr_key) {
            if last.elapsed().as_secs() < FAUCET_RATE_LIMIT_SECS {
                let remaining = FAUCET_RATE_LIMIT_SECS - last.elapsed().as_secs();
                return (StatusCode::TOO_MANY_REQUESTS, Json(FaucetResponse {
                    success: false,
                    balance: 0,
                    message: Some(format!(
                        "Rate limited. Try again in {} minutes.",
                        remaining / 60 + 1
                    )),
                }));
            }
        }
        limits.insert(addr_key, Instant::now());
        // Evict expired entries to prevent unbounded memory growth
        if limits.len() > 10_000 {
            limits.retain(|_, last| last.elapsed().as_secs() < FAUCET_RATE_LIMIT_SECS);
        }
    }

    // Submit faucet as a transfer from the "faucet account" (all-zeros address)
    // through consensus so all validators see it. Reserve a unique nonce
    // via the pending-nonce cache so concurrent faucet hits don't all
    // collide on the same db.nonce (only one would land otherwise).
    let faucet_addr = [0u8; 32]; // special faucet/mint address (pre-seeded at genesis)
    let nonce = state.reserve_nonce(&faucet_addr);
    let mut tx = Transaction::Transfer(TransferTx {
        from: faucet_addr,
        to: addr,
        amount: FAUCET_AMOUNT,
        nonce,
        signature: None,
        public_key: None,
    });
    sign_transaction(&mut tx, &state, None);
    state.submit_tx(tx);

    // Return expected balance (may not be applied yet until next block)
    let balance = {
        let db = safe_lock(&state.db);
        db.get_account(&addr).map(|a| a.balance).unwrap_or(0) + FAUCET_AMOUNT
    };

    (StatusCode::OK, Json(FaucetResponse {
        success: true,
        balance,
        message: Some("Faucet transaction submitted to consensus — balance updates after next block".into()),
    }))
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
        Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
    };

    let hash = tx_hash(&format!("oracle:{}:{}:{}", req.source, hex::encode(&obj_id_val[..8]), req.energy));

    // Prepend source tag to data
    let data_str = format!("[{}] {}", req.source, req.data);

    let mut tx = Transaction::CreateObject(CreateObjectTx {
        creator,
        object_id: obj_id_val,
        energy: req.energy,
        half_life: req.half_life,
        data: data_str.into_bytes(),
        decay_curve: None,
        signature: None,
        public_key: None,
    });
    sign_transaction(&mut tx, &state, None);
    state.submit_tx(tx);

    Json(TxResultResponse {
        success: true,
        message: format!("Oracle data ingested: {} (energy={}, half_life={})", req.source, req.energy, req.half_life),
        tx_hash: Some(hash),
    })
}

// ──────────────────── Oracle Consensus Handlers ──────────────────────────

async fn get_oracle_status(
    State(state): State<Arc<ApiState>>,
) -> Json<serde_json::Value> {
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

async fn get_shard_status(
    State(state): State<Arc<ApiState>>,
) -> Json<serde_json::Value> {
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

async fn get_shard_health(
    State(state): State<Arc<ApiState>>,
) -> Json<serde_json::Value> {
    if let Some(ref sb) = state.shard_bridge {
        let bridge = safe_lock(sb);
        let healths = bridge.shard_healths();
        let candidates = bridge.find_compaction_candidates();
        let shard_data: Vec<serde_json::Value> = healths.iter().map(|h| {
            serde_json::json!({
                "shard_id": h.shard_id.0,
                "total_objects": h.total_objects,
                "live_objects": h.live_objects,
                "total_energy": h.total_energy,
                "liveness_ratio": h.liveness_ratio(),
                "is_dead": h.is_dead(),
            })
        }).collect();
        Json(serde_json::json!({
            "shards": shard_data,
            "compaction_candidates": candidates.len(),
        }))
    } else {
        Json(serde_json::json!({ "active": false }))
    }
}

// ──────────────────────────── Ghost Bridge Handlers ─────────────────────

async fn get_ghost_list(
    State(state): State<Arc<ApiState>>,
) -> Json<serde_json::Value> {
    let db = safe_lock(&state.db);
    let ghost_ids = db.all_ghost_ids();
    let ghosts: Vec<serde_json::Value> = ghost_ids.iter().take(100).filter_map(|id| {
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
    }).collect();
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

    let mut nfts: Vec<NftResponse> = store.tokens.iter().map(|n| nft_to_response(n, epoch)).collect();
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

    let nft = store.tokens.iter().find(|n| n.id == id).ok_or(StatusCode::NOT_FOUND)?;
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
        Ok(n) => n, Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
    };
    if name.is_empty() {
        return Json(TxResultResponse { success: false, message: "Name is required".into(), tx_hash: None });
    }
    if req.energy == 0 {
        return Json(TxResultResponse { success: false, message: "Energy must be greater than zero".into(), tx_hash: None });
    }
    if req.half_life == 0 {
        return Json(TxResultResponse { success: false, message: "Half-life must be greater than zero".into(), tx_hash: None });
    }
    if req.energy > 1_000_000_000 {
        return Json(TxResultResponse { success: false, message: "Energy exceeds maximum (1,000,000,000)".into(), tx_hash: None });
    }
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);

    let metadata = match sanitize_string(&req.metadata, 10_000) {
        Ok(m) => m, Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
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
        message: format!("NFT #{} '{}' minted with energy={}, half_life={}", id, req.name, req.energy, req.half_life),
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
            message: format!("NFT #{} transferred from {} to {}", req.nft_id, from, req.to),
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
                message: format!("NFT #{} '{}' resurrected with energy={}", nft.id, nft.name, req.energy),
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
                message: format!("NFT #{} '{}' refreshed, energy now {}", nft.id, nft.name, nft.energy),
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
        if self.total_supply == 0 { return 100.0; }
        let cur = self.current_supply(epoch);
        ((self.total_supply - cur) as f64 / self.total_supply as f64 * 1000.0).round() / 10.0
    }

    /// Apply proportional decay to all holder balances.
    pub fn tick_decay(&mut self, epoch: u64) {
        if epoch <= self.last_decay_epoch { return; }
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

async fn tokens_html() -> impl IntoResponse { Html(include_str!("../dashboard/tokens.html")) }

#[derive(Serialize)]
struct TokenResponse {
    id: u64, name: String, symbol: String, total_supply: u64, current_supply: u64,
    decay_half_life: u64, deployed_epoch: u64, deployer: String,
    decay_percentage: f64, holder_count: usize,
    holders: Vec<TokenHolder>,
}
#[derive(Serialize)]
struct TokenHolder { address: String, balance: u64 }

async fn get_tokens(State(state): State<Arc<ApiState>>) -> Json<Vec<TokenResponse>> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.token_store);
    for t in store.tokens.iter_mut() { t.tick_decay(epoch); }
    let res: Vec<TokenResponse> = store.tokens.iter().map(|t| {
        let mut holders: Vec<TokenHolder> = t.balances.iter()
            .filter(|(_, b)| **b > 0)
            .map(|(a, b)| TokenHolder { address: a.clone(), balance: *b })
            .collect();
        holders.sort_by_key(|a| std::cmp::Reverse(a.balance));
        TokenResponse {
            id: t.id, name: t.name.clone(), symbol: t.symbol.clone(),
            total_supply: t.total_supply, current_supply: t.current_supply(epoch),
            decay_half_life: t.decay_half_life, deployed_epoch: t.deployed_epoch,
            deployer: t.deployer.clone(), decay_percentage: t.decay_pct(epoch),
            holder_count: holders.len(), holders,
        }
    }).collect();
    Json(res)
}

async fn get_single_token(State(state): State<Arc<ApiState>>, Path(id): Path<u64>) -> Result<Json<TokenResponse>, StatusCode> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.token_store);
    let t = store.tokens.iter_mut().find(|t| t.id == id).ok_or(StatusCode::NOT_FOUND)?;
    t.tick_decay(epoch);
    let mut holders: Vec<TokenHolder> = t.balances.iter()
        .filter(|(_, b)| **b > 0)
        .map(|(a, b)| TokenHolder { address: a.clone(), balance: *b })
        .collect();
    holders.sort_by_key(|a| std::cmp::Reverse(a.balance));
    Ok(Json(TokenResponse {
        id: t.id, name: t.name.clone(), symbol: t.symbol.clone(),
        total_supply: t.total_supply, current_supply: t.current_supply(epoch),
        decay_half_life: t.decay_half_life, deployed_epoch: t.deployed_epoch,
        deployer: t.deployer.clone(), decay_percentage: t.decay_pct(epoch),
        holder_count: holders.len(), holders,
    }))
}

#[derive(Deserialize)]
struct DeployTokenRequest { name: String, symbol: String, total_supply: u64, decay_half_life: u64, deployer: Option<String> }

async fn post_deploy_token(State(state): State<Arc<ApiState>>, headers: HeaderMap, Json(req): Json<DeployTokenRequest>) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, false) { return resp; }
    let token_name = match sanitize_string(&req.name, 100) {
        Ok(n) => n, Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
    };
    let token_symbol = match sanitize_string(&req.symbol, 20) {
        Ok(s) => s, Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
    };
    if token_name.is_empty() { return Json(TxResultResponse { success: false, message: "Token name is required".into(), tx_hash: None }); }
    if token_symbol.is_empty() { return Json(TxResultResponse { success: false, message: "Token symbol is required".into(), tx_hash: None }); }
    if req.total_supply == 0 { return Json(TxResultResponse { success: false, message: "Total supply must be > 0".into(), tx_hash: None }); }
    if req.decay_half_life == 0 { return Json(TxResultResponse { success: false, message: "Decay half-life must be > 0".into(), tx_hash: None }); }
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let deployer = req.deployer.unwrap_or_else(|| format!("0x{}", GENESIS_FOUNDATION));
    let mut store = safe_lock(&state.token_store);
    let id = store.next_id; store.next_id += 1;
    let mut balances = HashMap::new();
    balances.insert(deployer.clone(), req.total_supply);
    store.tokens.push(DeployedToken {
        id, name: token_name.clone(), symbol: token_symbol.clone(),
        total_supply: req.total_supply, decay_half_life: req.decay_half_life,
        deployed_epoch: epoch, deployer, balances, last_decay_epoch: epoch,
    });
    let hash = tx_hash(&format!("token:deploy:{}:{}", token_symbol, req.total_supply));
    Json(TxResultResponse { success: true, message: format!("{} ({}) deployed with supply={}, half_life={}", token_name, token_symbol, req.total_supply, req.decay_half_life), tx_hash: Some(hash) })
}

#[derive(Deserialize)]
struct TokenTransferRequest { token_id: u64, from: String, to: String, amount: u64 }

async fn post_token_transfer(State(state): State<Arc<ApiState>>, headers: HeaderMap, Json(req): Json<TokenTransferRequest>) -> Json<TxResultResponse> {
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
        Some(t) => t, None => return Json(TxResultResponse { success: false, message: "Token not found".into(), tx_hash: None }),
    };
    let from_bal = t.balances.get(&req.from).copied().unwrap_or(0);
    if from_bal < req.amount {
        return Json(TxResultResponse { success: false, message: format!("Insufficient balance: {} < {}", from_bal, req.amount), tx_hash: None });
    }
    *t.balances.entry(req.from.clone()).or_insert(0) -= req.amount;
    *t.balances.entry(req.to.clone()).or_insert(0) += req.amount;
    let hash = tx_hash(&format!("token:transfer:{}:{}:{}:{}", req.token_id, req.from, req.to, req.amount));
    Json(TxResultResponse { success: true, message: format!("{} {} transferred from {} to {}", req.amount, t.symbol, req.from, req.to), tx_hash: Some(hash) })
}

#[derive(Deserialize)]
struct TokenBalanceRequest { token_id: u64, address: String }

async fn post_token_balance(State(state): State<Arc<ApiState>>, Json(req): Json<TokenBalanceRequest>) -> Json<serde_json::Value> {
    let store = safe_lock(&state.token_store);
    let t = match store.tokens.iter().find(|t| t.id == req.token_id) {
        Some(t) => t, None => return Json(serde_json::json!({"error":"Token not found"})),
    };
    let bal = t.balances.get(&req.address).copied().unwrap_or(0);
    Json(serde_json::json!({"token_id": req.token_id, "address": req.address, "balance": bal, "symbol": t.symbol}))
}

// ──────────────────────────── Staking Store ─────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
pub struct StakingPool {
    pub id: u64,
    pub name: String,
    pub reward_rate: u64,       // per epoch
    pub reward_decay_hl: u64,   // reward decay half-life
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
        if self.total_staked == 0 || staker.amount == 0 { return 0; }
        let epochs_since_claim = epoch.saturating_sub(staker.last_claim_epoch);
        if epochs_since_claim == 0 { return staker.pending_rewards; }
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

async fn staking_html() -> impl IntoResponse { Html(include_str!("../dashboard/staking.html")) }

#[derive(Serialize)]
struct StakingPoolResponse {
    id: u64, name: String, reward_rate: u64, reward_decay_hl: u64,
    total_staked: u64, created_epoch: u64, staker_count: usize,
    stakers: Vec<StakerResponse>,
}
#[derive(Serialize)]
struct StakerResponse {
    address: String, amount: u64, staked_epoch: u64,
    pending_rewards: u64, last_claim_epoch: u64,
    total_claimed: u64, total_decayed: u64,
    reward_decay_pct: f64,
}

async fn get_staking_pools(State(state): State<Arc<ApiState>>) -> Json<Vec<StakingPoolResponse>> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let store = safe_lock(&state.staking_store);
    let res: Vec<StakingPoolResponse> = store.pools.iter().map(|p| {
        let stakers: Vec<StakerResponse> = p.stakers.iter().map(|s| {
            let pending = p.compute_rewards(s, epoch);
            let raw_epochs = epoch.saturating_sub(s.last_claim_epoch);
            let share = if p.total_staked > 0 { s.amount as f64 / p.total_staked as f64 } else { 0.0 };
            let raw = (share * p.reward_rate as f64 * raw_epochs as f64) as u64 + s.pending_rewards;
            let decay_pct = if raw > 0 { ((raw - pending) as f64 / raw as f64 * 1000.0).round() / 10.0 } else { 0.0 };
            StakerResponse {
                address: s.address.clone(), amount: s.amount, staked_epoch: s.staked_epoch,
                pending_rewards: pending, last_claim_epoch: s.last_claim_epoch,
                total_claimed: s.total_claimed, total_decayed: s.total_decayed,
                reward_decay_pct: decay_pct,
            }
        }).collect();
        StakingPoolResponse {
            id: p.id, name: p.name.clone(), reward_rate: p.reward_rate,
            reward_decay_hl: p.reward_decay_hl, total_staked: p.total_staked,
            created_epoch: p.created_epoch, staker_count: stakers.len(), stakers,
        }
    }).collect();
    Json(res)
}

async fn get_single_pool(State(state): State<Arc<ApiState>>, Path(id): Path<u64>) -> Result<Json<StakingPoolResponse>, StatusCode> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let store = safe_lock(&state.staking_store);
    let p = store.pools.iter().find(|p| p.id == id).ok_or(StatusCode::NOT_FOUND)?;
    let stakers: Vec<StakerResponse> = p.stakers.iter().map(|s| {
        let pending = p.compute_rewards(s, epoch);
        let raw_epochs = epoch.saturating_sub(s.last_claim_epoch);
        let share = if p.total_staked > 0 { s.amount as f64 / p.total_staked as f64 } else { 0.0 };
        let raw = (share * p.reward_rate as f64 * raw_epochs as f64) as u64 + s.pending_rewards;
        let decay_pct = if raw > 0 { ((raw - pending) as f64 / raw as f64 * 1000.0).round() / 10.0 } else { 0.0 };
        StakerResponse { address: s.address.clone(), amount: s.amount, staked_epoch: s.staked_epoch, pending_rewards: pending, last_claim_epoch: s.last_claim_epoch, total_claimed: s.total_claimed, total_decayed: s.total_decayed, reward_decay_pct: decay_pct }
    }).collect();
    Ok(Json(StakingPoolResponse { id: p.id, name: p.name.clone(), reward_rate: p.reward_rate, reward_decay_hl: p.reward_decay_hl, total_staked: p.total_staked, created_epoch: p.created_epoch, staker_count: stakers.len(), stakers }))
}

#[derive(Deserialize)]
struct StakeRequest { pool_id: u64, address: String, amount: u64 }

async fn post_stake(State(state): State<Arc<ApiState>>, headers: HeaderMap, Json(req): Json<StakeRequest>) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, false) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_wallet_ownership(&state, user_id, &req.address) {
        return resp;
    }
    if req.amount == 0 {
        return Json(TxResultResponse { success: false, message: "Amount must be greater than zero".into(), tx_hash: None });
    }
    // Balance pre-check
    {
        let addr = parse_hex_address(&req.address);
        if let Ok(addr_bytes) = addr {
            let db = safe_lock(&state.db);
            if let Some(acct) = db.get_account(&addr_bytes) {
                if acct.balance < req.amount {
                    return Json(TxResultResponse { success: false, message: format!("Insufficient balance: {} < {}", acct.balance, req.amount), tx_hash: None });
                }
            } else {
                return Json(TxResultResponse { success: false, message: "Account not found — use faucet first".into(), tx_hash: None });
            }
        }
    }
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.staking_store);
    let p = match store.pools.iter_mut().find(|p| p.id == req.pool_id) {
        Some(p) => p, None => return Json(TxResultResponse { success: false, message: "Pool not found".into(), tx_hash: None }),
    };
    if let Some(s) = p.stakers.iter_mut().find(|s| s.address == req.address) {
        s.amount += req.amount;
    } else {
        p.stakers.push(Staker { address: req.address.clone(), amount: req.amount, staked_epoch: epoch, pending_rewards: 0, last_claim_epoch: epoch, total_claimed: 0, total_decayed: 0 });
    }
    p.total_staked += req.amount;
    let hash = tx_hash(&format!("stake:{}:{}:{}", req.pool_id, req.address, req.amount));
    Json(TxResultResponse { success: true, message: format!("Staked {} in {}", req.amount, p.name), tx_hash: Some(hash) })
}

#[derive(Deserialize)]
struct UnstakeRequest { pool_id: u64, address: String, amount: u64 }

async fn post_unstake(State(state): State<Arc<ApiState>>, headers: HeaderMap, Json(req): Json<UnstakeRequest>) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, false) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_wallet_ownership(&state, user_id, &req.address) {
        return resp;
    }
    let mut store = safe_lock(&state.staking_store);
    let p = match store.pools.iter_mut().find(|p| p.id == req.pool_id) {
        Some(p) => p, None => return Json(TxResultResponse { success: false, message: "Pool not found".into(), tx_hash: None }),
    };
    let s = match p.stakers.iter_mut().find(|s| s.address == req.address) {
        Some(s) => s, None => return Json(TxResultResponse { success: false, message: "Not staked".into(), tx_hash: None }),
    };
    if s.amount < req.amount {
        return Json(TxResultResponse { success: false, message: format!("Insufficient stake: {} < {}", s.amount, req.amount), tx_hash: None });
    }
    s.amount -= req.amount;
    p.total_staked -= req.amount;
    let hash = tx_hash(&format!("unstake:{}:{}:{}", req.pool_id, req.address, req.amount));
    Json(TxResultResponse { success: true, message: format!("Unstaked {} from {}", req.amount, p.name), tx_hash: Some(hash) })
}

#[derive(Deserialize)]
struct ClaimRequest { pool_id: u64, address: String }

async fn post_claim(State(state): State<Arc<ApiState>>, headers: HeaderMap, Json(req): Json<ClaimRequest>) -> Json<TxResultResponse> {
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
        Some(p) => p, None => return Json(TxResultResponse { success: false, message: "Pool not found".into(), tx_hash: None }),
    };
    let reward_decay_hl = p.reward_decay_hl;
    let reward_rate = p.reward_rate;
    let total_staked = p.total_staked;
    let s = match p.stakers.iter_mut().find(|s| s.address == req.address) {
        Some(s) => s, None => return Json(TxResultResponse { success: false, message: "Not staked".into(), tx_hash: None }),
    };
    let epochs_since = epoch.saturating_sub(s.last_claim_epoch);
    let share = if total_staked > 0 { s.amount as f64 / total_staked as f64 } else { 0.0 };
    let raw = (share * reward_rate as f64 * epochs_since as f64) as u64 + s.pending_rewards;
    let actual = evaporchain_types::energy_at_epoch(raw, reward_decay_hl, epochs_since);
    let decayed = raw.saturating_sub(actual);
    s.total_claimed += actual;
    s.total_decayed += decayed;
    s.pending_rewards = 0;
    s.last_claim_epoch = epoch;
    let hash = tx_hash(&format!("claim:{}:{}:{}", req.pool_id, req.address, actual));
    Json(TxResultResponse { success: true, message: format!("Claimed {} rewards ({} decayed)", actual, decayed), tx_hash: Some(hash) })
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
        if self.status != "Active" { return; }
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

async fn dao_html() -> impl IntoResponse { Html(include_str!("../dashboard/dao.html")) }

#[derive(Serialize)]
struct ProposalResponse {
    id: u64, title: String, description: String, options: Vec<String>,
    created_epoch: u64, voting_period: u64, end_epoch: u64,
    creator: String, status: String, total_votes: u64,
    vote_totals: HashMap<String, u64>, epochs_remaining: u64,
    evaporated_epoch: Option<u64>, voter_count: usize,
}

async fn get_proposals(State(state): State<Arc<ApiState>>) -> Json<Vec<ProposalResponse>> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.dao_store);
    for p in store.proposals.iter_mut() { p.tick(epoch); }
    let res: Vec<ProposalResponse> = store.proposals.iter().map(|p| {
        let remaining = if epoch < p.end_epoch() { p.end_epoch() - epoch } else { 0 };
        ProposalResponse {
            id: p.id, title: p.title.clone(), description: p.description.clone(),
            options: p.options.clone(), created_epoch: p.created_epoch,
            voting_period: p.voting_period, end_epoch: p.end_epoch(),
            creator: p.creator.clone(), status: p.status.clone(),
            total_votes: p.total_votes(), vote_totals: p.vote_totals(),
            epochs_remaining: remaining, evaporated_epoch: p.evaporated_epoch,
            voter_count: p.votes.iter().map(|v| &v.voter).collect::<std::collections::HashSet<_>>().len(),
        }
    }).collect();
    Json(res)
}

async fn get_single_proposal(State(state): State<Arc<ApiState>>, Path(id): Path<u64>) -> Result<Json<ProposalResponse>, StatusCode> {
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.dao_store);
    let p = store.proposals.iter_mut().find(|p| p.id == id).ok_or(StatusCode::NOT_FOUND)?;
    p.tick(epoch);
    let remaining = if epoch < p.end_epoch() { p.end_epoch() - epoch } else { 0 };
    Ok(Json(ProposalResponse {
        id: p.id, title: p.title.clone(), description: p.description.clone(),
        options: p.options.clone(), created_epoch: p.created_epoch,
        voting_period: p.voting_period, end_epoch: p.end_epoch(),
        creator: p.creator.clone(), status: p.status.clone(),
        total_votes: p.total_votes(), vote_totals: p.vote_totals(),
        epochs_remaining: remaining, evaporated_epoch: p.evaporated_epoch,
        voter_count: p.votes.iter().map(|v| &v.voter).collect::<std::collections::HashSet<_>>().len(),
    }))
}

#[derive(Deserialize)]
struct ProposeRequest { title: String, description: String, options: Vec<String>, voting_period: u64, creator: Option<String> }

async fn post_propose(State(state): State<Arc<ApiState>>, headers: HeaderMap, Json(req): Json<ProposeRequest>) -> Json<TxResultResponse> {
    if let Err(resp) = require_tx_auth(&headers, &state, false) { return resp; }
    let title = match sanitize_string(&req.title, 200) {
        Ok(t) => t, Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
    };
    if title.is_empty() {
        return Json(TxResultResponse { success: false, message: "Title is required".into(), tx_hash: None });
    }
    let description = match sanitize_string(&req.description, 2000) {
        Ok(d) => d, Err(e) => return Json(TxResultResponse { success: false, message: e, tx_hash: None }),
    };
    let options: Vec<String> = req.options.iter().map(|o| strip_html_tags(o)).collect();
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let creator = req.creator.unwrap_or_else(|| format!("0x{}", GENESIS_FOUNDATION));
    let mut store = safe_lock(&state.dao_store);
    let id = store.next_id; store.next_id += 1;
    store.proposals.push(DAOProposal {
        id, title: title.clone(), description,
        options, votes: vec![], created_epoch: epoch,
        voting_period: req.voting_period, creator, status: "Active".into(),
        evaporated_epoch: None,
    });
    let hash = tx_hash(&format!("dao:propose:{}:{}", id, title));
    Json(TxResultResponse { success: true, message: format!("Proposal #{} '{}' created, voting for {} epochs", id, title, req.voting_period), tx_hash: Some(hash) })
}

#[derive(Deserialize)]
struct VoteRequest { proposal_id: u64, option: String, weight: u64, voter: Option<String> }

async fn post_vote(State(state): State<Arc<ApiState>>, headers: HeaderMap, Json(req): Json<VoteRequest>) -> Json<TxResultResponse> {
    let user_id = match require_tx_auth(&headers, &state, false) {
        Ok(uid) => uid,
        Err(resp) => return resp,
    };
    let voter = match req.voter {
        Some(ref v) if !v.is_empty() => v.clone(),
        _ => return Json(TxResultResponse { success: false, message: "Voter address is required".into(), tx_hash: None }),
    };
    // Ownership check: caller must own the voter address
    if let Err(resp) = require_wallet_ownership(&state, user_id, &voter) {
        return resp;
    }
    // Validate vote weight: must be > 0 and <= staked amount
    if req.weight == 0 {
        return Json(TxResultResponse { success: false, message: "Vote weight must be greater than zero".into(), tx_hash: None });
    }
    {
        let staking = safe_lock(&state.staking_store);
        let total_staked: u64 = staking.pools.iter().flat_map(|p| p.stakers.iter())
            .filter(|s| s.address == voter)
            .map(|s| s.amount)
            .sum();
        if req.weight > total_staked {
            return Json(TxResultResponse { success: false, message: format!("Vote weight {} exceeds your total stake {}", req.weight, total_staked), tx_hash: None });
        }
    }
    let history = safe_lock(&state.block_history);
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = safe_lock(&state.dao_store);
    let p = match store.proposals.iter_mut().find(|p| p.id == req.proposal_id) {
        Some(p) => p, None => return Json(TxResultResponse { success: false, message: "Proposal not found".into(), tx_hash: None }),
    };
    if p.status != "Active" {
        return Json(TxResultResponse { success: false, message: format!("Proposal is {}, cannot vote", p.status), tx_hash: None });
    }
    if !p.options.contains(&req.option) {
        return Json(TxResultResponse { success: false, message: format!("Invalid option: {}", req.option), tx_hash: None });
    }
    if p.votes.iter().any(|v| v.voter == voter) {
        return Json(TxResultResponse { success: false, message: "Already voted".into(), tx_hash: None });
    }
    p.votes.push(DAOVote { voter: voter.clone(), option: req.option.clone(), weight: req.weight, epoch });
    let hash = tx_hash(&format!("dao:vote:{}:{}:{}", req.proposal_id, voter, req.option));
    Json(TxResultResponse { success: true, message: format!("Voted '{}' with weight {} on proposal #{}", req.option, req.weight, req.proposal_id), tx_hash: Some(hash) })
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
async fn get_latest_block(State(state): State<Arc<ApiState>>) -> Result<Json<BlockRecord>, StatusCode> {
    let history = safe_lock(&state.block_history);
    history.back().cloned().ok_or(StatusCode::NOT_FOUND).map(Json)
}

/// Mempool endpoint with transaction details.
async fn get_mempool(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let txs = state.mempool_transactions();
    Json(serde_json::json!({
        "pending": txs.len(),
        "transactions": txs,
    }))
}

/// Transaction receipt lookup by hash.
async fn get_tx_receipt(
    State(state): State<Arc<ApiState>>,
    Path(hash): Path<String>,
) -> Result<Json<crate::persistence::TxReceipt>, StatusCode> {
    let store = state.chain_store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store.get_tx_receipt(&hash).map(Json).ok_or(StatusCode::NOT_FOUND)
}

/// Address transaction history.
async fn get_address_txs(
    State(state): State<Arc<ApiState>>,
    Path(addr): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = state.chain_store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
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
    let store = state.chain_store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
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
    let store = state.chain_store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
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
    let store = state.chain_store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
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
    let store = state.chain_store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
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
        collections.entry(nft.collection.clone()).or_default().push(nft.id);
    }
    let result: Vec<serde_json::Value> = collections.iter().map(|(name, ids)| {
        serde_json::json!({ "name": name, "count": ids.len(), "nft_ids": ids })
    }).collect();
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

/// GET /metrics — Prometheus text exposition format for scraping.
async fn get_prometheus_metrics(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_admin_auth(&headers) {
        return e.into_response();
    }
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
    out.push_str(&format!("evaporchain_total_transactions {}\n", stats.total_transactions));
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
    out.push_str(&format!("evaporchain_avg_block_exec_ms {:.2}\n", t.avg_exec_time_us() as f64 / 1000.0));
    out.push_str("# HELP evaporchain_avg_gas_per_block Average gas used per block\n");
    out.push_str("# TYPE evaporchain_avg_gas_per_block gauge\n");
    out.push_str(&format!("evaporchain_avg_gas_per_block {}\n", t.avg_gas_per_block()));
    out.push_str("# HELP evaporchain_uptime_seconds Node uptime in seconds\n");
    out.push_str("# TYPE evaporchain_uptime_seconds counter\n");
    out.push_str(&format!("evaporchain_uptime_seconds {}\n", uptime));

    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        out,
    ).into_response()
}

/// GET /api/proof/latest — generate and return the latest chain proof.
async fn get_proof_latest(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ChainProofResponse>, StatusCode> {
    let p = safe_lock(&state.chain_prover);
    let chain_proof = p.generate_chain_proof().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

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
async fn get_proof_status(
    State(state): State<Arc<ApiState>>,
) -> Json<ProverStatusResponse> {
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
    let genesis = hex::decode(&q.genesis_state_root)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if genesis.len() != 32 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let proof_bytes = hex::decode(&q.proof_hex)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

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
    let store = state.chain_store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let block = store.load_full_block(block_number).ok_or(StatusCode::NOT_FOUND)?;
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
    let store = state.chain_store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
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
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"})))
}

/// Security headers middleware.
fn security_headers(response: &mut axum::http::Response<axum::body::Body>) {
    let h = response.headers_mut();
    // Only set headers not already handled by nginx reverse proxy
    h.insert("Permissions-Policy", "camera=(), microphone=(), geolocation=()".parse().unwrap());
}

// ─────────────── Frontier Primitives ─────────────────────────────────────

async fn get_frontier_status(
    State(state): State<Arc<ApiState>>,
) -> impl IntoResponse {
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

        let epoch = params.epoch.unwrap_or_else(|| {
            fs.lazy_cache.latest_anchor_epoch().unwrap_or(0)
        });

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
        let epoch = params.epoch.unwrap_or_else(|| {
            fs.lazy_cache.latest_anchor_epoch().unwrap_or(0)
        });

        let results = fs.query_all_lazy(epoch);
        let items: Vec<_> = results.iter().map(|r| {
            serde_json::json!({
                "object_id": hex::encode(r.object_id),
                "energy": r.energy,
                "state": format!("{:?}", r.state),
                "half_life": r.half_life,
            })
        }).collect();

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

async fn get_da_status(
    State(state): State<Arc<ApiState>>,
) -> impl IntoResponse {
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
        ).into_response();
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
        })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{}", e)})),
        ).into_response(),
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
        ).into_response();
    };

    // Generate 4 random sample queries using block hash as seed
    let seed = blake3::hash(&block.to_le_bytes());
    let queries = evaporchain_da::block_da::BlockDA::generate_sample_queries(
        block, &package.header, 4, seed.as_bytes(),
    );

    let da = evaporchain_da::block_da::BlockDA::new().unwrap();
    let mut samples = Vec::new();
    let mut all_valid = true;

    for query in &queries {
        if let Ok(response) = da.prove_shard(package, query.shard_index) {
            let valid = evaporchain_da::block_da::BlockDA::verify_shard_sample(
                &package.header, &response,
            );
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
    })).into_response()
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
        ).into_response();
    };

    if row >= package.header.row_roots.len() || col >= package.header.col_roots.len() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("cell ({},{}) out of range ({}x{})", row, col,
                    package.header.row_roots.len(), package.header.col_roots.len())
            })),
        ).into_response();
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
        })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{}", e)})),
        ).into_response(),
    }
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
        ).into_response();
    };

    let da2d = evaporchain_da::block_da_2d::BlockDA2D::new();
    let seed = blake3::hash(&block.to_le_bytes());
    let num_samples = std::cmp::min(8, package.header.extended_dim * package.header.extended_dim);
    let queries = evaporchain_da::commitments::generate_2d_queries(
        block, package.header.extended_dim, num_samples, seed.as_bytes(),
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
    })).into_response()
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
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "invalid object_id hex (need 32 bytes)"}))).into_response();
        }
    };

    let db = state.db.lock().unwrap();
    let Some(ghost) = db.get_ghost(&object_id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "object not evaporated (no ghost record)"}))).into_response();
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
            let shard_index = (u64::from_le_bytes(object_id[..8].try_into().unwrap_or([0u8; 8])) as usize) % package.shards.len();
            let snapshot = evaporchain_da::evaporation_da::EnergySnapshot {
                object_id,
                energy_at_evaporation: 0,
                evaporation_epoch: evap_epoch,
                half_life: ghost.original_half_life.unwrap_or(10),
                last_refreshed: 0,
                energy_at_refresh: 0,
            };
            if let Ok(proof) = evaporchain_da::evaporation_da::EvaporationDAProofBuilder::create_proof(
                object_id,
                ghost.original_data.as_deref().unwrap_or(&ghost.data_hash),
                snapshot,
                &package.shards,
                shard_index,
            ) {
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
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error": "frontier not enabled"}))).into_response();
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
        })).into_response()
    } else if let Some(ghost) = fs.poha.get_ghost(block_number) {
        Json(serde_json::json!({
            "block_number": ghost.block_number,
            "data_root": hex::encode(ghost.data_root),
            "cert_hash": hex::encode(ghost.cert_hash),
            "evaporated_epoch": ghost.evaporated_epoch,
            "total_re_attestations": ghost.total_re_attestations,
            "temperature": "Evaporated",
            "is_ghost": true,
        })).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "no PoHA certificate for this block"}))).into_response()
    }
}

async fn get_poha_certificates(
    State(state): State<Arc<ApiState>>,
) -> impl IntoResponse {
    let Some(ref fs_arc) = state.frontier_state else {
        return Json(serde_json::json!({"error": "frontier not enabled"}));
    };
    let fs = fs_arc.lock().unwrap();
    let dist = fs.poha.temperature_distribution();

    let certs: Vec<_> = fs.poha.all_active().map(|(&bn, cert)| {
        serde_json::json!({
            "block_number": bn,
            "energy": cert.energy,
            "initial_energy": cert.initial_energy,
            "temperature": format!("{:?}", cert.temperature()),
            "re_attestations": cert.re_attestation_count,
        })
    }).collect();

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
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "success": false, "message": "invalid commitment hex (need 64 chars)"
        }))),
    };
    let nonce_hash = match hex_to_32(&body.nonce_hash) {
        Some(n) => n,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "success": false, "message": "invalid nonce_hash hex (need 64 chars)"
        }))),
    };
    let encrypted_payload = match hex::decode(&body.encrypted_payload) {
        Ok(p) => p,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "success": false, "message": "invalid encrypted_payload hex"
        }))),
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

    (StatusCode::OK, Json(serde_json::json!({
        "success": true,
        "message": "encrypted transaction submitted",
        "commitment": body.commitment,
        "reveal_epoch": current_epoch + state.encrypted_mempool.lock().unwrap().reveal_delay(),
    })))
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
        None => return Json(serde_json::json!({
            "success": false, "message": "invalid commitment hex"
        })),
    };
    let nonce = match hex_to_32(&body.nonce) {
        Some(n) => n,
        None => return Json(serde_json::json!({
            "success": false, "message": "invalid nonce hex"
        })),
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

async fn get_encrypted_mempool_status(
    State(state): State<Arc<ApiState>>,
) -> impl IntoResponse {
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
    if bytes.len() != 32 { return None; }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Some(arr)
}

// ─────────────────── Light Client ────────────────────────────────────────

async fn get_light_client_status(
    State(state): State<Arc<ApiState>>,
) -> impl IntoResponse {
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
        None => return Json(serde_json::json!({ "verified": false, "error": "invalid block_hash" })),
    };
    let parent_hash = match hex_to_32(&body.parent_hash) {
        Some(h) => h,
        None => return Json(serde_json::json!({ "verified": false, "error": "invalid parent_hash" })),
    };
    let state_root = match hex_to_32(&body.state_root) {
        Some(h) => h,
        None => return Json(serde_json::json!({ "verified": false, "error": "invalid state_root" })),
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

async fn get_weak_subjectivity_checkpoint(
    State(state): State<Arc<ApiState>>,
) -> impl IntoResponse {
    if let Some(ref tc_arc) = state.tendermint {
        let tc = tc_arc.lock().unwrap();
        let ws_period = tc.weak_subjectivity_period();
        let trusted = tc.trusted_checkpoint();
        let latest = tc.latest_checkpoint();
        let all_checkpoints: Vec<_> = tc.checkpoints().iter().map(|(h, r)| {
            serde_json::json!({"height": h, "state_root": hex::encode(r)})
        }).collect();

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
        })).into_response()
    } else {
        Json(serde_json::json!({"error": "consensus not in Tendermint mode"})).into_response()
    }
}

async fn get_finality(
    State(state): State<Arc<ApiState>>,
) -> impl IntoResponse {
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

async fn get_sync_snapshot_info(
    State(state): State<Arc<ApiState>>,
) -> impl IntoResponse {
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

pub fn create_router(state: Arc<ApiState>, auth_state: Arc<crate::auth::AuthState>) -> Router {
    let allowed_origins = [
        "https://evaporchain.com".parse().unwrap(),
        "https://testnet.evaporchain.com".parse().unwrap(),
        "http://localhost:3000".parse().unwrap(),
    ];
    let cors = CorsLayer::new()
        .allow_origin(allowed_origins.to_vec())
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST, axum::http::Method::OPTIONS])
        .allow_headers([axum::http::header::CONTENT_TYPE, axum::http::header::AUTHORIZATION]);

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

    Router::new()
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
        .route("/api/hbct/mint", post(post_hbct_mint))
        .route("/api/hbct/transfer", post(post_hbct_transfer))
        .route("/api/hbct/burn", post(post_hbct_burn))
        .route("/api/hbct/balance", post(post_hbct_balance))
        .route("/api/hbct/tick", post(post_hbct_tick))
        .route("/api/hbct/seed_attestation", post(post_hbct_seed_attestation))
        .route("/api/hbct/settle", post(post_hbct_settle))
        .route("/api/sentinel/register", post(post_sentinel_register_param))
        .route("/api/sentinel/vote", post(post_sentinel_vote))
        .route("/api/sentinel/tick", post(post_sentinel_tick))
        .route("/api/sentinel/parameter/:id", get(get_sentinel_param))
        .route("/api/sentinel/all", get(get_sentinel_all))
        .route("/api/boltzmann_stake/:validator_id/at/:current_epoch", get(get_boltzmann_stake))
        .route("/api/lamport_time", get(get_lamport_time))
        .route("/api/light_cone", get(get_light_cone))
        .route("/api/causal_cone", get(get_causal_cone))
        .route("/api/objects", get(get_objects))
        .route("/api/object/:id", get(get_single_object))
        .route("/api/accounts", get(get_accounts))
        .route("/api/blocks", get(get_blocks))
        .route("/api/blocks/latest", get(get_latest_block))
        .route("/api/block/latest", get(get_latest_block))
        .route("/api/block/:number", get(get_single_block))
        .route("/api/tx/:hash", get(get_tx_by_hash))
        .route("/api/transactions", get(get_transactions))
        .route("/block/:number", get(block_detail_html))
        .route("/tx/:hash", get(tx_detail_html))
        .route("/api/mempool", get(get_mempool))
        .route("/api/events", get(get_events))
        // Stats
        .route("/api/stats", get(get_stats_summary))
        .route("/api/stats/timeline", get(get_stats_timeline))
        .route("/api/stats/summary", get(get_stats_summary))
        // Network
        .route("/api/network", get(get_network))
        // Wallet / Transactions
        .route("/api/tx/transfer", post(post_transfer))
        .route("/api/tx/create-object", post(post_create_object))
        .route("/api/tx/refresh", post(post_refresh))
        .route("/api/tx/resurrect", post(post_resurrect))
        .route("/api/tx/batch", post(post_batch))
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
        // Nova Proofs / Light Client
        .route("/api/proof/latest", get(get_proof_latest))
        .route("/api/proof/status", get(get_proof_status))
        .route("/api/proof/verify", get(get_proof_verify))
        // Data Availability sampling
        .route("/api/light/state-proof/account/:addr", get(get_account_state_proof))
        .route("/api/light/state-proof/object/:id", get(get_object_state_proof))
        .route("/api/light/tx-proof/:block/:tx_index", get(get_tx_inclusion_proof))
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
        .route("/api/da/2d-light-sample/:block", get(get_da_2d_light_sample))
        .route("/api/da/evaporation-proof/:object_id", get(get_evaporation_da_proof))
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
        .route("/api/weak-subjectivity", get(get_weak_subjectivity_checkpoint))
        .route("/api/finality", get(get_finality))
        .route("/api/finality/proof/:height", get(get_finality_proof))
        // State sync
        .route("/api/sync/snapshot-info", get(get_sync_snapshot_info))
        // PWA
        .route("/manifest.json", get(manifest_json))
        .route("/sw.js", get(service_worker_js))
        // WebSocket subscriptions
        .route("/ws", get(ws_upgrade_handler))
        // JSON-RPC 2.0 endpoint
        .route("/rpc", post(crate::jsonrpc::handle_jsonrpc))
        .with_state(state)
        // Merge auth routes (different state type)
        .merge(auth_router)
        .fallback(fallback_404)
        .layer(cors)
        .layer(axum::middleware::map_response(|mut resp: axum::http::Response<axum::body::Body>| async move {
            security_headers(&mut resp);
            resp
        }))
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
    if !limiter.check(addr.ip()) {
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
pub async fn start_api_server(state: Arc<ApiState>, auth_state: Arc<crate::auth::AuthState>, port: u16) -> anyhow::Result<()> {
    let limiter = Arc::new(RateLimiter::new(200, 10));
    let app = create_router(state, auth_state)
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(axum::Extension(limiter))
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    let is_localhost = port != 443 && std::env::var("EVAPORCHAIN_TLS_CERT").is_err();
    if is_localhost {
        eprintln!(
            "\x1b[33m⚠ Dashboard serving over HTTP (plaintext). \
             For production, use a TLS-terminating reverse proxy \
             or set EVAPORCHAIN_TLS_CERT + EVAPORCHAIN_TLS_KEY.\x1b[0m"
        );
    }
    println!(
        "\x1b[1;36m━━━ Dashboard: http://localhost:{} ━━━\x1b[0m",
        port
    );
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
            100_000 + 20_000 * ptx.input_nullifiers.len() as u64
                + 15_000 * ptx.output_commitments.len() as u64
        }
        Transaction::Deferred(dtx) => {
            75_000 + 5_000 * dtx.guards.len() as u64
        }
        Transaction::Blob(tx) => {
            50_000 + 10 * tx.data.len() as u64
        }
        Transaction::Governance(_) => 25_000,
        Transaction::MultiSig(_) => 50_000,
        Transaction::UserOp(tx) => 30_000 + tx.call_data.len() as u64 * 16,
        Transaction::UpgradeContract(tx) => 100_000 + tx.new_bytecode.len() as u64 * 200,
        Transaction::Delegate(_) => 40_000,
        Transaction::Undelegate(_) => 40_000,
        Transaction::RotateValidatorKey(_) => 80_000,
        Transaction::ClaimDelegation(_) => 30_000,
    }
}

pub fn tx_records_from_block(block: &Block) -> Vec<TxRecord> {
    block
        .transactions
        .iter()
        .map(|tx| {
            let hash = hex::encode(blake3::hash(&tx.signable_bytes()).as_bytes());
            let gas = estimate_tx_gas(tx);
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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
                    status: "success".to_string(),
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

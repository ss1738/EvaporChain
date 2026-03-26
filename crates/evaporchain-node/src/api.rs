use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use axum::http::HeaderMap;
use evaporchain_consensus::MockConsensus;
use evaporchain_crypto::signatures::{MlDsaKeypair, Signer};
use evaporchain_state::db::StateDB;
use evaporchain_state::RocksDBStateDB;
use evaporchain_types::{
    Block, CallContractTx, CreateObjectTx, DeployContractTx, ObjectState, RefreshTx, Transaction,
    TransferTx,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{atomic::AtomicUsize, Arc, Mutex};
use std::time::Instant;
use tower_http::cors::CorsLayer;

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
        let t = (self.half_life as f64 * (current as f64).log2()).ceil() as u64;
        t
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
    pub transactions: Vec<TxRecord>,
}

/// Minimal transaction record for block history.
#[derive(Clone, Serialize, Deserialize)]
pub struct TxRecord {
    #[serde(rename = "type")]
    pub tx_type: String,
    pub detail: String,
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

pub fn genesis_addr_display(hex: &str) -> String {
    format!("0x{}", hex)
}

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

fn obj_id_from_byte(b: u8) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[0] = b;
    id
}

/// Display address as truncated hex (first 4 bytes + last 3 bytes of 20-byte portion).
fn account_name(addr: &[u8; 32]) -> String {
    let full = hex::encode(&addr[..20]);
    if full.trim_start_matches('0').is_empty() {
        return "0x0000...0000".to_string();
    }
    format!("0x{}...{}", &full[..8], &full[34..])
}

/// Full 20-byte address as 0x-prefixed hex.
fn account_full(addr: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(&addr[..20]))
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

fn hex_short(bytes: &[u8]) -> String {
    let full = hex::encode(bytes);
    if full.len() > 16 {
        format!("{}...{}", &full[..10], &full[full.len()-6..])
    } else {
        full
    }
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
}

#[derive(Serialize)]
struct AccountResponse {
    address: String,
    name: String,
    balance: u64,
    nonce: u64,
}

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
    if let Some(ref sessions) = state.auth_sessions {
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
/// First tries the wallet's own ML-DSA keypair from the user DB.
/// Falls back to the node-level keypair if wallet keys are unavailable or legacy (too short).
fn sign_transaction(tx: &mut Transaction, state: &ApiState, sender_address: Option<&str>) {
    // Already signed by client — skip
    if tx.signature().is_some() {
        return;
    }

    // Try wallet-specific keys first
    if let (Some(ref user_db), Some(addr)) = (&state.user_db, sender_address) {
        if let Ok(Some((pk_hex, sk_hex))) = user_db.get_wallet_keys(addr) {
            // Real ML-DSA public keys are 1952 bytes (3904 hex chars)
            if pk_hex.len() > 1000 {
                if let (Ok(pk_bytes), Ok(sk_bytes)) = (hex::decode(&pk_hex), hex::decode(&sk_hex)) {
                    if let Ok(kp) = MlDsaKeypair::from_bytes(&pk_bytes, &sk_bytes) {
                        let msg = tx.signable_bytes();
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
    let msg = tx.signable_bytes();
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
    let db = state.db.lock().unwrap();
    let history = state.block_history.lock().unwrap();
    let stats = state.stats.lock().unwrap();
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

async fn get_objects(State(state): State<Arc<ApiState>>) -> Json<Vec<ObjectResponse>> {
    let db = state.db.lock().unwrap();
    let history = state.block_history.lock().unwrap();
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

    let db = state.db.lock().unwrap();
    let history = state.block_history.lock().unwrap();
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
    }))
}

async fn get_accounts(State(state): State<Arc<ApiState>>) -> Json<Vec<AccountResponse>> {
    let db = state.db.lock().unwrap();
    let addrs = db.all_account_addresses();
    let mut accounts: Vec<AccountResponse> = addrs
        .iter()
        .filter_map(|addr| {
            let acc = db.get_account(addr)?;
            Some(AccountResponse {
                address: hex::encode(addr),
                name: account_name(addr),
                balance: acc.balance,
                nonce: acc.nonce,
            })
        })
        .collect();
    accounts.sort_by(|a, b| b.balance.cmp(&a.balance));
    Json(accounts)
}

async fn get_ghosts(State(state): State<Arc<ApiState>>) -> Json<Vec<GhostResponse>> {
    let db = state.db.lock().unwrap();
    let ghost_ids = db.all_ghost_ids();
    let ghosts: Vec<GhostResponse> = ghost_ids
        .iter()
        .filter_map(|id| {
            let g = db.get_ghost(id)?;
            Some(GhostResponse {
                id: hex::encode(g.object_id),
                original_owner: hex::encode(g.owner),
                evaporated_epoch: g.evaporated_at,
                data_hash: hex::encode(g.data_hash),
            })
        })
        .collect();
    Json(ghosts)
}

async fn get_blocks(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<BlocksQuery>,
) -> Json<Vec<BlockRecord>> {
    let history = state.block_history.lock().unwrap();
    let limit = params.limit.unwrap_or(50).min(500);
    let blocks: Vec<BlockRecord> = history.iter().rev().take(limit).cloned().collect();
    Json(blocks)
}

async fn get_single_block(
    State(state): State<Arc<ApiState>>,
    Path(number): Path<u64>,
) -> Result<Json<BlockRecord>, StatusCode> {
    let history = state.block_history.lock().unwrap();
    let block = history
        .iter()
        .find(|b| b.number == number)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(block))
}

async fn get_stats_timeline(State(state): State<Arc<ApiState>>) -> Json<StatsTimelineResponse> {
    let stats = state.stats.lock().unwrap();
    Json(StatsTimelineResponse {
        epochs: stats.state_size_trend.clone(),
    })
}

async fn get_stats_summary(State(state): State<Arc<ApiState>>) -> Json<StatsSummaryResponse> {
    let stats = state.stats.lock().unwrap();
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
    let events = state.events.lock().unwrap();
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
        let db = state.db.lock().unwrap();
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
    // Atomic dedup check + submit (single lock to prevent TOCTOU race)
    {
        let mut c = state.consensus.lock().unwrap();
        let is_dup = c.mempool.pending().iter().any(|tx| {
            if let Transaction::Transfer(t) = tx {
                t.from == from && t.to == to && t.amount == req.amount && t.nonce == req.nonce
            } else {
                false
            }
        });
        if is_dup {
            return Json(TxResultResponse { success: false, message: "Duplicate transaction already in mempool".into(), tx_hash: None });
        }
        let mut tx = Transaction::Transfer(TransferTx {
            from, to, amount: req.amount, nonce: req.nonce,
            signature: req.signature.and_then(|s| hex::decode(s).ok()),
            public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
        });
        let sender_addr = format!("0x{}", hex::encode(from));
        sign_transaction(&mut tx, &state, Some(&sender_addr));
        c.mempool.submit(tx);
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
    let mut tx = Transaction::CreateObject(CreateObjectTx {
        creator, object_id: obj_id_val, energy: req.energy, half_life: req.half_life,
        data: format!("obj-0x{}", &obj_label).into_bytes(),
        signature: req.signature.and_then(|s| hex::decode(s).ok()),
        public_key: req.public_key.and_then(|s| hex::decode(s).ok()),
    });
    let creator_addr = format!("0x{}", hex::encode(creator));
    sign_transaction(&mut tx, &state, Some(&creator_addr));
    let mut c = state.consensus.lock().unwrap();
    c.mempool.submit(tx);
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
    let mut c = state.consensus.lock().unwrap();
    c.mempool.submit(tx);
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
    let mut c = state.consensus.lock().unwrap();
    c.mempool.submit(tx);
    Json(TxResultResponse {
        success: true,
        message: format!("Resurrect queued: obj=0x{} energy_deposit={}", hex::encode(&obj_id_val[..4]), req.energy_deposit),
        tx_hash: Some(hash),
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
    let mut c = state.consensus.lock().unwrap();
    c.mempool.submit(tx);
    let hash = tx_hash(&format!("deploy:{}:{}:{}", req.template, req.energy, req.half_life));
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Deploy queued: template={} energy={} hl={} (mempool={})",
            req.template, req.energy, req.half_life, c.mempool.len()
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
    let mut c = state.consensus.lock().unwrap();
    c.mempool.submit(tx);
    let hash = tx_hash(&format!("call:{}:{}:{}", req.contract_id, req.method, c.mempool.len()));
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Call queued: contract={} method={} (mempool={})",
            req.contract_id, req.method, c.mempool.len()
        ),
        tx_hash: Some(hash),
    })
}

async fn get_contracts(
    State(state): State<Arc<ApiState>>,
) -> Json<serde_json::Value> {
    let c = state.consensus.lock().unwrap();
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
    let c = state.consensus.lock().unwrap();
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

async fn dashboard_html() -> impl IntoResponse {
    // Serve the bundled dashboard (built React app)
    // Falls back to a simple redirect message if no build exists
    let html = include_str!("../dashboard/index.html");
    Html(html)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

// ──────────────────────────── Address Detail ─────────────────────────────

async fn address_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/address.html"))
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

    let db = state.db.lock().unwrap();
    let history = state.block_history.lock().unwrap();
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
        })
    }).collect();
    drop(history);
    drop(db);

    // NFTs owned by this address
    let nft_store = state.nft_store.lock().unwrap();
    let nfts: Vec<NftResponse> = nft_store.tokens.iter()
        .filter(|n| n.owner == full_hex || n.owner.contains(&addr_hex))
        .map(|n| nft_to_response(n, epoch))
        .collect();
    drop(nft_store);

    // Token balances
    let token_store = state.token_store.lock().unwrap();
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
    Json(req): Json<FaucetRequest>,
) -> impl IntoResponse {
    let addr = match parse_address_value(&req.address) {
        Ok(a) => a,
        Err(e) => return (StatusCode::OK, Json(FaucetResponse { success: false, balance: 0, message: Some(format!("Invalid address: {}", e)) })),
    };
    let addr_key = hex::encode(&addr[..20]);

    // Rate limit check
    {
        let mut limits = state.faucet_rate_limit.lock().unwrap();
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

    // Credit the account
    let mut db = state.db.lock().unwrap();
    let account = db.get_or_create_account(&addr);
    account.balance += FAUCET_AMOUNT;
    let balance = account.balance;

    (StatusCode::OK, Json(FaucetResponse {
        success: true,
        balance,
        message: None,
    }))
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
    let store = state.nft_store.lock().unwrap();
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);

    // Tick lifecycle for display (decay state transitions)
    drop(store);
    tick_nft_lifecycle(&state, epoch);
    let store = state.nft_store.lock().unwrap();

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
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);

    tick_nft_lifecycle(&state, epoch);
    let store = state.nft_store.lock().unwrap();

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
    let history = state.block_history.lock().unwrap();
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

    let mut store = state.nft_store.lock().unwrap();
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
    let mut store = state.nft_store.lock().unwrap();
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
        let store = state.nft_store.lock().unwrap();
        if let Some(nft) = store.tokens.iter().find(|n| n.id == req.nft_id) {
            if let Err(resp) = require_wallet_ownership(&state, user_id, &nft.owner) {
                return resp;
            }
        }
    }
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);

    let mut store = state.nft_store.lock().unwrap();
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
    let mut store = state.nft_store.lock().unwrap();
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
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = state.token_store.lock().unwrap();
    for t in store.tokens.iter_mut() { t.tick_decay(epoch); }
    let res: Vec<TokenResponse> = store.tokens.iter().map(|t| {
        let mut holders: Vec<TokenHolder> = t.balances.iter()
            .filter(|(_, b)| **b > 0)
            .map(|(a, b)| TokenHolder { address: a.clone(), balance: *b })
            .collect();
        holders.sort_by(|a, b| b.balance.cmp(&a.balance));
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
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = state.token_store.lock().unwrap();
    let t = store.tokens.iter_mut().find(|t| t.id == id).ok_or(StatusCode::NOT_FOUND)?;
    t.tick_decay(epoch);
    let mut holders: Vec<TokenHolder> = t.balances.iter()
        .filter(|(_, b)| **b > 0)
        .map(|(a, b)| TokenHolder { address: a.clone(), balance: *b })
        .collect();
    holders.sort_by(|a, b| b.balance.cmp(&a.balance));
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
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let deployer = req.deployer.unwrap_or_else(|| format!("0x{}", GENESIS_FOUNDATION));
    let mut store = state.token_store.lock().unwrap();
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
    let mut store = state.token_store.lock().unwrap();
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
    let store = state.token_store.lock().unwrap();
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
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let store = state.staking_store.lock().unwrap();
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
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let store = state.staking_store.lock().unwrap();
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
            let db = state.db.lock().unwrap();
            if let Some(acct) = db.get_account(&addr_bytes) {
                if acct.balance < req.amount {
                    return Json(TxResultResponse { success: false, message: format!("Insufficient balance: {} < {}", acct.balance, req.amount), tx_hash: None });
                }
            } else {
                return Json(TxResultResponse { success: false, message: "Account not found — use faucet first".into(), tx_hash: None });
            }
        }
    }
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = state.staking_store.lock().unwrap();
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
    let mut store = state.staking_store.lock().unwrap();
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
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = state.staking_store.lock().unwrap();
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
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = state.dao_store.lock().unwrap();
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
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = state.dao_store.lock().unwrap();
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
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let creator = req.creator.unwrap_or_else(|| format!("0x{}", GENESIS_FOUNDATION));
    let mut store = state.dao_store.lock().unwrap();
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
        let staking = state.staking_store.lock().unwrap();
        let total_staked: u64 = staking.pools.iter().flat_map(|p| p.stakers.iter())
            .filter(|s| s.address == voter)
            .map(|s| s.amount)
            .sum();
        if req.weight > total_staked {
            return Json(TxResultResponse { success: false, message: format!("Vote weight {} exceeds your total stake {}", req.weight, total_staked), tx_hash: None });
        }
    }
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = state.dao_store.lock().unwrap();
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
    let history = state.block_history.lock().unwrap();
    let latest = history.back();
    let stats = state.stats.lock().unwrap();
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
    let history = state.block_history.lock().unwrap();
    history.back().cloned().ok_or(StatusCode::NOT_FOUND).map(Json)
}

/// Mempool endpoint.
async fn get_mempool(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let c = state.consensus.lock().unwrap();
    Json(serde_json::json!({
        "pending": c.mempool.len(),
    }))
}

/// NFT collections endpoint.
async fn get_nft_collections(State(state): State<Arc<ApiState>>) -> Json<serde_json::Value> {
    let store = state.nft_store.lock().unwrap();
    let mut collections: HashMap<String, Vec<u64>> = HashMap::new();
    for nft in &store.tokens {
        collections.entry(nft.collection.clone()).or_default().push(nft.id);
    }
    let result: Vec<serde_json::Value> = collections.iter().map(|(name, ids)| {
        serde_json::json!({ "name": name, "count": ids.len(), "nft_ids": ids })
    }).collect();
    Json(serde_json::json!(result))
}

/// JSON 404 fallback handler.
async fn fallback_404() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"})))
}

/// Security headers middleware.
fn security_headers(response: &mut axum::http::Response<axum::body::Body>) {
    let h = response.headers_mut();
    h.insert("X-Frame-Options", "SAMEORIGIN".parse().unwrap());
    h.insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    h.insert("X-XSS-Protection", "1; mode=block".parse().unwrap());
    h.insert("Referrer-Policy", "strict-origin-when-cross-origin".parse().unwrap());
    h.insert("Permissions-Policy", "camera=(), microphone=(), geolocation=()".parse().unwrap());
    h.insert("Content-Security-Policy", "default-src 'self' 'unsafe-inline' 'unsafe-eval' https://testnet.evaporchain.com https://evaporchain.com; img-src 'self' data: https:; font-src 'self' https://fonts.gstatic.com; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; connect-src 'self' https://testnet.evaporchain.com".parse().unwrap());
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
        // Chain metadata
        .route("/api/chain", get(get_chain))
        // Explorer
        .route("/api/status", get(get_status))
        .route("/api/objects", get(get_objects))
        .route("/api/object/:id", get(get_single_object))
        .route("/api/accounts", get(get_accounts))
        .route("/api/ghosts", get(get_ghosts))
        .route("/api/blocks", get(get_blocks))
        .route("/api/blocks/latest", get(get_latest_block))
        .route("/api/block/latest", get(get_latest_block))
        .route("/api/block/:number", get(get_single_block))
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
        // Contracts
        .route("/api/contracts", get(get_contracts))
        .route("/api/contract/:id", get(get_contract))
        .route("/api/tx/deploy-contract", post(post_deploy_contract))
        .route("/api/tx/call-contract", post(post_call_contract))
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
        .route("/api/faucet", post(post_faucet))
        // PWA
        .route("/manifest.json", get(manifest_json))
        .route("/sw.js", get(service_worker_js))
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

/// Start the API server on the given port.
pub async fn start_api_server(state: Arc<ApiState>, auth_state: Arc<crate::auth::AuthState>, port: u16) -> anyhow::Result<()> {
    let app = create_router(state, auth_state);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!(
        "\x1b[1;36m━━━ Dashboard: http://localhost:{} ━━━\x1b[0m",
        port
    );
    axum::serve(listener, app).await?;
    Ok(())
}

// ──────────────────────────── Helpers for main.rs ─────────────────────

/// Build TxRecord entries from a block's transactions.
pub fn tx_records_from_block(block: &Block) -> Vec<TxRecord> {
    block
        .transactions
        .iter()
        .map(|tx| match tx {
            Transaction::Transfer(t) => TxRecord {
                tx_type: "transfer".to_string(),
                detail: format!(
                    "{} -> {} amount={}",
                    account_name(&t.from),
                    account_name(&t.to),
                    t.amount
                ),
            },
            Transaction::CreateObject(t) => TxRecord {
                tx_type: "create_object".to_string(),
                detail: format!(
                    "creator={} id={} energy={} hl={}",
                    account_name(&t.creator),
                    hex_short(&t.object_id),
                    t.energy,
                    t.half_life
                ),
            },
            Transaction::Refresh(t) => TxRecord {
                tx_type: "refresh".to_string(),
                detail: format!(
                    "obj={} +{}",
                    hex_short(&t.object_id),
                    t.energy_deposit
                ),
            },
            Transaction::DeployContract(t) => TxRecord {
                tx_type: "deploy_contract".to_string(),
                detail: format!(
                    "deployer={} template={} energy={} hl={}",
                    account_name(&t.deployer),
                    t.template,
                    t.energy,
                    t.half_life
                ),
            },
            Transaction::CallContract(t) => TxRecord {
                tx_type: "call_contract".to_string(),
                detail: format!(
                    "caller={} contract={} method={}",
                    account_name(&t.caller),
                    t.contract_id,
                    t.method
                ),
            },
            Transaction::DeployScript(t) => TxRecord {
                tx_type: "deploy_script".to_string(),
                detail: format!(
                    "deployer={} energy={} hl={}",
                    account_name(&t.deployer),
                    t.energy,
                    t.half_life
                ),
            },
            Transaction::CallScript(t) => TxRecord {
                tx_type: "call_script".to_string(),
                detail: format!(
                    "caller={} script={} method={}",
                    account_name(&t.caller),
                    t.contract_id,
                    t.method
                ),
            },
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
    let mut evts = events.lock().unwrap();
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

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use evaporchain_consensus::MockConsensus;
use evaporchain_state::db::StateDB;
use evaporchain_state::InMemoryStateDB;
use evaporchain_types::{
    Block, CallContractTx, CreateObjectTx, DeployContractTx, ObjectState, RefreshTx, Transaction,
    TransferTx,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{atomic::AtomicUsize, Arc, Mutex};
use std::time::Instant;
use tower_http::cors::{Any, CorsLayer};

// ──────────────────────────── Shared State ────────────────────────────

/// Shared application state accessible from API handlers.
pub struct ApiState {
    pub db: Arc<Mutex<InMemoryStateDB>>,
    pub consensus: Arc<Mutex<MockConsensus>>,
    pub peer_count: Arc<AtomicUsize>,
    pub block_history: Arc<Mutex<VecDeque<BlockRecord>>>,
    pub stats: Arc<Mutex<ChainStats>>,
    pub events: Arc<Mutex<VecDeque<EventRecord>>>,
    pub prove_mode: bool,
    pub start_time: Instant,
    /// Faucet rate limiter: address byte -> last request timestamp.
    pub faucet_rate_limit: Mutex<HashMap<u8, Instant>>,
    /// NFT marketplace store.
    pub nft_store: Arc<Mutex<NftStore>>,
    /// Token store.
    pub token_store: Arc<Mutex<TokenStore>>,
    /// Staking store.
    pub staking_store: Arc<Mutex<StakingStore>>,
    /// DAO store.
    pub dao_store: Arc<Mutex<DAOStore>>,
}

// ──────────────────────────── NFT Store ────────────────────────────────

/// In-memory NFT storage for the MortalNFT marketplace.
#[derive(Clone, Serialize)]
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

pub struct NftStore {
    pub tokens: Vec<NftToken>,
    pub next_id: u64,
}

/// Record of a produced/applied block for the API.
#[derive(Clone, Serialize)]
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
#[derive(Clone, Serialize)]
pub struct TxRecord {
    #[serde(rename = "type")]
    pub tx_type: String,
    pub detail: String,
}

/// Accumulated chain statistics.
#[derive(Clone, Serialize)]
pub struct ChainStats {
    pub total_objects_created: u64,
    pub total_evaporated: u64,
    pub total_resurrected: u64,
    pub total_refreshed: u64,
    pub total_transactions: u64,
    pub state_size_trend: Vec<EpochSnapshot>,
}

#[derive(Clone, Serialize)]
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
#[derive(Clone, Serialize)]
pub struct EventRecord {
    pub epoch: u64,
    pub event_type: String, // "grace", "evaporated", "created", "refreshed", "transfer", "resurrected"
    pub message: String,
    pub timestamp_ms: u64,
}

// ──────────────────────────── Name Helpers ─────────────────────────────

fn addr_from_byte(b: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    a[0] = b;
    a
}

fn obj_id_from_byte(b: u8) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[0] = b;
    id
}

/// Display address as truncated hex (no human names).
fn account_name(addr: &[u8; 32]) -> String {
    format!("0x{}…{}", &hex::encode(&addr[..3]), &hex::encode(&addr[30..]))
}

/// Try to extract a name from the object's data field, otherwise use hex id.
fn object_name(id: &[u8; 32], data: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(data) {
        if !s.is_empty() && s.len() < 64 {
            return s.to_string();
        }
    }
    format!("0x{:02x}{:02x}...", id[0], id[1])
}

fn hex_short(bytes: &[u8]) -> String {
    let full = hex::encode(bytes);
    if full.len() > 16 {
        format!("{}...", &full[..16])
    } else {
        full
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

// ── Transaction request types ──

#[derive(Deserialize)]
struct TransferRequest {
    from: u8,
    to: u8,
    amount: u64,
    nonce: u64,
}

#[derive(Deserialize)]
struct CreateObjectRequest {
    creator: u8,
    object_id: u8,
    energy: u64,
    half_life: u64,
}

#[derive(Deserialize)]
struct RefreshRequest {
    object_id: u8,
    energy_deposit: u64,
}

#[derive(Serialize)]
struct TxResultResponse {
    success: bool,
    message: String,
}

#[derive(Deserialize)]
struct FaucetRequest {
    address: u8,
}

#[derive(Serialize)]
struct FaucetResponse {
    success: bool,
    balance: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
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
    Json(req): Json<TransferRequest>,
) -> Json<TxResultResponse> {
    let tx = Transaction::Transfer(TransferTx {
        from: addr_from_byte(req.from),
        to: addr_from_byte(req.to),
        amount: req.amount,
        nonce: req.nonce,
        signature: None,
        public_key: None,
    });
    let mut c = state.consensus.lock().unwrap();
    c.mempool.submit(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Transfer queued: {} -> {} amount={} (mempool={})",
            account_name(&addr_from_byte(req.from)),
            account_name(&addr_from_byte(req.to)),
            req.amount,
            c.mempool.len()
        ),
    })
}

async fn post_create_object(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<CreateObjectRequest>,
) -> Json<TxResultResponse> {
    let tx = Transaction::CreateObject(CreateObjectTx {
        creator: addr_from_byte(req.creator),
        object_id: obj_id_from_byte(req.object_id),
        energy: req.energy,
        half_life: req.half_life,
        data: format!("UserObj-{}", req.object_id).into_bytes(),
        signature: None,
        public_key: None,
    });
    let mut c = state.consensus.lock().unwrap();
    c.mempool.submit(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "CreateObject queued: id=0x{:02x} energy={} half_life={} (mempool={})",
            req.object_id,
            req.energy,
            req.half_life,
            c.mempool.len()
        ),
    })
}

async fn post_refresh(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<RefreshRequest>,
) -> Json<TxResultResponse> {
    let tx = Transaction::Refresh(RefreshTx {
        object_id: obj_id_from_byte(req.object_id),
        energy_deposit: req.energy_deposit,
        signature: None,
        public_key: None,
    });
    let mut c = state.consensus.lock().unwrap();
    c.mempool.submit(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Refresh queued: obj=0x{:02x} energy_deposit={} (mempool={})",
            req.object_id,
            req.energy_deposit,
            c.mempool.len()
        ),
    })
}

// Resurrect uses the same mechanism as refresh (refresh on a ghost)
async fn post_resurrect(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<RefreshRequest>,
) -> Json<TxResultResponse> {
    let tx = Transaction::Refresh(RefreshTx {
        object_id: obj_id_from_byte(req.object_id),
        energy_deposit: req.energy_deposit,
        signature: None,
        public_key: None,
    });
    let mut c = state.consensus.lock().unwrap();
    c.mempool.submit(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Resurrect queued: obj=0x{:02x} energy_deposit={} (mempool={})",
            req.object_id,
            req.energy_deposit,
            c.mempool.len()
        ),
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
    Json(req): Json<DeployContractRequest>,
) -> Json<TxResultResponse> {
    let tx = Transaction::DeployContract(DeployContractTx {
        deployer: addr_from_byte(req.deployer),
        template: req.template.clone(),
        init_args: serde_json::to_string(&req.init_args).unwrap_or_default(),
        energy: req.energy,
        half_life: req.half_life,
        rules: req.rules.map(|r| serde_json::to_string(&r).unwrap_or_default()),
        signature: None,
        public_key: None,
    });
    let mut c = state.consensus.lock().unwrap();
    c.mempool.submit(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Deploy queued: template={} energy={} hl={} (mempool={})",
            req.template, req.energy, req.half_life, c.mempool.len()
        ),
    })
}

async fn post_call_contract(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<CallContractRequest>,
) -> Json<TxResultResponse> {
    let tx = Transaction::CallContract(CallContractTx {
        caller: addr_from_byte(req.caller),
        contract_id: req.contract_id,
        method: req.method.clone(),
        args: serde_json::to_string(&req.args).unwrap_or_default(),
        epoch: req.epoch,
        signature: None,
        public_key: None,
    });
    let mut c = state.consensus.lock().unwrap();
    c.mempool.submit(tx);
    Json(TxResultResponse {
        success: true,
        message: format!(
            "Call queued: contract={} method={} (mempool={})",
            req.contract_id, req.method, c.mempool.len()
        ),
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

// ──────────────────────────── Faucet ───────────────────────────────────

const FAUCET_AMOUNT: u64 = 10_000;
const FAUCET_RATE_LIMIT_SECS: u64 = 3600; // 1 hour

async fn wallet_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/wallet.html"))
}

async fn faucet_html() -> impl IntoResponse {
    Html(include_str!("../dashboard/faucet.html"))
}

async fn post_faucet(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<FaucetRequest>,
) -> Json<FaucetResponse> {
    // Rate limit check
    {
        let mut limits = state.faucet_rate_limit.lock().unwrap();
        if let Some(last) = limits.get(&req.address) {
            if last.elapsed().as_secs() < FAUCET_RATE_LIMIT_SECS {
                let remaining = FAUCET_RATE_LIMIT_SECS - last.elapsed().as_secs();
                return Json(FaucetResponse {
                    success: false,
                    balance: 0,
                    message: Some(format!(
                        "Rate limited. Try again in {} minutes.",
                        remaining / 60 + 1
                    )),
                });
            }
        }
        limits.insert(req.address, Instant::now());
    }

    // Credit the account
    let mut db = state.db.lock().unwrap();
    let addr = addr_from_byte(req.address);
    let account = db.get_or_create_account(&addr);
    account.balance += FAUCET_AMOUNT;
    let balance = account.balance;

    Json(FaucetResponse {
        success: true,
        balance,
        message: None,
    })
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
    Json(req): Json<MintNftRequest>,
) -> Json<TxResultResponse> {
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);

    let metadata_hash = blake3::hash(req.metadata.as_bytes()).to_hex().to_string();
    let owner = req.owner.unwrap_or_else(|| "0x7f0000…0000".to_string());
    let collection = req.collection.unwrap_or_else(|| "Genesis Collection".to_string());

    let mut store = state.nft_store.lock().unwrap();
    let id = store.next_id;
    store.next_id += 1;
    store.tokens.push(NftToken {
        id,
        name: req.name.clone(),
        collection,
        owner,
        metadata_hash: metadata_hash[..16].to_string(),
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

    Json(TxResultResponse {
        success: true,
        message: format!("NFT #{} '{}' minted with energy={}, half_life={}", id, req.name, req.energy, req.half_life),
    })
}

#[derive(Deserialize)]
struct TransferNftRequest {
    nft_id: u64,
    to: String,
}

async fn post_transfer_nft(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<TransferNftRequest>,
) -> Json<TxResultResponse> {
    let mut store = state.nft_store.lock().unwrap();
    if let Some(nft) = store.tokens.iter_mut().find(|n| n.id == req.nft_id) {
        if nft.state == "Ghost" {
            return Json(TxResultResponse {
                success: false,
                message: "Cannot transfer a ghost NFT".to_string(),
            });
        }
        let from = nft.owner.clone();
        nft.owner = req.to.clone();
        Json(TxResultResponse {
            success: true,
            message: format!("NFT #{} transferred from {} to {}", req.nft_id, from, req.to),
        })
    } else {
        Json(TxResultResponse {
            success: false,
            message: format!("NFT #{} not found", req.nft_id),
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
    Json(req): Json<RefreshNftRequest>,
) -> Json<TxResultResponse> {
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);

    let mut store = state.nft_store.lock().unwrap();
    if let Some(nft) = store.tokens.iter_mut().find(|n| n.id == req.nft_id) {
        if nft.state == "Ghost" {
            // Resurrect
            nft.state = "Active".to_string();
            nft.energy = req.energy;
            nft.max_energy = req.energy;
            nft.last_refreshed = epoch;
            nft.grace_epoch = None;
            nft.evaporated_epoch = None;
            nft.ghost_proof = None;
            Json(TxResultResponse {
                success: true,
                message: format!("NFT #{} '{}' resurrected with energy={}", nft.id, nft.name, req.energy),
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
            Json(TxResultResponse {
                success: true,
                message: format!("NFT #{} '{}' refreshed, energy now {}", nft.id, nft.name, nft.energy),
            })
        }
    } else {
        Json(TxResultResponse {
            success: false,
            message: format!("NFT #{} not found", req.nft_id),
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
                    nft.ghost_proof = Some(hash.to_hex()[..32].to_string());
                }
            }
        }
    }
}

// ──────────────────────────── Token Store ───────────────────────────────

#[derive(Clone, Serialize)]
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

async fn post_deploy_token(State(state): State<Arc<ApiState>>, Json(req): Json<DeployTokenRequest>) -> Json<TxResultResponse> {
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let deployer = req.deployer.unwrap_or_else(|| "0x7f0000…0000".to_string());
    let mut store = state.token_store.lock().unwrap();
    let id = store.next_id; store.next_id += 1;
    let mut balances = HashMap::new();
    balances.insert(deployer.clone(), req.total_supply);
    store.tokens.push(DeployedToken {
        id, name: req.name.clone(), symbol: req.symbol.clone(),
        total_supply: req.total_supply, decay_half_life: req.decay_half_life,
        deployed_epoch: epoch, deployer, balances, last_decay_epoch: epoch,
    });
    Json(TxResultResponse { success: true, message: format!("{} ({}) deployed with supply={}, half_life={}", req.name, req.symbol, req.total_supply, req.decay_half_life) })
}

#[derive(Deserialize)]
struct TokenTransferRequest { token_id: u64, from: String, to: String, amount: u64 }

async fn post_token_transfer(State(state): State<Arc<ApiState>>, Json(req): Json<TokenTransferRequest>) -> Json<TxResultResponse> {
    let mut store = state.token_store.lock().unwrap();
    let t = match store.tokens.iter_mut().find(|t| t.id == req.token_id) {
        Some(t) => t, None => return Json(TxResultResponse { success: false, message: "Token not found".into() }),
    };
    let from_bal = t.balances.get(&req.from).copied().unwrap_or(0);
    if from_bal < req.amount {
        return Json(TxResultResponse { success: false, message: format!("Insufficient balance: {} < {}", from_bal, req.amount) });
    }
    *t.balances.entry(req.from.clone()).or_insert(0) -= req.amount;
    *t.balances.entry(req.to.clone()).or_insert(0) += req.amount;
    Json(TxResultResponse { success: true, message: format!("{} {} transferred from {} to {}", req.amount, t.symbol, req.from, req.to) })
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

#[derive(Clone, Serialize)]
pub struct StakingPool {
    pub id: u64,
    pub name: String,
    pub reward_rate: u64,       // per epoch
    pub reward_decay_hl: u64,   // reward decay half-life
    pub total_staked: u64,
    pub created_epoch: u64,
    pub stakers: Vec<Staker>,
}

#[derive(Clone, Serialize)]
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

async fn post_stake(State(state): State<Arc<ApiState>>, Json(req): Json<StakeRequest>) -> Json<TxResultResponse> {
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = state.staking_store.lock().unwrap();
    let p = match store.pools.iter_mut().find(|p| p.id == req.pool_id) {
        Some(p) => p, None => return Json(TxResultResponse { success: false, message: "Pool not found".into() }),
    };
    if let Some(s) = p.stakers.iter_mut().find(|s| s.address == req.address) {
        s.amount += req.amount;
    } else {
        p.stakers.push(Staker { address: req.address.clone(), amount: req.amount, staked_epoch: epoch, pending_rewards: 0, last_claim_epoch: epoch, total_claimed: 0, total_decayed: 0 });
    }
    p.total_staked += req.amount;
    Json(TxResultResponse { success: true, message: format!("Staked {} in {}", req.amount, p.name) })
}

#[derive(Deserialize)]
struct UnstakeRequest { pool_id: u64, address: String, amount: u64 }

async fn post_unstake(State(state): State<Arc<ApiState>>, Json(req): Json<UnstakeRequest>) -> Json<TxResultResponse> {
    let mut store = state.staking_store.lock().unwrap();
    let p = match store.pools.iter_mut().find(|p| p.id == req.pool_id) {
        Some(p) => p, None => return Json(TxResultResponse { success: false, message: "Pool not found".into() }),
    };
    let s = match p.stakers.iter_mut().find(|s| s.address == req.address) {
        Some(s) => s, None => return Json(TxResultResponse { success: false, message: "Not staked".into() }),
    };
    if s.amount < req.amount {
        return Json(TxResultResponse { success: false, message: format!("Insufficient stake: {} < {}", s.amount, req.amount) });
    }
    s.amount -= req.amount;
    p.total_staked -= req.amount;
    Json(TxResultResponse { success: true, message: format!("Unstaked {} from {}", req.amount, p.name) })
}

#[derive(Deserialize)]
struct ClaimRequest { pool_id: u64, address: String }

async fn post_claim(State(state): State<Arc<ApiState>>, Json(req): Json<ClaimRequest>) -> Json<TxResultResponse> {
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let mut store = state.staking_store.lock().unwrap();
    let p = match store.pools.iter_mut().find(|p| p.id == req.pool_id) {
        Some(p) => p, None => return Json(TxResultResponse { success: false, message: "Pool not found".into() }),
    };
    let reward_decay_hl = p.reward_decay_hl;
    let reward_rate = p.reward_rate;
    let total_staked = p.total_staked;
    let s = match p.stakers.iter_mut().find(|s| s.address == req.address) {
        Some(s) => s, None => return Json(TxResultResponse { success: false, message: "Not staked".into() }),
    };
    // Compute rewards
    let epochs_since = epoch.saturating_sub(s.last_claim_epoch);
    let share = if total_staked > 0 { s.amount as f64 / total_staked as f64 } else { 0.0 };
    let raw = (share * reward_rate as f64 * epochs_since as f64) as u64 + s.pending_rewards;
    let actual = evaporchain_types::energy_at_epoch(raw, reward_decay_hl, epochs_since);
    let decayed = raw.saturating_sub(actual);
    s.total_claimed += actual;
    s.total_decayed += decayed;
    s.pending_rewards = 0;
    s.last_claim_epoch = epoch;
    Json(TxResultResponse { success: true, message: format!("Claimed {} rewards ({} decayed)", actual, decayed) })
}

// ──────────────────────────── DAO Store ─────────────────────────────────

#[derive(Clone, Serialize)]
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

#[derive(Clone, Serialize)]
pub struct DAOVote {
    pub voter: String,
    pub option: String,
    pub weight: u64,
    pub epoch: u64,
}

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

async fn post_propose(State(state): State<Arc<ApiState>>, Json(req): Json<ProposeRequest>) -> Json<TxResultResponse> {
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let creator = req.creator.unwrap_or_else(|| "0x7f0000…0000".to_string());
    let mut store = state.dao_store.lock().unwrap();
    let id = store.next_id; store.next_id += 1;
    store.proposals.push(DAOProposal {
        id, title: req.title.clone(), description: req.description,
        options: req.options, votes: vec![], created_epoch: epoch,
        voting_period: req.voting_period, creator, status: "Active".into(),
        evaporated_epoch: None,
    });
    Json(TxResultResponse { success: true, message: format!("Proposal #{} '{}' created, voting for {} epochs", id, req.title, req.voting_period) })
}

#[derive(Deserialize)]
struct VoteRequest { proposal_id: u64, option: String, weight: u64, voter: Option<String> }

async fn post_vote(State(state): State<Arc<ApiState>>, Json(req): Json<VoteRequest>) -> Json<TxResultResponse> {
    let history = state.block_history.lock().unwrap();
    let epoch = history.back().map(|b| b.epoch).unwrap_or(0);
    drop(history);
    let voter = req.voter.unwrap_or_else(|| "0x7f0000…0000".to_string());
    let mut store = state.dao_store.lock().unwrap();
    let p = match store.proposals.iter_mut().find(|p| p.id == req.proposal_id) {
        Some(p) => p, None => return Json(TxResultResponse { success: false, message: "Proposal not found".into() }),
    };
    if p.status != "Active" {
        return Json(TxResultResponse { success: false, message: format!("Proposal is {}, cannot vote", p.status) });
    }
    if !p.options.contains(&req.option) {
        return Json(TxResultResponse { success: false, message: format!("Invalid option: {}", req.option) });
    }
    // Check if already voted
    if p.votes.iter().any(|v| v.voter == voter) {
        return Json(TxResultResponse { success: false, message: "Already voted".into() });
    }
    p.votes.push(DAOVote { voter: voter.clone(), option: req.option.clone(), weight: req.weight, epoch });
    Json(TxResultResponse { success: true, message: format!("Voted '{}' with weight {} on proposal #{}", req.option, req.weight, req.proposal_id) })
}

// ──────────────────────────── Router ───────────────────────────────────

pub fn create_router(state: Arc<ApiState>, auth_state: Arc<crate::auth::AuthState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

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
        // Explorer
        .route("/api/status", get(get_status))
        .route("/api/objects", get(get_objects))
        .route("/api/object/{id}", get(get_single_object))
        .route("/api/accounts", get(get_accounts))
        .route("/api/ghosts", get(get_ghosts))
        .route("/api/blocks", get(get_blocks))
        .route("/api/block/{number}", get(get_single_block))
        .route("/api/events", get(get_events))
        // Stats
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
        .route("/api/contract/{id}", get(get_contract))
        .route("/api/tx/deploy-contract", post(post_deploy_contract))
        .route("/api/tx/call-contract", post(post_call_contract))
        // NFT Marketplace
        .route("/nft", get(nft_html))
        .route("/api/nfts", get(get_nfts))
        .route("/api/nft/{id}", get(get_single_nft))
        .route("/api/nft/mint", post(post_mint_nft))
        .route("/api/nft/transfer", post(post_transfer_nft))
        .route("/api/nft/refresh", post(post_refresh_nft))
        // Tokens
        .route("/tokens", get(tokens_html))
        .route("/api/tokens", get(get_tokens))
        .route("/api/token/{id}", get(get_single_token))
        .route("/api/token/deploy", post(post_deploy_token))
        .route("/api/token/transfer", post(post_token_transfer))
        .route("/api/token/balance", post(post_token_balance))
        // Staking
        .route("/staking", get(staking_html))
        .route("/api/staking/pools", get(get_staking_pools))
        .route("/api/staking/pool/{id}", get(get_single_pool))
        .route("/api/staking/stake", post(post_stake))
        .route("/api/staking/unstake", post(post_unstake))
        .route("/api/staking/claim", post(post_claim))
        // DAO
        .route("/dao", get(dao_html))
        .route("/api/dao/proposals", get(get_proposals))
        .route("/api/dao/proposal/{id}", get(get_single_proposal))
        .route("/api/dao/propose", post(post_propose))
        .route("/api/dao/vote", post(post_vote))
        // Faucet
        .route("/faucet", get(faucet_html))
        .route("/api/faucet", post(post_faucet))
        .with_state(state)
        // Merge auth routes (different state type)
        .merge(auth_router)
        .layer(cors)
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

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

/// Map known genesis addresses to human names.
fn account_name(addr: &[u8; 32]) -> String {
    match addr[0] {
        1 if addr[1..].iter().all(|&b| b == 0) => "Alice".to_string(),
        2 if addr[1..].iter().all(|&b| b == 0) => "Bob".to_string(),
        3 if addr[1..].iter().all(|&b| b == 0) => "Charlie".to_string(),
        _ => format!("0x{}...", &hex::encode(&addr[..4])),
    }
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

// ──────────────────────────── Router ───────────────────────────────────

pub fn create_router(state: Arc<ApiState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Dashboard
        .route("/", get(dashboard_html))
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
        // Faucet
        .route("/faucet", get(faucet_html))
        .route("/api/faucet", post(post_faucet))
        .layer(cors)
        .with_state(state)
}

/// Start the API server on the given port.
pub async fn start_api_server(state: Arc<ApiState>, port: u16) -> anyhow::Result<()> {
    let app = create_router(state);
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

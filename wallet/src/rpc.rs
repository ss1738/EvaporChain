//! Typed async RPC client for EvaporChain node.
//!
//! Provides strongly-typed methods for every REST endpoint on the
//! EvaporChain node, including chain queries, transaction submission,
//! NFT/token operations, staking, and governance.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use url::Url;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum RpcError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("invalid base URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("deserialization error: {0}")]
    Deserialize(String),
}

// ──────────────────────────── Response Types ───────────────────────────

/// Chain status from GET /api/status.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatusResponse {
    pub chain_name: String,
    pub version: String,
    pub block_height: u64,
    pub epoch: u64,
    pub active_objects: usize,
    pub ghost_count: usize,
    pub total_evaporated: u64,
    pub peer_count: usize,
    pub state_root: String,
    pub proving_enabled: bool,
    pub uptime_seconds: u64,
}

/// Chain metadata from GET /api/chain.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChainInfoResponse {
    pub chain_name: String,
    pub chain_id: String,
    pub version: String,
    pub block_height: u64,
    pub epoch: u64,
    pub total_transactions: u64,
    pub consensus: String,
    pub signature_scheme: String,
}

/// Single account from GET /api/accounts.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountResponse {
    pub address: String,
    pub name: String,
    pub balance: u64,
    pub nonce: u64,
}

/// Full address detail from GET /api/address/:addr.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AddressDetailResponse {
    pub address: String,
    pub balance: u64,
    pub nonce: u64,
    pub objects: Vec<ObjectResponse>,
    pub nfts: Vec<NftResponse>,
    pub tokens: Vec<TokenBalanceEntry>,
}

/// Token balance entry inside AddressDetailResponse.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenBalanceEntry {
    pub token_id: u64,
    pub symbol: String,
    pub name: String,
    pub balance: u64,
}

/// State object from GET /api/objects or /api/object/:id.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ObjectResponse {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub owner_name: String,
    pub energy: u64,
    pub max_energy: u64,
    pub half_life: u64,
    pub state: String,
    pub created_epoch: u64,
    pub last_refreshed: u64,
    pub grace_epoch: Option<u64>,
    pub current_energy: u64,
    pub decay_percentage: f64,
}

/// Ghost record from GET /api/ghosts.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GhostResponse {
    pub id: String,
    pub original_owner: String,
    pub evaporated_epoch: u64,
    pub data_hash: String,
}

/// Block record from GET /api/blocks or /api/block/:number.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
}

/// Transaction record from GET /api/transactions (the historical list).
///
/// NOTE: The `/api/tx/:hash` endpoint no longer returns this shape. It now
/// returns [`TxStatus`] (lifecycle state only — no tx body). If a caller
/// needs the originating tx fields (`from`, `to`, `amount`, `tx_type`,
/// etc.) for a known hash, it must scan `/api/transactions` for a
/// matching `hash`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TxRecord {
    pub hash: String,
    #[serde(rename = "type")]
    pub tx_type: String,
    pub from: String,
    pub to: String,
    pub amount: Option<u64>,
    pub object_id: Option<String>,
    pub energy: Option<u64>,
    pub half_life: Option<u64>,
    pub method: Option<String>,
    pub gas: u64,
    pub block_number: u64,
    pub epoch: u64,
    pub status: String,
}

/// Lifecycle state of a transaction, as returned by `GET /api/tx/:hash`.
///
/// Wire-format strings are lowercase (`"pending"`, `"mempool"`,
/// `"included"`, `"finalised"`, `"rejected"`) so the JSON deserialises
/// directly from the node's `TxStatusResponse.state` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TxState {
    Pending,
    Mempool,
    Included,
    Finalised,
    Rejected,
}

/// Transaction status from `GET /api/tx/:hash` (new shape, post commit
/// `d0394b1`). Carries lifecycle state only — no tx body. Use
/// [`RpcClient::get_transactions`] to fetch the full body for a hash.
///
/// `block_height` / `epoch` are present once the tx is `included`,
/// `finalised`, or `rejected`. `error` is populated only when
/// `state == TxState::Rejected`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TxStatus {
    /// Canonical 64-char lowercase hex (no `0x` prefix) per the node API.
    pub hash: String,
    pub state: TxState,
    #[serde(default)]
    pub block_height: Option<u64>,
    #[serde(default)]
    pub epoch: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Stats summary from GET /api/stats/summary.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatsSummaryResponse {
    pub total_created: u64,
    pub total_evaporated: u64,
    pub total_resurrected: u64,
    pub total_refreshed: u64,
    pub avg_lifetime_epochs: f64,
    pub total_transactions: u64,
}

/// Epoch snapshot for timeline.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EpochSnapshot {
    pub epoch: u64,
    pub active_count: usize,
    pub ghost_count: usize,
    pub total_energy: u64,
}

/// Event record from GET /api/events.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventRecord {
    pub epoch: u64,
    pub event_type: String,
    pub message: String,
    pub timestamp_ms: u64,
}

/// NFT from GET /api/nfts or /api/nft/:id.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NftResponse {
    pub id: u64,
    pub name: String,
    pub collection: String,
    pub owner: String,
    pub metadata_hash: String,
    pub energy: u64,
    pub max_energy: u64,
    pub current_energy: u64,
    pub half_life: u64,
    pub minted_epoch: u64,
    pub last_refreshed: u64,
    pub state: String,
    pub decay_percentage: f64,
    pub epochs_remaining: u64,
    pub grace_epoch: Option<u64>,
    pub evaporated_epoch: Option<u64>,
    pub ghost_proof: Option<String>,
}

/// Token from GET /api/tokens or /api/token/:id.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenResponse {
    pub id: u64,
    pub name: String,
    pub symbol: String,
    pub total_supply: u64,
    pub current_supply: u64,
    pub decay_half_life: u64,
    pub deployed_epoch: u64,
    pub deployer: String,
    pub decay_percentage: f64,
    pub holder_count: usize,
    pub holders: Vec<TokenHolder>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenHolder {
    pub address: String,
    pub balance: u64,
}

/// Staking pool from GET /api/staking/pools.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StakingPoolResponse {
    pub id: u64,
    pub name: String,
    pub reward_rate: u64,
    pub reward_decay_hl: u64,
    pub total_staked: u64,
    pub created_epoch: u64,
    pub staker_count: usize,
    pub stakers: Vec<StakerResponse>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StakerResponse {
    pub address: String,
    pub amount: u64,
    pub staked_epoch: u64,
    pub pending_rewards: u64,
    pub last_claim_epoch: u64,
    pub total_claimed: u64,
    pub total_decayed: u64,
    pub reward_decay_pct: f64,
}

/// DAO proposal from GET /api/dao/proposals.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProposalResponse {
    pub id: u64,
    pub title: String,
    pub description: String,
    pub options: Vec<String>,
    pub created_epoch: u64,
    pub voting_period: u64,
    pub end_epoch: u64,
    pub creator: String,
    pub status: String,
    pub total_votes: u64,
    pub vote_totals: HashMap<String, u64>,
    pub epochs_remaining: u64,
    pub evaporated_epoch: Option<u64>,
    pub voter_count: usize,
}

/// Standard transaction result from POST endpoints.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TxResultResponse {
    pub success: bool,
    pub message: String,
    pub tx_hash: Option<String>,
}

/// Faucet response from POST /api/faucet.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FaucetResponse {
    pub success: bool,
    pub balance: u64,
    pub message: Option<String>,
}

/// Paginated transactions response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionsResponse {
    pub transactions: Vec<TxRecord>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

/// Events response wrapper.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventsResponse {
    pub events: Vec<EventRecord>,
}

/// Stats timeline response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatsTimelineResponse {
    pub epochs: Vec<EpochSnapshot>,
}

/// Network info response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NetworkResponse {
    pub peer_count: usize,
}

/// Mempool info response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MempoolResponse {
    pub pending: usize,
}

/// Contract info from GET /api/contracts.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContractInfo {
    pub id: u64,
    pub template: String,
    pub creator: String,
    pub energy: u64,
    pub half_life: u64,
    pub created_epoch: u64,
    pub evaporated: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContractsResponse {
    pub contracts: Vec<ContractInfo>,
}

/// NFT collection summary.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NftCollection {
    pub name: String,
    pub count: usize,
    pub nft_ids: Vec<u64>,
}

// ──────────────────────────── Request Types ────────────────────────────

/// Transfer request body.
#[derive(Debug, Serialize)]
pub struct TransferRequest {
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub nonce: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
}

/// Create object request body.
#[derive(Debug, Serialize)]
pub struct CreateObjectRequest {
    pub creator: String,
    pub object_id: String,
    pub energy: u64,
    pub half_life: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
}

/// Refresh request body.
#[derive(Debug, Serialize)]
pub struct RefreshRequest {
    pub object_id: String,
    pub energy_deposit: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
}

/// Deploy contract request body.
#[derive(Debug, Serialize)]
pub struct DeployContractRequest {
    pub deployer: String,
    pub template: String,
    pub init_args: serde_json::Value,
    pub energy: u64,
    pub half_life: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<serde_json::Value>,
}

/// Call contract request body.
#[derive(Debug, Serialize)]
pub struct CallContractRequest {
    pub caller: String,
    pub contract_id: u64,
    pub method: String,
    pub args: serde_json::Value,
    pub epoch: u64,
}

/// Mint NFT request body.
#[derive(Debug, Serialize)]
pub struct MintNftRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    pub metadata: String,
    pub energy: u64,
    pub half_life: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

/// Transfer NFT request body.
#[derive(Debug, Serialize)]
pub struct TransferNftRequest {
    pub nft_id: u64,
    pub to: String,
}

/// Refresh NFT request body.
#[derive(Debug, Serialize)]
pub struct RefreshNftRequest {
    pub nft_id: u64,
    pub energy: u64,
}

/// Deploy token request body.
#[derive(Debug, Serialize)]
pub struct DeployTokenRequest {
    pub name: String,
    pub symbol: String,
    pub total_supply: u64,
    pub decay_half_life: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployer: Option<String>,
}

/// Transfer token request body.
#[derive(Debug, Serialize)]
pub struct TokenTransferRequest {
    pub token_id: u64,
    pub from: String,
    pub to: String,
    pub amount: u64,
}

/// Token balance query body.
#[derive(Debug, Serialize)]
pub struct TokenBalanceRequest {
    pub token_id: u64,
    pub address: String,
}

/// Stake request body.
#[derive(Debug, Serialize)]
pub struct StakeRequest {
    pub pool_id: u64,
    pub address: String,
    pub amount: u64,
}

/// Unstake request body.
#[derive(Debug, Serialize)]
pub struct UnstakeRequest {
    pub pool_id: u64,
    pub address: String,
    pub amount: u64,
}

/// Claim rewards request body.
#[derive(Debug, Serialize)]
pub struct ClaimRequest {
    pub pool_id: u64,
    pub address: String,
}

/// Create DAO proposal request body.
#[derive(Debug, Serialize)]
pub struct ProposeRequest {
    pub title: String,
    pub description: String,
    pub options: Vec<String>,
    pub voting_period: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
}

/// Vote on DAO proposal request body.
#[derive(Debug, Serialize)]
pub struct VoteRequest {
    pub proposal_id: u64,
    pub option: String,
    pub weight: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voter: Option<String>,
}

/// Faucet request body.
#[derive(Debug, Serialize)]
pub struct FaucetRequest {
    pub address: String,
}

// ──────────────────────────── Substrate primitives ─────────────────────
//
// Typed request/response shapes mirroring `crates/evaporchain-node/src/
// api.rs` byte-for-byte. Field names match the node structs exactly so
// JSON round-trips without renames.

// ─── Patronage covenants ─────────────────────────────────────────────

/// GET /api/patronage/status response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PatronageStatus {
    pub active_covenants: usize,
    pub total_pre_funded: u64,
    pub total_active_score: u64,
    pub patronage_ns_hex: String,
}

/// GET /api/patronage/immune response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PatronageImmunity {
    pub object_id_hex: String,
    pub epoch: u64,
    pub immune: bool,
    pub patronage_score: u64,
}

/// POST /api/patronage/pledge request body.
#[derive(Debug, Clone, Serialize)]
pub struct PatronagePledgeRequest {
    pub object_id_hex: String,
    pub namespace_id_hex: String,
    pub donation_per_epoch: u64,
    pub epochs: u64,
    pub current_epoch: u64,
}

/// POST /api/patronage/pledge response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PatronagePledgeResponse {
    pub status: String,
    pub object_id_hex: String,
    pub pre_funded: u64,
    pub expires_epoch: u64,
    pub detail: String,
}

/// POST /api/patronage/honour and /revoke share `{object_id_hex, epoch}`.
#[derive(Debug, Clone, Serialize)]
pub struct PatronageActionRequest {
    pub object_id_hex: String,
    pub epoch: u64,
}

/// POST /api/patronage/honour response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PatronageHonourResponse {
    pub status: String,
    pub donated: u64,
    pub patronage_score: u64,
    pub detail: String,
}

// ─── Governance fork-choice ──────────────────────────────────────────

/// One attractor entry inside a fork-choice amendment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttractorSpec {
    pub center: u64,
    pub basin_radius: u64,
}

/// POST /api/governance/fork_choice_mode request body. The amendment is
/// authorised by the supplied `endorser_stakes` summing to at least
/// `required_stake` — there is no separate signature on the body
/// (matches `ForkChoiceAmendReq` in api.rs).
#[derive(Debug, Clone, Serialize)]
pub struct ForkChoiceAmendRequest {
    /// `"mcc"` or `"singh_attractor"`.
    pub mode: String,
    /// Required when `mode == "singh_attractor"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attractors: Option<Vec<AttractorSpec>>,
    pub endorser_stakes: Vec<u64>,
    pub required_stake: u64,
}

// ─── Refresh pool / fee controller / demurrage ───────────────────────

/// One per-namespace credit row inside the refresh pool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RefreshPoolCredit {
    pub namespace_hex: String,
    pub accrued: u64,
    pub last_touched_epoch: u64,
}

/// GET /api/refresh_pool response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RefreshPoolStatus {
    pub total_accrued: u64,
    pub credits: Vec<RefreshPoolCredit>,
}

/// POST /api/fee_controller/step request body.
#[derive(Debug, Clone, Serialize)]
pub struct FeeControllerStepRequest {
    pub gas_used: u64,
    pub epochs_elapsed: u64,
}

/// POST /api/demurrage/owed request body.
#[derive(Debug, Clone, Serialize)]
pub struct DemurrageOwedRequest {
    pub balance: u64,
    pub last_touched_epoch: u64,
    pub current_epoch: u64,
    pub lambda_base_ppm: u64,
    pub threshold: u64,
}

/// POST /api/tx/settle_demurrage request body. ML-DSA signed; canonical
/// signing payload is `JSON({type:"settle_demurrage",from,current_epoch})`
/// — exactly the format consumed by `post_settle_demurrage` in api.rs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettleDemurrageRequest {
    pub from: String,
    pub signature: String,
    pub public_key: String,
}

/// POST /api/tx/settle_demurrage response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SettleDemurrageResponse {
    /// `"settled" | "nothing_owed" | "error"`.
    pub status: String,
    pub settled: u64,
    pub new_balance: u64,
    pub new_last_touched_epoch: u64,
    pub detail: String,
}

// ─── HLWA — Half-Life Wrapped Asset ──────────────────────────────────

/// Shared body for /api/hlwa/effective_supply and /api/hlwa/re_attest.
#[derive(Debug, Clone, Serialize)]
pub struct HlwaEffectiveSupplyRequest {
    pub current_supply: u64,
    pub origin_attested_supply: u64,
    pub last_attested_epoch: u64,
    pub attestation_lambda_epochs: u64,
    pub current_epoch: u64,
}

// ─── DSN — Decay-Stamped Nullifiers ──────────────────────────────────

/// POST /api/dsn/fold_nullifier request body.
#[derive(Debug, Clone, Serialize)]
pub struct DsnFoldRequest {
    pub nullifier_hex: String,
}

/// GET /api/dsn/status response (also returned by fold/advance).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DsnStatus {
    /// `"ok"` for the bare GET; fold/advance also include this field.
    #[serde(default)]
    pub status: Option<String>,
    pub total_count: u64,
    pub aggregate_root_hex: String,
}

// ─── LAD-VM ──────────────────────────────────────────────────────────

/// LAD-VM substructural mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LadMode {
    Linear,
    Affine,
    Decaying,
}

/// LAD-VM action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LadAction {
    Use,
    Drop,
    Tick,
}

/// POST /api/lad_vm/simulate request body.
#[derive(Debug, Clone, Serialize)]
pub struct LadSimulateRequest {
    pub mode: LadMode,
    pub value: u64,
    pub created_at_epoch: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decay_window: Option<u64>,
    pub current_epoch: u64,
    pub action: LadAction,
}

/// POST /api/lad_vm/simulate response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LadSimulateResponse {
    pub status: String,
    pub action: String,
    pub mode: String,
    pub outcome: String,
    pub returned_value: Option<u64>,
    pub is_evaporated_at_query: bool,
    pub created_at_epoch: u64,
    pub current_epoch: u64,
    pub decay_window: Option<u64>,
    pub detail: String,
}

// ─── Bell-Beacon ─────────────────────────────────────────────────────

/// GET /api/bell/latest response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BellBeaconLatest {
    /// `"ok" | "no_data" | "error"`.
    pub status: String,
    pub s_value_milli: u64,
    pub threshold_milli: u64,
    pub bell_certified: bool,
    pub block_height: u64,
    pub epoch: u64,
    pub detail: String,
}

/// POST /api/bell_beacon request body (worked-example simulator).
#[derive(Debug, Clone, Serialize)]
pub struct BellBeaconRequest {
    pub e_ab: i64,
    pub e_ab_prime: i64,
    pub e_a_prime_b: i64,
    pub e_a_prime_b_prime: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold_milli: Option<u64>,
}

/// POST /api/bell_beacon response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BellBeaconResponse {
    pub status: String,
    pub s_value_milli: u64,
    pub threshold_milli: u64,
    pub bell_certified: bool,
    pub detail: String,
}

// ──────────────────────────── RPC Client ───────────────────────────────

/// Async HTTP client for the EvaporChain node REST API.
pub struct RpcClient {
    base_url: Url,
    client: Client,
    auth_token: Option<String>,
    rate_limiter: Option<crate::rate_limit::RateLimiter>,
}

impl RpcClient {
    /// Create a new RPC client pointing to the given node URL.
    pub fn new(base_url: &str) -> Result<Self, RpcError> {
        let url = Url::parse(base_url)?;
        Ok(Self {
            base_url: url,
            client: Client::new(),
            auth_token: None,
            rate_limiter: None,
        })
    }

    /// Create a new RPC client with rate limiting enabled.
    pub fn with_rate_limit(
        base_url: &str,
        requests_per_second: f64,
        burst: u32,
    ) -> Result<Self, RpcError> {
        let url = Url::parse(base_url)?;
        Ok(Self {
            base_url: url,
            client: Client::new(),
            auth_token: None,
            rate_limiter: Some(crate::rate_limit::RateLimiter::new(
                requests_per_second,
                burst,
            )),
        })
    }

    /// Enable default rate limiting (10 req/s, burst 20).
    pub fn enable_rate_limit(&mut self) {
        self.rate_limiter = Some(crate::rate_limit::RateLimiter::default_rpc());
    }

    /// Disable rate limiting.
    pub fn disable_rate_limit(&mut self) {
        self.rate_limiter = None;
    }

    /// Set the auth token for authenticated requests.
    pub fn set_auth_token(&mut self, token: String) {
        self.auth_token = Some(token);
    }

    /// Clear the auth token.
    pub fn clear_auth_token(&mut self) {
        self.auth_token = None;
    }

    /// Get the base URL.
    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    // ── Internal helpers ───────────────────────────────────────────────

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.as_str().trim_end_matches('/'), path)
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, RpcError> {
        if let Some(ref limiter) = self.rate_limiter {
            limiter.acquire().await;
        }
        let resp = self.client.get(self.url(path)).send().await?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(RpcError::Api {
                status,
                message: body,
            });
        }
        resp.json::<T>()
            .await
            .map_err(|e| RpcError::Deserialize(e.to_string()))
    }

    async fn post<R: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &R,
    ) -> Result<T, RpcError> {
        if let Some(ref limiter) = self.rate_limiter {
            limiter.acquire().await;
        }
        let mut req = self.client.post(self.url(path)).json(body);
        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() && status != 200 {
            let body = resp.text().await.unwrap_or_default();
            return Err(RpcError::Api {
                status,
                message: body,
            });
        }
        resp.json::<T>()
            .await
            .map_err(|e| RpcError::Deserialize(e.to_string()))
    }

    // ── Chain queries ──────────────────────────────────────────────────

    /// Get chain status (block height, epoch, object counts, etc.).
    pub async fn get_status(&self) -> Result<StatusResponse, RpcError> {
        self.get("/api/status").await
    }

    /// Get chain metadata (chain_id, consensus, signature scheme).
    pub async fn get_chain_info(&self) -> Result<ChainInfoResponse, RpcError> {
        self.get("/api/chain").await
    }

    /// Get network info (peer count).
    pub async fn get_network(&self) -> Result<NetworkResponse, RpcError> {
        self.get("/api/network").await
    }

    /// Get mempool info (pending tx count).
    pub async fn get_mempool(&self) -> Result<MempoolResponse, RpcError> {
        self.get("/api/mempool").await
    }

    /// Get stats summary (total created, evaporated, resurrected, etc.).
    pub async fn get_stats(&self) -> Result<StatsSummaryResponse, RpcError> {
        self.get("/api/stats/summary").await
    }

    /// Get stats timeline (epoch snapshots).
    pub async fn get_stats_timeline(&self) -> Result<StatsTimelineResponse, RpcError> {
        self.get("/api/stats/timeline").await
    }

    /// Get recent events.
    pub async fn get_events(&self, limit: Option<usize>) -> Result<EventsResponse, RpcError> {
        let path = match limit {
            Some(l) => format!("/api/events?limit={}", l),
            None => "/api/events".to_string(),
        };
        self.get(&path).await
    }

    // ── Block queries ──────────────────────────────────────────────────

    /// Get recent blocks.
    pub async fn get_blocks(&self, limit: Option<usize>) -> Result<Vec<BlockRecord>, RpcError> {
        let path = match limit {
            Some(l) => format!("/api/blocks?limit={}", l),
            None => "/api/blocks".to_string(),
        };
        self.get(&path).await
    }

    /// Get a specific block by number.
    pub async fn get_block(&self, number: u64) -> Result<BlockRecord, RpcError> {
        self.get(&format!("/api/block/{}", number)).await
    }

    /// Get the latest block.
    pub async fn get_latest_block(&self) -> Result<BlockRecord, RpcError> {
        self.get("/api/blocks/latest").await
    }

    // ── Account queries ────────────────────────────────────────────────

    /// Get all accounts.
    pub async fn get_accounts(&self) -> Result<Vec<AccountResponse>, RpcError> {
        self.get("/api/accounts").await
    }

    /// Get full address detail (balance, nonce, objects, NFTs, tokens).
    pub async fn get_address_detail(
        &self,
        address: &str,
    ) -> Result<AddressDetailResponse, RpcError> {
        self.get(&format!("/api/address/{}", address)).await
    }

    // ── Object queries ─────────────────────────────────────────────────

    /// Get all state objects.
    pub async fn get_objects(&self) -> Result<Vec<ObjectResponse>, RpcError> {
        self.get("/api/objects").await
    }

    /// Get a single state object by ID.
    pub async fn get_object(&self, id: &str) -> Result<ObjectResponse, RpcError> {
        self.get(&format!("/api/object/{}", id)).await
    }

    /// Get all ghost (evaporated) records.
    pub async fn get_ghosts(&self) -> Result<Vec<GhostResponse>, RpcError> {
        self.get("/api/ghosts").await
    }

    // ── Transaction queries ────────────────────────────────────────────

    /// Get paginated transactions with optional filters.
    pub async fn get_transactions(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
        tx_type: Option<&str>,
        address: Option<&str>,
    ) -> Result<TransactionsResponse, RpcError> {
        let mut params = Vec::new();
        if let Some(l) = limit {
            params.push(format!("limit={}", l));
        }
        if let Some(o) = offset {
            params.push(format!("offset={}", o));
        }
        if let Some(t) = tx_type {
            params.push(format!("type={}", t));
        }
        if let Some(a) = address {
            params.push(format!("address={}", a));
        }
        let path = if params.is_empty() {
            "/api/transactions".to_string()
        } else {
            format!("/api/transactions?{}", params.join("&"))
        };
        self.get(&path).await
    }

    /// Get a transaction's lifecycle status by hash.
    ///
    /// Returns the new [`TxStatus`] shape (post commit `d0394b1`): hash +
    /// `pending | mempool | included | finalised | rejected` state, with
    /// optional `block_height` / `epoch` once the tx has been included.
    /// Does NOT return the tx body — use [`RpcClient::get_transactions`]
    /// (the historical list) if you need `from`/`to`/`amount`/etc. for a
    /// given hash.
    pub async fn get_tx(&self, hash: &str) -> Result<TxStatus, RpcError> {
        self.get(&format!("/api/tx/{}", hash)).await
    }

    // ── Transaction submission ─────────────────────────────────────────

    /// Submit a transfer transaction.
    pub async fn submit_transfer(
        &self,
        req: &TransferRequest,
    ) -> Result<TxResultResponse, RpcError> {
        self.post("/api/tx/transfer", req).await
    }

    /// Submit a `UserOpTx` (potentially paymaster-sponsored) for
    /// inclusion in the mempool. Pairs with the `evaporchain-paymaster`
    /// service: the wallet builds a `UserOpTx`, asks a paymaster to
    /// stamp its sponsorship sig via `PaymasterClient::sponsor`, signs
    /// the user side via `wallet::paymaster::sign_user_op_as_sender`,
    /// then calls this method to submit.
    ///
    /// Heavy validation (signature verification, nonce match,
    /// paymaster balance, inner call_data whitelist) happens at execute
    /// time. The wallet should poll `GET /api/tx/<hash>` to learn the
    /// verdict — `tx_hash` in the response is the canonical
    /// `blake3(signable_bytes)` value the chain indexes by.
    pub async fn submit_user_op(
        &self,
        user_op: &evaporchain_types::UserOpTx,
    ) -> Result<TxResultResponse, RpcError> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            user_op: &'a evaporchain_types::UserOpTx,
        }
        self.post("/api/tx/user_op", &Body { user_op }).await
    }

    /// Submit a create-object transaction.
    pub async fn submit_create_object(
        &self,
        req: &CreateObjectRequest,
    ) -> Result<TxResultResponse, RpcError> {
        self.post("/api/tx/create-object", req).await
    }

    /// Submit a refresh transaction.
    pub async fn submit_refresh(&self, req: &RefreshRequest) -> Result<TxResultResponse, RpcError> {
        self.post("/api/tx/refresh", req).await
    }

    /// Submit a resurrect transaction (same shape as refresh).
    pub async fn submit_resurrect(
        &self,
        req: &RefreshRequest,
    ) -> Result<TxResultResponse, RpcError> {
        self.post("/api/tx/resurrect", req).await
    }

    /// Submit a deploy-contract transaction.
    pub async fn submit_deploy_contract(
        &self,
        req: &DeployContractRequest,
    ) -> Result<TxResultResponse, RpcError> {
        self.post("/api/tx/deploy-contract", req).await
    }

    /// Submit a call-contract transaction.
    pub async fn submit_call_contract(
        &self,
        req: &CallContractRequest,
    ) -> Result<TxResultResponse, RpcError> {
        self.post("/api/tx/call-contract", req).await
    }

    // ── NFT operations ─────────────────────────────────────────────────

    /// Get all NFTs.
    pub async fn get_nfts(&self) -> Result<Vec<NftResponse>, RpcError> {
        self.get("/api/nfts").await
    }

    /// Get a single NFT by ID.
    pub async fn get_nft(&self, id: u64) -> Result<NftResponse, RpcError> {
        self.get(&format!("/api/nft/{}", id)).await
    }

    /// Get NFT collections.
    pub async fn get_nft_collections(&self) -> Result<Vec<NftCollection>, RpcError> {
        self.get("/api/nft/collections").await
    }

    /// Mint a new NFT (requires auth token).
    pub async fn mint_nft(&self, req: &MintNftRequest) -> Result<TxResultResponse, RpcError> {
        self.post("/api/nft/mint", req).await
    }

    /// Transfer an NFT (requires auth token).
    pub async fn transfer_nft(
        &self,
        req: &TransferNftRequest,
    ) -> Result<TxResultResponse, RpcError> {
        self.post("/api/nft/transfer", req).await
    }

    /// Refresh an NFT's energy (requires auth token).
    pub async fn refresh_nft(&self, req: &RefreshNftRequest) -> Result<TxResultResponse, RpcError> {
        self.post("/api/nft/refresh", req).await
    }

    // ── Token operations ───────────────────────────────────────────────

    /// Get all tokens.
    pub async fn get_tokens(&self) -> Result<Vec<TokenResponse>, RpcError> {
        self.get("/api/tokens").await
    }

    /// Get a single token by ID.
    pub async fn get_token(&self, id: u64) -> Result<TokenResponse, RpcError> {
        self.get(&format!("/api/token/{}", id)).await
    }

    /// Deploy a new token (requires auth token).
    pub async fn deploy_token(
        &self,
        req: &DeployTokenRequest,
    ) -> Result<TxResultResponse, RpcError> {
        self.post("/api/token/deploy", req).await
    }

    /// Transfer tokens (requires auth token).
    pub async fn transfer_token(
        &self,
        req: &TokenTransferRequest,
    ) -> Result<TxResultResponse, RpcError> {
        self.post("/api/token/transfer", req).await
    }

    /// Check token balance.
    pub async fn token_balance(
        &self,
        req: &TokenBalanceRequest,
    ) -> Result<serde_json::Value, RpcError> {
        self.post("/api/token/balance", req).await
    }

    // ── Contracts ──────────────────────────────────────────────────────

    /// Get all contracts.
    pub async fn get_contracts(&self) -> Result<ContractsResponse, RpcError> {
        self.get("/api/contracts").await
    }

    /// Get a single contract by ID.
    pub async fn get_contract(&self, id: u64) -> Result<serde_json::Value, RpcError> {
        self.get(&format!("/api/contract/{}", id)).await
    }

    // ── Staking ────────────────────────────────────────────────────────

    /// Get all staking pools.
    pub async fn get_staking_pools(&self) -> Result<Vec<StakingPoolResponse>, RpcError> {
        self.get("/api/staking/pools").await
    }

    /// Get a single staking pool by ID.
    pub async fn get_staking_pool(&self, id: u64) -> Result<StakingPoolResponse, RpcError> {
        self.get(&format!("/api/staking/pool/{}", id)).await
    }

    /// Stake tokens in a pool (requires auth token).
    pub async fn stake(&self, req: &StakeRequest) -> Result<TxResultResponse, RpcError> {
        self.post("/api/staking/stake", req).await
    }

    /// Unstake tokens from a pool (requires auth token).
    pub async fn unstake(&self, req: &UnstakeRequest) -> Result<TxResultResponse, RpcError> {
        self.post("/api/staking/unstake", req).await
    }

    /// Claim staking rewards (requires auth token).
    pub async fn claim_rewards(&self, req: &ClaimRequest) -> Result<TxResultResponse, RpcError> {
        self.post("/api/staking/claim", req).await
    }

    // ── DAO / Governance ───────────────────────────────────────────────

    /// Get all DAO proposals.
    pub async fn get_proposals(&self) -> Result<Vec<ProposalResponse>, RpcError> {
        self.get("/api/dao/proposals").await
    }

    /// Get a single proposal by ID.
    pub async fn get_proposal(&self, id: u64) -> Result<ProposalResponse, RpcError> {
        self.get(&format!("/api/dao/proposal/{}", id)).await
    }

    /// Create a new DAO proposal (requires auth token).
    pub async fn create_proposal(
        &self,
        req: &ProposeRequest,
    ) -> Result<TxResultResponse, RpcError> {
        self.post("/api/dao/propose", req).await
    }

    /// Vote on a DAO proposal (requires auth token).
    pub async fn vote(&self, req: &VoteRequest) -> Result<TxResultResponse, RpcError> {
        self.post("/api/dao/vote", req).await
    }

    // ── Faucet ─────────────────────────────────────────────────────────

    /// Request testnet tokens from the faucet.
    pub async fn faucet(&self, address: &str) -> Result<FaucetResponse, RpcError> {
        let req = FaucetRequest {
            address: address.to_string(),
        };
        self.post("/api/faucet", &req).await
    }

    // ── Patronage covenants ────────────────────────────────────────────

    /// GET /api/patronage/status — summary of all active covenants.
    pub async fn get_patronage_status(&self) -> Result<PatronageStatus, RpcError> {
        self.get("/api/patronage/status").await
    }

    /// GET /api/patronage/immune?object_id_hex=&epoch= — eviction-immunity
    /// and patronage-score query for a single object at an epoch.
    pub async fn get_patronage_immune(
        &self,
        object_id_hex: &str,
        epoch: u64,
    ) -> Result<PatronageImmunity, RpcError> {
        self.get(&format!(
            "/api/patronage/immune?object_id_hex={}&epoch={}",
            object_id_hex, epoch
        ))
        .await
    }

    /// POST /api/patronage/pledge — create a Patronage Covenant for an object.
    pub async fn submit_patronage_pledge(
        &self,
        req: &PatronagePledgeRequest,
    ) -> Result<PatronagePledgeResponse, RpcError> {
        self.post("/api/patronage/pledge", req).await
    }

    /// POST /api/patronage/honour — release one epoch's donation from a covenant.
    pub async fn submit_patronage_honour(
        &self,
        req: &PatronageActionRequest,
    ) -> Result<PatronageHonourResponse, RpcError> {
        self.post("/api/patronage/honour", req).await
    }

    /// POST /api/patronage/revoke — remove a covenant early; refunds unused
    /// pre-funded surplus back to the namespace pool credit. Node returns a
    /// free-form JSON envelope (`status`, `refunded`, etc.).
    pub async fn submit_patronage_revoke(
        &self,
        req: &PatronageActionRequest,
    ) -> Result<serde_json::Value, RpcError> {
        self.post("/api/patronage/revoke", req).await
    }

    // ── Governance fork-choice ────────────────────────────────────────

    /// GET /api/governance/fork_choice_mode — current fork-choice mode +
    /// attractor set. Returns the raw envelope so callers can read both
    /// `fork_choice_mode` and the `attractors` array.
    pub async fn get_fork_choice_mode(&self) -> Result<serde_json::Value, RpcError> {
        self.get("/api/governance/fork_choice_mode").await
    }

    /// POST /api/governance/fork_choice_mode — governance amendment to
    /// switch fork-choice between MCC and Singh-Attractor. Authorised by
    /// stake quorum (`endorser_stakes` summing to ≥ `required_stake`).
    pub async fn submit_fork_choice_amend(
        &self,
        req: &ForkChoiceAmendRequest,
    ) -> Result<serde_json::Value, RpcError> {
        self.post("/api/governance/fork_choice_mode", req).await
    }

    // ── Refresh pool / fee controller / demurrage ─────────────────────

    /// GET /api/refresh_pool — protocol-owned refresh pool total +
    /// per-namespace credits.
    pub async fn get_refresh_pool(&self) -> Result<RefreshPoolStatus, RpcError> {
        self.get("/api/refresh_pool").await
    }

    /// GET /api/fee_controller/status — Lyapunov fee-controller state.
    pub async fn get_fee_controller_status(&self) -> Result<serde_json::Value, RpcError> {
        self.get("/api/fee_controller/status").await
    }

    /// POST /api/fee_controller/step — advance the fee controller by one
    /// block (compute helper; mutates server-side fee state).
    pub async fn submit_fee_controller_step(
        &self,
        req: &FeeControllerStepRequest,
    ) -> Result<serde_json::Value, RpcError> {
        self.post("/api/fee_controller/step", req).await
    }

    /// POST /api/demurrage/owed — pure compute: how much demurrage an
    /// idle balance owes at `current_epoch`.
    pub async fn compute_demurrage_owed(
        &self,
        req: &DemurrageOwedRequest,
    ) -> Result<serde_json::Value, RpcError> {
        self.post("/api/demurrage/owed", req).await
    }

    /// POST /api/tx/settle_demurrage — debit `owed` from `from`'s balance
    /// and credit it to the refresh pool. Body must already carry an
    /// ML-DSA signature over the canonical signing payload — use
    /// [`crate::tx_builder::TxBuilder::build_settle_demurrage_request`]
    /// to produce one.
    pub async fn submit_settle_demurrage(
        &self,
        req: &SettleDemurrageRequest,
    ) -> Result<SettleDemurrageResponse, RpcError> {
        self.post("/api/tx/settle_demurrage", req).await
    }

    // ── HLWA — Half-Life Wrapped Asset ────────────────────────────────

    /// POST /api/hlwa/effective_supply — read-only simulation of effective
    /// wrapped-asset supply after λ-decay of attestation freshness.
    pub async fn hlwa_effective_supply(
        &self,
        req: &HlwaEffectiveSupplyRequest,
    ) -> Result<serde_json::Value, RpcError> {
        self.post("/api/hlwa/effective_supply", req).await
    }

    /// POST /api/hlwa/re_attest — read-only simulation of a re-attestation
    /// from the origin chain.
    pub async fn hlwa_re_attest(
        &self,
        req: &HlwaEffectiveSupplyRequest,
    ) -> Result<serde_json::Value, RpcError> {
        self.post("/api/hlwa/re_attest", req).await
    }

    // ── DSN — Decay-Stamped Nullifiers ────────────────────────────────

    /// GET /api/dsn/status — current sliding-window total + aggregate root.
    pub async fn get_dsn_status(&self) -> Result<DsnStatus, RpcError> {
        self.get("/api/dsn/status").await
    }

    /// POST /api/dsn/fold_nullifier — fold a 32-byte nullifier hex into
    /// the current DSN window accumulator.
    pub async fn submit_dsn_fold_nullifier(
        &self,
        req: &DsnFoldRequest,
    ) -> Result<DsnStatus, RpcError> {
        self.post("/api/dsn/fold_nullifier", req).await
    }

    /// POST /api/dsn/advance_window — advance the sliding window by one
    /// slot, dropping the oldest accumulator. No body.
    pub async fn submit_dsn_advance_window(&self) -> Result<DsnStatus, RpcError> {
        self.post("/api/dsn/advance_window", &serde_json::json!({}))
            .await
    }

    // ── LAD-VM ────────────────────────────────────────────────────────

    /// POST /api/lad_vm/simulate — single LAD-VM resource-lifecycle step.
    pub async fn lad_vm_simulate(
        &self,
        req: &LadSimulateRequest,
    ) -> Result<LadSimulateResponse, RpcError> {
        self.post("/api/lad_vm/simulate", req).await
    }

    // ── Bell-Beacon ───────────────────────────────────────────────────

    /// GET /api/bell/latest — most recent live VRF-derived CHSH S-value.
    pub async fn get_bell_latest(&self) -> Result<BellBeaconLatest, RpcError> {
        self.get("/api/bell/latest").await
    }

    /// POST /api/bell_beacon — worked-example CHSH S-value + Bell
    /// certification simulator.
    pub async fn bell_beacon_simulate(
        &self,
        req: &BellBeaconRequest,
    ) -> Result<BellBeaconResponse, RpcError> {
        self.post("/api/bell_beacon", req).await
    }
}

// ──────────────────────────── Tests ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = RpcClient::new("http://localhost:3000").unwrap();
        assert_eq!(client.base_url(), "http://localhost:3000/");
    }

    #[test]
    fn test_client_invalid_url() {
        let result = RpcClient::new("not a url");
        assert!(result.is_err());
    }

    #[test]
    fn test_url_building() {
        let client = RpcClient::new("http://localhost:3000").unwrap();
        assert_eq!(
            client.url("/api/status"),
            "http://localhost:3000/api/status"
        );
    }

    #[test]
    fn test_url_building_trailing_slash() {
        let client = RpcClient::new("http://localhost:3000/").unwrap();
        assert_eq!(
            client.url("/api/status"),
            "http://localhost:3000/api/status"
        );
    }

    #[test]
    fn test_auth_token_management() {
        let mut client = RpcClient::new("http://localhost:3000").unwrap();
        assert!(client.auth_token.is_none());

        client.set_auth_token("test_token_123".to_string());
        assert_eq!(client.auth_token.as_deref(), Some("test_token_123"));

        client.clear_auth_token();
        assert!(client.auth_token.is_none());
    }

    #[test]
    fn test_deserialize_status_response() {
        let json = r#"{
            "chain_name": "EvaporChain",
            "version": "0.2.0",
            "block_height": 42,
            "epoch": 42,
            "active_objects": 10,
            "ghost_count": 3,
            "total_evaporated": 5,
            "peer_count": 4,
            "state_root": "0xabc",
            "proving_enabled": true,
            "uptime_seconds": 3600
        }"#;
        let resp: StatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.block_height, 42);
        assert_eq!(resp.active_objects, 10);
        assert!(resp.proving_enabled);
    }

    #[test]
    fn test_deserialize_account_response() {
        let json = r#"{
            "address": "0xabcd",
            "name": "Alice",
            "balance": 100000,
            "nonce": 5
        }"#;
        let resp: AccountResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.balance, 100000);
        assert_eq!(resp.nonce, 5);
    }

    #[test]
    fn test_deserialize_address_detail_response() {
        let json = r#"{
            "address": "0xabcd",
            "balance": 50000,
            "nonce": 2,
            "objects": [],
            "nfts": [],
            "tokens": [{"token_id": 1, "symbol": "EVAP", "name": "EvaporToken", "balance": 1000}]
        }"#;
        let resp: AddressDetailResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.balance, 50000);
        assert_eq!(resp.tokens.len(), 1);
        assert_eq!(resp.tokens[0].symbol, "EVAP");
    }

    #[test]
    fn test_deserialize_object_response() {
        let json = r#"{
            "id": "0xcc00",
            "name": "test-obj",
            "owner": "0xaa00",
            "owner_name": "Alice",
            "energy": 5000,
            "max_energy": 5000,
            "half_life": 100,
            "state": "Active",
            "created_epoch": 0,
            "last_refreshed": 0,
            "grace_epoch": null,
            "current_energy": 4500,
            "decay_percentage": 10.0
        }"#;
        let resp: ObjectResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.state, "Active");
        assert_eq!(resp.current_energy, 4500);
        assert!(resp.grace_epoch.is_none());
    }

    #[test]
    fn test_deserialize_ghost_response() {
        let json = r#"{
            "id": "0xdead",
            "original_owner": "0xaa00",
            "evaporated_epoch": 50,
            "data_hash": "0xbeef"
        }"#;
        let resp: GhostResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.evaporated_epoch, 50);
    }

    #[test]
    fn test_deserialize_block_record() {
        let json = r#"{
            "number": 10,
            "epoch": 10,
            "parent_hash": "0x00",
            "state_root": "0xab",
            "tx_count": 3,
            "evaporations": 1,
            "entered_grace": 0,
            "timestamp": 1700000000,
            "active_objects": 8,
            "ghost_count": 2,
            "gas_used": 150000,
            "base_fee": 100,
            "total_fees": 300,
            "transactions": []
        }"#;
        let resp: BlockRecord = serde_json::from_str(json).unwrap();
        assert_eq!(resp.number, 10);
        assert_eq!(resp.tx_count, 3);
    }

    #[test]
    fn test_deserialize_tx_record() {
        // TxRecord is now only emitted from /api/transactions (the
        // historical list), not /api/tx/:hash.
        let json = r#"{
            "hash": "0xabc123",
            "type": "transfer",
            "from": "0xaa",
            "to": "0xbb",
            "amount": 1000,
            "gas": 21000,
            "block_number": 5,
            "epoch": 5,
            "status": "success"
        }"#;
        let resp: TxRecord = serde_json::from_str(json).unwrap();
        assert_eq!(resp.tx_type, "transfer");
        assert_eq!(resp.amount, Some(1000));
        assert!(resp.object_id.is_none());
    }

    #[test]
    fn test_deserialize_tx_status_finalised() {
        // GET /api/tx/:hash new shape — finalised tx carries
        // block_height + epoch.
        let json = r#"{
            "hash": "abc123",
            "state": "finalised",
            "block_height": 42,
            "epoch": 42
        }"#;
        let resp: TxStatus = serde_json::from_str(json).unwrap();
        assert_eq!(resp.hash, "abc123");
        assert_eq!(resp.state, TxState::Finalised);
        assert_eq!(resp.block_height, Some(42));
        assert_eq!(resp.epoch, Some(42));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_deserialize_tx_status_pending() {
        // pending / mempool — no block_height, no epoch, no error.
        let json = r#"{"hash": "deadbeef", "state": "pending"}"#;
        let resp: TxStatus = serde_json::from_str(json).unwrap();
        assert_eq!(resp.state, TxState::Pending);
        assert!(resp.block_height.is_none());
        assert!(resp.epoch.is_none());
    }

    #[test]
    fn test_deserialize_tx_status_rejected() {
        let json = r#"{
            "hash": "abc",
            "state": "rejected",
            "block_height": 7,
            "epoch": 7,
            "error": "insufficient balance"
        }"#;
        let resp: TxStatus = serde_json::from_str(json).unwrap();
        assert_eq!(resp.state, TxState::Rejected);
        assert_eq!(resp.error.as_deref(), Some("insufficient balance"));
    }

    #[test]
    fn test_tx_state_lowercase_roundtrip() {
        // serde(rename_all = "lowercase") on TxState — strings on the
        // wire match the node's `pending|mempool|included|finalised|rejected`.
        for (s, st) in [
            ("\"pending\"", TxState::Pending),
            ("\"mempool\"", TxState::Mempool),
            ("\"included\"", TxState::Included),
            ("\"finalised\"", TxState::Finalised),
            ("\"rejected\"", TxState::Rejected),
        ] {
            let parsed: TxState = serde_json::from_str(s).unwrap();
            assert_eq!(parsed, st);
            assert_eq!(serde_json::to_string(&st).unwrap(), s);
        }
    }

    #[test]
    fn test_deserialize_nft_response() {
        let json = r#"{
            "id": 1,
            "name": "CoolNFT",
            "collection": "Art",
            "owner": "0xaa",
            "metadata_hash": "0xmeta",
            "energy": 10000,
            "max_energy": 10000,
            "current_energy": 8000,
            "half_life": 200,
            "minted_epoch": 5,
            "last_refreshed": 5,
            "state": "Active",
            "decay_percentage": 20.0,
            "epochs_remaining": 150,
            "grace_epoch": null,
            "evaporated_epoch": null,
            "ghost_proof": null
        }"#;
        let resp: NftResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.name, "CoolNFT");
        assert_eq!(resp.current_energy, 8000);
        assert!(resp.evaporated_epoch.is_none());
    }

    #[test]
    fn test_deserialize_token_response() {
        let json = r#"{
            "id": 1,
            "name": "EvaporToken",
            "symbol": "EVAP",
            "total_supply": 1000000,
            "current_supply": 900000,
            "decay_half_life": 500,
            "deployed_epoch": 0,
            "deployer": "0xaa",
            "decay_percentage": 10.0,
            "holder_count": 2,
            "holders": [
                {"address": "0xaa", "balance": 600000},
                {"address": "0xbb", "balance": 300000}
            ]
        }"#;
        let resp: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.symbol, "EVAP");
        assert_eq!(resp.holders.len(), 2);
    }

    #[test]
    fn test_deserialize_staking_pool_response() {
        let json = r#"{
            "id": 1,
            "name": "Main Pool",
            "reward_rate": 100,
            "reward_decay_hl": 1000,
            "total_staked": 500000,
            "created_epoch": 0,
            "staker_count": 1,
            "stakers": [{
                "address": "0xaa",
                "amount": 500000,
                "staked_epoch": 10,
                "pending_rewards": 250,
                "last_claim_epoch": 10,
                "total_claimed": 0,
                "total_decayed": 0,
                "reward_decay_pct": 0.0
            }]
        }"#;
        let resp: StakingPoolResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.name, "Main Pool");
        assert_eq!(resp.stakers[0].pending_rewards, 250);
    }

    #[test]
    fn test_deserialize_proposal_response() {
        let json = r#"{
            "id": 1,
            "title": "Increase block size",
            "description": "Proposal to increase max block size",
            "options": ["Yes", "No"],
            "created_epoch": 100,
            "voting_period": 50,
            "end_epoch": 150,
            "creator": "0xaa",
            "status": "Active",
            "total_votes": 10,
            "vote_totals": {"Yes": 7, "No": 3},
            "epochs_remaining": 25,
            "evaporated_epoch": null,
            "voter_count": 5
        }"#;
        let resp: ProposalResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.title, "Increase block size");
        assert_eq!(*resp.vote_totals.get("Yes").unwrap(), 7);
    }

    #[test]
    fn test_deserialize_tx_result_response() {
        let json = r#"{"success": true, "message": "Transfer queued", "tx_hash": "0xabc"}"#;
        let resp: TxResultResponse = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert_eq!(resp.tx_hash.as_deref(), Some("0xabc"));

        let json_fail = r#"{"success": false, "message": "Insufficient balance"}"#;
        let resp_fail: TxResultResponse = serde_json::from_str(json_fail).unwrap();
        assert!(!resp_fail.success);
        assert!(resp_fail.tx_hash.is_none());
    }

    #[test]
    fn test_deserialize_faucet_response() {
        let json = r#"{"success": true, "balance": 10000}"#;
        let resp: FaucetResponse = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert_eq!(resp.balance, 10000);
        assert!(resp.message.is_none());
    }

    #[test]
    fn test_serialize_transfer_request() {
        let req = TransferRequest {
            from: "0xaa".to_string(),
            to: "0xbb".to_string(),
            amount: 1000,
            nonce: 0,
            signature: Some("0xsig".to_string()),
            public_key: Some("0xpk".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["amount"], 1000);
        assert_eq!(json["signature"], "0xsig");
    }

    #[test]
    fn test_serialize_transfer_request_unsigned() {
        let req = TransferRequest {
            from: "0xaa".to_string(),
            to: "0xbb".to_string(),
            amount: 500,
            nonce: 1,
            signature: None,
            public_key: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("signature").is_none());
        assert!(json.get("public_key").is_none());
    }

    #[test]
    fn test_deserialize_transactions_response() {
        let json = r#"{
            "transactions": [],
            "total": 0,
            "limit": 50,
            "offset": 0
        }"#;
        let resp: TransactionsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total, 0);
        assert_eq!(resp.limit, 50);
    }

    #[test]
    fn test_deserialize_stats_summary() {
        let json = r#"{
            "total_created": 100,
            "total_evaporated": 30,
            "total_resurrected": 5,
            "total_refreshed": 20,
            "avg_lifetime_epochs": 42.5,
            "total_transactions": 200
        }"#;
        let resp: StatsSummaryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total_created, 100);
        assert_eq!(resp.avg_lifetime_epochs, 42.5);
    }
}

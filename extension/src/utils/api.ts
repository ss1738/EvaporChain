/**
 * EvaporChain node API client for the browser extension.
 * Wraps the REST API endpoints used by the wallet.
 */

const DEFAULT_NODE = "https://testnet.evaporchain.com";

export interface ChainStatus {
  chain_name: string;
  version: string;
  block_height: number;
  epoch: number;
  active_objects: number;
  ghost_count: number;
  total_evaporated: number;
  peer_count: number;
  uptime_seconds: number;
}

export interface AccountDetail {
  address: string;
  name: string;
  balance: number;
  nonce: number;
  /**
   * Epoch the account was last "touched" by an outbound tx — used as the
   * demurrage anchor. The node currently serves `0` because Account does
   * not yet carry the field; once that lands this will be the real value
   * and `DemurrageBadge` becomes accurate.
   * api.rs §AddressDetailResponse.last_touched_epoch (~L7393).
   */
  last_touched_epoch?: number;
}

export interface StateObject {
  id: string;
  name: string;
  owner: string;
  energy: number;
  max_energy: number;
  half_life: number;
  state: "Active" | "Grace" | "Ghost" | "Risen";
  current_energy: number;
  decay_percentage: number;
  /**
   * Whether this object is governed by the LAD-VM substructural-resource
   * type system (linear/affine/decaying). Backed by
   * `StateObject.lad_mode.is_some()` on the node side. Use this as the
   * cheap boolean gate for showing the "LAD" pill / LAD-VM preview;
   * read `lad_mode` below if you need to discriminate which substructural
   * mode actually applies.
   * api.rs §ObjectResponse.is_lad_typed.
   */
  is_lad_typed?: boolean;
  /**
   * The actual LAD-VM substructural mode when `is_lad_typed === true`.
   * Wire format is the lowercase enum variant; absent (or `null`) for
   * ordinary non-substructural objects.
   * api.rs §ObjectResponse.lad_mode.
   */
  lad_mode?: "linear" | "affine" | "decaying";
}

export interface NftItem {
  id: string;
  name: string;
  collection: string;
  owner: string;
  image_url?: string;
  energy: number;
  max_energy: number;
  current_energy: number;
  half_life: number;
  decay_percentage: number;
  state: "Active" | "Grace" | "Ghost";
  epochs_remaining: number;
  created_epoch: number;
}

export interface TxResult {
  success: boolean;
  message: string;
  hash?: string;
}

// Validator delegation (P0 #4) — wire shapes match
// crates/evaporchain-node/src/api.rs::{DelegateRequest, UndelegateRequest,
// ClaimDelegationRequest, ValidatorDelegationsResponse, DelegationView}.
// Price feed entry — returned by GET /api/prices.
export interface PriceData {
  symbol: string;
  price_usd: number;
  change_24h_pct: number;
}

export interface DelegateRequest {
  delegator: string;
  validatorId: number;
  amount: number;
  nonce: number;
  signature?: string;
  publicKey?: string;
}

export interface UndelegateRequest {
  delegator: string;
  validatorId: number;
  amount: number;
  nonce: number;
  signature?: string;
  publicKey?: string;
}

export interface ClaimDelegationRequest {
  delegator: string;
  validatorId: number;
  nonce: number;
  signature?: string;
  publicKey?: string;
}

export interface DelegationView {
  delegator: string;
  validator_id: number;
  amount: number;
  delegated_at_epoch: number;
  unbonding_amount: number;
  unbonding_epoch: number | null;
}

export interface ValidatorDelegationsResponse {
  validator_id: number;
  delegation_count: number;
  total_delegated: number;
  delegations: DelegationView[];
}

export interface TransactionRecord {
  type: string;
  detail: string;
  hash?: string;
  /**
   * Lifecycle state when this record was emitted by the node:
   * "success" for accepted txs, "rejected" for executor-failed txs.
   * Mirrors `TxRecord.status` (api.rs §TxRecord).
   */
  status?: "success" | "rejected" | string;
  /**
   * Executor error message when `status === "rejected"` — e.g.
   * "InsufficientBalance" / "InvalidNonce". Absent on success.
   * Wired through TxRecord.error from the new field added in this
   * commit (skip_serializing_if = Option::is_none on the Rust side).
   */
  error?: string;
}

export interface SimulateTransactionRequest {
  from: string;
  to: string;
  amount: number;
}

export interface SimulationResult {
  balanceBefore: number;
  balanceAfter: number;
  maxEnergy: number;
  gasCost: number;
  energyCost: number;
  nonce: number;
  estimatedBlock: number;
  recipientIsGhost: boolean;
  objectSurvivalEpochs?: number;
  objectHalfLife?: number;
  objectStateAfter: "Active" | "Grace" | "Ghost" | "Risen";
}

export interface DecayForecastResult {
  objectId: string;
  currentEnergy: number;
  maxEnergy: number;
  halfLife: number;
  currentEpoch: number;
  epochDurationMs: number;
  projectedEnergy: Array<{ epoch: number; energy: number; percent: number }>;
  evaporationEpoch: number;
  evaporationDate: string;
}

export interface TokenInfo {
  symbol: string;
  name: string;
  address: string;
  decimals: number;
  balance: number;
  logo?: string;
}

export interface SwapQuote {
  from_token: string;
  to_token: string;
  amount_in: number;
  amount_out: number;
  rate: number;
  price_impact: number;
  energy_cost: number;
  estimated_fee: number;
}

export interface SwapResult {
  success: boolean;
  message: string;
  hash?: string;
  amount_in: number;
  amount_out: number;
}

export interface RefreshCostEstimate {
  object_id: string;
  current_energy: number;
  target_energy: number;
  energy_needed: number;
  evap_cost: number;
  epochs_extended: number;
}

export interface GhostObject {
  id: string;
  name: string;
  owner: string;
  original_energy: number;
  max_energy: number;
  half_life: number;
  evaporated_epoch: number;
  epochs_since_evaporation: number;
  recovery_cost: number;
  recovery_window_remaining: number;
  recovery_window_total: number;
  proof_status: "valid" | "expiring" | "expired";
}

export interface GhostDetail extends GhostObject {
  created_epoch: number;
  evaporation_date: string;
  mint_date: string;
  merkle_proof: string;
  metadata: Record<string, string>;
  energy_history: Array<{ epoch: number; energy: number; percent: number }>;
}

export interface RecoveryCostEstimate {
  object_id: string;
  base_cost: number;
  decay_penalty: number;
  total_cost: number;
  proof_valid: boolean;
  epochs_until_expiry: number;
}

export interface EnergyPortfolio {
  total_energy: number;
  total_max_energy: number;
  object_count: number;
  ghost_count: number;
  at_risk_count: number;
  expiring_today_count: number;
  energy_trend: number;
  objects_refreshed_this_week: number;
  objects_evaporated_this_week: number;
  total_energy_spent_this_week: number;
  net_energy_change_this_week: number;
  objects: Array<{
    id: string;
    name: string;
    energy: number;
    max_energy: number;
    state: string;
  }>;
}

export interface EnergyHistory {
  address: string;
  epochs: Array<{
    epoch: number;
    energy: number;
  }>;
}

export interface SocialAuthResult {
  success: boolean;
  address: string;
  encrypted_key: string;
  is_new_account: boolean;
  message?: string;
}

// ── Tx status tracking ──────────────────────────────────────────────
//
// GET /api/tx/:hash returns TxStatus directly with one of five states:
// pending → mempool → included → finalised | rejected. Always 200
// except invalid hex (400). The node body is not carried — fetch via
// /api/transactions if needed.

export interface TxStatus {
  hash: string;
  state: "pending" | "mempool" | "included" | "finalised" | "rejected";
  block_height?: number;
  epoch?: number;
  error?: string;
  /** head_height − block_height. ≥6 is the typical "safe" UI gate. */
  confirmations?: number;
  /** Position within the block (only set for chain_store-indexed txs). */
  tx_index?: number;
  /** Gas units consumed, when known. */
  gas_used?: number;
  /** Zero-indexed FIFO position when state === "mempool". */
  mempool_position?: number;
  /** Total mempool depth at lookup time when state === "mempool". */
  mempool_size?: number;
}

// ── Patronage Covenant types ────────────────────────────────────────

export interface PatronagePledgeReq {
  object_id_hex: string;
  namespace_id_hex: string;
  donation_per_epoch: number;
  epochs: number;
  current_epoch: number;
}

export interface PatronageActionReq {
  object_id_hex: string;
  epoch: number;
}

export interface PatronagePledgeResp {
  status: string;          // "pledged" | "error"
  object_id_hex: string;
  pre_funded: number;
  expires_epoch: number;
  detail: string;
}

export interface PatronageHonourResp {
  status: string;          // "honoured" | "error"
  donated: number;
  patronage_score: number;
  detail: string;
}

export interface PatronageRevokeResp {
  status: string;          // "revoked" | "error"
  object_id_hex?: string;
  patronage_score_archived?: number;
  refunded?: number;
  detail: string;
}

export interface PatronageStatusResp {
  active_covenants: number;
  total_pre_funded: number;
  total_active_score: number;
  patronage_ns_hex: string;
}

export interface PatronageImmuneResp {
  object_id_hex: string;
  epoch: number;
  immune: boolean;
  patronage_score: number;
}

// ── Refresh pool ────────────────────────────────────────────────────
//
// GET /api/refresh_pool — protocol-owned pool that funds object refreshes,
// fed by demurrage + patronage payments. Per-namespace credits track which
// namespace has accrued how much and when it was last touched.
// api.rs §RefreshPoolResp / RefreshPoolCredit (~L2823-2834).

export interface RefreshPoolCredit {
  namespace_hex: string;
  accrued: number;
  last_touched_epoch: number;
}

export interface RefreshPoolStatus {
  total_accrued: number;
  credits: RefreshPoolCredit[];
}

// ── Fee controller status ──────────────────────────────────────────
//
// GET /api/fee_controller/status — current Lyapunov-stable fee controller
// state. `energy` is the accumulated energy of the controller; `base_fee`
// is the fee at the current pressure; `target_*`/`fee_response_ppm` are
// the genesis target params used to evaluate convergence.
// api.rs §get_fee_controller_status (~L4672) — returns serde_json::Value
// with fields: status, energy, base_fee, target_energy, target_gas,
// fee_response_ppm.

export interface FeeControllerStatus {
  status: string;
  energy: number;
  base_fee: number;
  target_energy: number;
  target_gas: number;
  fee_response_ppm: number;
}

// One step return from POST /api/fee_controller/step. Used by the widget
// to plot V-function drift trace (delta < 0 = converging).
export interface FeeControllerStepResp {
  status: string;
  energy_after?: number;
  base_fee?: number;
  lyapunov_v_before?: number;
  lyapunov_v_after?: number;
  lyapunov_delta?: number;
  gas_used?: number;
  detail?: string;
}

// ── Demurrage owed ─────────────────────────────────────────────────
//
// POST /api/demurrage/owed — pure compute, no chain mutation. Returns
// `owed` (energy units), `rate_ppm` (per-epoch rate), and the recomputed
// remaining_balance. Note: the response does NOT include a `capped`
// field — it includes `is_disabled` and `remaining_balance` instead.
// api.rs §post_demurrage_owed (~L5366).

export interface DemurrageOwedReq {
  balance: number;
  last_touched_epoch: number;
  current_epoch: number;
  lambda_base_ppm: number;
  threshold: number;
}

export interface DemurrageOwedResp {
  status: string;
  balance: number;
  last_touched_epoch: number;
  current_epoch: number;
  elapsed_epochs: number;
  rate_ppm: number;
  owed: number;
  remaining_balance: number;
  is_disabled: boolean;
}

// POST /api/tx/settle_demurrage — debits owed from the active account's
// balance and credits the protocol-owned refresh pool under namespace
// "DEMU". ML-DSA signature required over the canonical payload
// JSON({type:"settle_demurrage",from,current_epoch}).
// api.rs §post_settle_demurrage / SettleDemurrageReq / SettleDemurrageResp.

export interface SettleDemurrageReq {
  from: string;
  signature: string;
  public_key: string;
}

export interface SettleDemurrageResp {
  status: "settled" | "nothing_owed" | "error";
  settled: number;
  new_balance: number;
  new_last_touched_epoch: number;
  detail: string;
}

// ── Governance fork-choice ─────────────────────────────────────────
//
// GET  /api/governance/fork_choice_mode (~L1951) — current mode + attractors.
// POST /api/governance/fork_choice_mode (~L1912) — stake-quorum amendment.
// The POST does NOT take signature/public_key — it accepts endorser_stakes
// and required_stake; quorum is satisfied when sum(endorser_stakes) >=
// required_stake. The mode is "mcc" or "singh_attractor".

export interface ForkChoiceAttractor {
  center: number;
  basin_radius: number;
}

export interface ForkChoiceModeStatus {
  fork_choice_mode: string;
  attractors: ForkChoiceAttractor[];
  detail: string;
}

export interface ForkChoiceAmendReq {
  mode: string;
  attractors?: ForkChoiceAttractor[];
  endorser_stakes: number[];
  required_stake: number;
}

export interface ForkChoiceAmendResp {
  status: string;             // "amended" | "error"
  fork_choice_mode?: string;
  attractor_count?: number;
  detail: string;
}

// ── HLWA — Half-Life Wrapped Asset ────────────────────────────────
//
// POST /api/hlwa/effective_supply (~L4374) — pure compute, no broadcast.
// POST /api/hlwa/re_attest (~L4401) — pure simulation; returns the asset
// state as if a fresh re-attestation had landed at current_epoch. Neither
// endpoint takes a signature.

export interface HlwaSupplyReq {
  current_supply: number;
  origin_attested_supply: number;
  last_attested_epoch: number;
  attestation_lambda_epochs: number;
  current_epoch: number;
}

export interface HlwaSupplyResp {
  status: string;
  effective_supply: number;
  current_supply: number;
  excess_to_burn: number;
  current_epoch: number;
  detail?: string;
}

export interface HlwaReAttestResp {
  status: string;
  new_attested_supply: number;
  new_last_attested_epoch: number;
  effective_supply_after: number;
  detail?: string;
}

// ── DSN — Decay-Stamped Nullifiers ─────────────────────────────────
//
// Privacy primitive: shielded transfers are made anonymous within a
// sliding-window accumulator. The window aggregates blake3-folded
// nullifiers; advancing rolls the oldest slot off.
//
// Endpoints (api.rs):
//   POST /api/dsn/fold_nullifier  (~L5170) — { nullifier_hex } →
//        { status, total_count, aggregate_root_hex }
//   POST /api/dsn/advance_window  (~L5195) — no body →
//        { status, total_count, aggregate_root_hex }
//   GET  /api/dsn/status          (~L5205) →
//        { status, total_count, aggregate_root_hex }
//
// All three responses share the same shape — they're emitted as
// serde_json::Value so the TS interface mirrors the keys exactly.

export interface DsnStatus {
  status: string;
  total_count: number;
  aggregate_root_hex: string;
}

export interface DsnFoldReq {
  nullifier_hex: string;
}

// ── LAD-VM — Linear/Affine/Decaying resource simulator ─────────────
//
// POST /api/lad_vm/simulate (~L1335). Pure compute, no chain mutation,
// no signature. Returns the lifecycle outcome for a single op against
// a Resource constructed from the request's mode + value. See
// LadSimQuery / LadSimResp in api.rs (L1300-L1329).
//
// `outcome` strings (api.rs L1319-1322):
//   "ok" | "evaporated" | "already-consumed" | "linear-cannot-drop" |
//   "ticked-fresh" | "ticked-evaporated" | "ticked-no-op" |
//   "missing-decay-window" | "error"

export type LadMode = "linear" | "affine" | "decaying";
export type LadAction = "use" | "drop" | "tick";

export interface LadSimReq {
  mode: LadMode;
  value: number;
  created_at_epoch: number;
  /** Required iff mode == "decaying". */
  decay_window?: number;
  current_epoch: number;
  action: LadAction;
}

export interface LadSimResp {
  status: string;
  action: string;
  mode: string;
  outcome: string;
  returned_value: number | null;
  is_evaporated_at_query: boolean;
  created_at_epoch: number;
  current_epoch: number;
  decay_window: number | null;
  detail: string;
}

// ── Bell-Beacon — CHSH quantum-randomness certification ───────────
//
// POST /api/bell_beacon (~L3447). CHSH inequality: S = E(a,b) − E(a,b')
// + E(a',b) + E(a',b'). Local-realism bound is |S| ≤ 2; in milli-units
// the threshold is 2000mb (LOCAL_REALISM_S_MILLI). Beacon is "Bell-
// certified" iff S > threshold.
//
// Request (BellBeaconQuery, api.rs L3427-3436): all four expectation
// values in milli-units (i64), optional threshold override.
// Response (BellBeaconResp, api.rs L3438-3445): { status, s_value_milli,
// threshold_milli, bell_certified, detail }.
//
// There is no read-only GET form for Bell-Beacon — beacon S-values are
// computed per-block and would need to be surfaced via the chain header
// to be queryable cheaply. The card therefore reflects a single design-
// target simulation on mount.

export interface BellBeaconReq {
  e_ab: number;
  e_ab_prime: number;
  e_a_prime_b: number;
  e_a_prime_b_prime: number;
  threshold_milli?: number;
}

export interface BellBeaconResp {
  status: string;
  s_value_milli: number;
  threshold_milli: number;
  bell_certified: boolean;
  detail: string;
}

// ── Sharding — cross-shard visibility ──────────────────────────────
//
// The node exposes two read-only endpoints:
//
//   GET /api/shards         (api.rs §get_shard_status, ~L8034)
//     → { active: true, num_shards, pending_cross_shard_messages }
//     | { active: false }   when sharding is disabled
//
//   GET /api/shards/health  (api.rs §get_shard_health, ~L8047)
//     → { shards: [{ shard_id, total_objects, live_objects,
//                    total_energy, liveness_ratio, is_dead }],
//         compaction_candidates }
//     | { active: false }   when sharding is disabled
//
// There is NO /api/address/:addr/shard endpoint and `/api/objects`
// does NOT include `shard_id`. We compute assignment client-side by
// mirroring `evaporchain_sharding::shard_for_object` (api.rs imports
// it via shard_bridge.rs):
//
//     shard = u16_be(id[0..2]) & (num_shards - 1)        when num_shards > 1
//     shard = 0                                          when num_shards == 1
//
// The same formula applies to 20-byte account addresses (the chain
// derives an address's shard from its first 2 bytes).

export interface ShardInfo {
  /** Numeric shard id (0..num_shards-1). Wire field: `shard_id`. */
  shard_id: number;
  /** Total objects ever recorded on this shard (live + dead). */
  total_objects: number;
  /** Currently-live objects (alive == true at last record). */
  live_objects: number;
  /** Sum of energy across live objects. */
  total_energy: number;
  /** live_objects / total_objects in [0,1]. */
  liveness_ratio: number;
  /** True when the shard's liveness ratio is at the compaction floor. */
  is_dead: boolean;
}

/**
 * Combined snapshot of sharding subsystem state. We synthesise
 * `total_shards`, `healthy`, `unhealthy` from /api/shards (num_shards)
 * and /api/shards/health (per-shard is_dead) so the UI can render
 * them as one row.
 */
export interface ShardsHealth {
  /** True iff `/api/shards` returned `active: true`. */
  active: boolean;
  /** Number of shards in the partition. 1 when sharding is single-shard. */
  total_shards: number;
  /** Count of shards with is_dead === false. */
  healthy: number;
  /** Count of shards with is_dead === true. */
  unhealthy: number;
  /** Bridge-level pending cross-shard messages (router queue depth). */
  pending_cross_shard_messages: number;
  /** From /api/shards/health: shards flagged for compaction. */
  compaction_candidates: number;
  /** Per-shard health rows from /api/shards/health. */
  shards: ShardInfo[];
}

/** Result of computing the shard for an account address client-side. */
export interface ShardAssignment {
  /** The 20-byte hex address that was looked up. */
  address: string;
  /** Computed shard id, mirroring `shard_for_object`. */
  shard_id: number;
}

// GET /api/bell/latest — most recent measured Bell-Beacon S-value. The
// node currently returns status:"no_data" until consensus persists S
// per-block; in that case wallets render a "no live measurement"
// fallback. api.rs §get_bell_latest / BellBeaconLatestResp.

export interface BellBeaconLatestResp {
  status: "ok" | "no_data" | "error";
  s_value_milli: number;
  threshold_milli: number;
  bell_certified: boolean;
  block_height: number;
  epoch: number;
  detail: string;
}

// ── Hot/Cold Stake — two-temperature validator equilibrium ─────────
//
// HotColdStakeReq mirrors api.rs §HotColdStakeReq (L4923-4935). All
// three endpoints (decay/promote/demote) take the same body; only
// promote/demote read the optional `amount` field. Pure compute, no
// signature.

export interface HotColdStakeReq {
  hot: number;
  cold: number;
  /** Hot half-life in epochs (api.rs default 100). */
  hot_lambda_epochs: number;
  /** Cold half-life in epochs (api.rs default 10000). */
  cold_lambda_epochs: number;
  last_touched_epoch: number;
  current_epoch: number;
  /** For promote/demote: amount to move cold↔hot. */
  amount?: number;
}

/** /api/hot_cold_stake/decay → status, hot_after, cold_after, total_after. */
export interface HotColdStakeDecayResp {
  status: string;
  hot_after: number;
  cold_after: number;
  total_after: number;
  epochs_elapsed: number;
}

/** /api/hot_cold_stake/{promote,demote}. `detail` is set on error. */
export interface HotColdStakeMoveResp {
  status: string;             // "ok" | "error"
  hot_after?: number;
  cold_after?: number;
  total_after?: number;
  /** Present on promote responses only; mirrors api.rs L4968. */
  promoted?: number;
  /** Present on demote responses only; mirrors api.rs L4983. */
  demoted?: number;
  detail?: string;
}

class EvaporChainAPI {
  private baseUrl: string;

  constructor(baseUrl: string = DEFAULT_NODE) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
  }

  setNode(url: string) {
    this.baseUrl = url.replace(/\/+$/, "");
  }

  private async get<T>(path: string): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`);
    if (!res.ok) throw new Error(`API ${res.status}: ${await res.text()}`);
    return res.json();
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`API ${res.status}: ${await res.text()}`);
    return res.json();
  }

  // ── Chain ──

  async getStatus(): Promise<ChainStatus> {
    return this.get("/api/status");
  }

  // ── Accounts ──

  async getAddressDetail(address: string): Promise<AccountDetail> {
    return this.get(`/api/address/${address}`);
  }

  async getAccounts(): Promise<AccountDetail[]> {
    return this.get("/api/accounts");
  }

  // ── Objects ──

  async getObjects(): Promise<StateObject[]> {
    return this.get("/api/objects");
  }

  async getObjectsByOwner(address: string): Promise<StateObject[]> {
    const all = await this.getObjects();
    return all.filter(o => o.owner === address);
  }

  // ── Prices ──

  async getPrices(): Promise<PriceData[]> {
    return this.get("/api/prices");
  }

  // ── Transactions ──

  async transfer(from: string, to: string, amount: number, nonce: number, signature?: string, publicKey?: string): Promise<TxResult> {
    return this.post("/api/tx/transfer", {
      from, to, amount, nonce,
      signature, public_key: publicKey,
    });
  }

  async refreshObject(objectId: string, energyDeposit: number): Promise<TxResult> {
    return this.post("/api/tx/refresh", {
      object_id: objectId,
      energy_deposit: energyDeposit,
    });
  }

  async createObject(creator: string, objectId: string, energy: number, halfLife: number): Promise<TxResult> {
    return this.post("/api/tx/create-object", {
      creator, object_id: objectId, energy, half_life: halfLife,
    });
  }

  // ── Validator delegation (P0 #4 — wallet-facing) ──
  //
  // The node accepts ML-DSA-signed Delegate / Undelegate / ClaimDelegation
  // transactions over /api/tx/{delegate,undelegate,claim_delegation}.
  // The chain refreshes per-validator delegated_stake at every consensus
  // tick (see TendermintConsensus::refresh_delegated_stakes); the
  // delegator's funds are locked at execute time, returned via
  // ClaimDelegation only after UNBONDING_PERIOD_EPOCHS.

  async delegate(req: DelegateRequest): Promise<TxResult> {
    return this.post("/api/tx/delegate", {
      delegator: req.delegator,
      validator_id: req.validatorId,
      amount: req.amount,
      nonce: req.nonce,
      signature: req.signature,
      public_key: req.publicKey,
    });
  }

  async undelegate(req: UndelegateRequest): Promise<TxResult> {
    return this.post("/api/tx/undelegate", {
      delegator: req.delegator,
      validator_id: req.validatorId,
      amount: req.amount,
      nonce: req.nonce,
      signature: req.signature,
      public_key: req.publicKey,
    });
  }

  async claimDelegation(req: ClaimDelegationRequest): Promise<TxResult> {
    return this.post("/api/tx/claim_delegation", {
      delegator: req.delegator,
      validator_id: req.validatorId,
      nonce: req.nonce,
      signature: req.signature,
      public_key: req.publicKey,
    });
  }

  async getValidatorDelegations(validatorId: number): Promise<ValidatorDelegationsResponse> {
    return this.get(`/api/validator/${validatorId}/delegations`);
  }

  // ── Faucet ──

  async claimFaucet(address: string): Promise<{ success: boolean; balance: number; message?: string }> {
    return this.post("/api/faucet", { address });
  }

  // ── Transactions history ──

  async getTransactions(): Promise<TransactionRecord[]> {
    return this.get("/api/transactions");
  }

  // ── Swap / DEX ──

  async getTokens(): Promise<TokenInfo[]> {
    return this.get("/api/tokens");
  }

  async getSwapQuote(fromToken: string, toToken: string, amount: number): Promise<SwapQuote> {
    return this.post("/api/swap/quote", { from_token: fromToken, to_token: toToken, amount });
  }

  async executeSwap(fromToken: string, toToken: string, amount: number, slippage: number, signature?: string, publicKey?: string): Promise<SwapResult> {
    return this.post("/api/swap/execute", { from_token: fromToken, to_token: toToken, amount, slippage, signature, public_key: publicKey });
  }

  // ── NFTs ──

  async getNfts(): Promise<NftItem[]> {
    return this.get("/api/nfts");
  }

  async getNftsByOwner(address: string): Promise<NftItem[]> {
    const all = await this.getNfts();
    return all.filter(n => n.owner === address);
  }

  async getNft(id: string): Promise<NftItem> {
    return this.get(`/api/nft/${id}`);
  }

  async refreshNft(id: string, energy: number): Promise<TxResult> {
    return this.post("/api/nft/refresh", { nft_id: id, energy_deposit: energy });
  }

  async transferNft(id: string, to: string): Promise<TxResult> {
    return this.post("/api/nft/transfer", { nft_id: id, to });
  }

  // ── Batch Refresh ──

  async batchRefresh(objects: Array<{ id: string; energy: number }>, signature?: string, publicKey?: string): Promise<TxResult> {
    return this.post("/api/tx/batch-refresh", { objects, signature, public_key: publicKey });
  }

  async getRefreshCost(objectId: string, targetEnergy: number): Promise<RefreshCostEstimate> {
    return this.post("/api/tx/refresh/estimate", { object_id: objectId, target_energy: targetEnergy });
  }

  // ── Ghost Recovery ──

  async getGhosts(owner?: string): Promise<GhostObject[]> {
    const ghosts = await this.get<GhostObject[]>("/api/ghosts");
    return owner ? ghosts.filter(g => g.owner === owner) : ghosts;
  }

  async getGhostDetail(id: string): Promise<GhostDetail> {
    return this.get(`/api/ghost/${id}`);
  }

  async resurrectObject(id: string, energy: number, signature?: string, publicKey?: string): Promise<TxResult> {
    return this.post("/api/tx/resurrect", { object_id: id, energy_deposit: energy, signature, public_key: publicKey });
  }

  async getRecoveryCost(id: string): Promise<RecoveryCostEstimate> {
    return this.get(`/api/ghost/${id}/cost`);
  }

  // ── Energy Dashboard ──

  async getEnergyPortfolio(address: string): Promise<EnergyPortfolio> {
    return this.get(`/api/address/${address}/energy`);
  }

  async getEnergyHistory(address: string): Promise<EnergyHistory> {
    return this.get(`/api/address/${address}/energy/history`);
  }

  // ── Simulation ──

  async simulateTransaction(tx: SimulateTransactionRequest): Promise<SimulationResult> {
    return this.post("/api/tx/simulate", tx);
  }

  async getDecayForecast(objectId: string): Promise<DecayForecastResult> {
    return this.get(`/api/object/${objectId}/forecast`);
  }

  // ── Social Auth ──

  async socialAuth(provider: "google" | "apple", token: string): Promise<SocialAuthResult> {
    return this.post("/api/auth/social", { provider, token });
  }

  // ── Tx status ──
  //
  // Wraps GET /api/tx/:hash. The node now returns TxStatus directly
  // with state ∈ {pending, mempool, included, finalised, rejected};
  // we just deserialise. Network errors degrade to "pending" with the
  // error attached so the poller keeps retrying.
  async getTxStatus(hash: string): Promise<TxStatus> {
    try {
      const res = await fetch(`${this.baseUrl}/api/tx/${hash}`);
      if (!res.ok) {
        return { hash, state: "pending", error: `HTTP ${res.status}` };
      }
      return (await res.json()) as TxStatus;
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      return { hash, state: "pending", error: msg };
    }
  }

  // ── Patronage Covenants ──

  async getPatronageStatus(): Promise<PatronageStatusResp> {
    return this.get("/api/patronage/status");
  }

  async getPatronageImmunity(objectIdHex: string, epoch: number): Promise<PatronageImmuneResp> {
    const q = new URLSearchParams({ object_id_hex: objectIdHex, epoch: String(epoch) });
    return this.get(`/api/patronage/immune?${q.toString()}`);
  }

  async pledgePatronage(req: PatronagePledgeReq): Promise<PatronagePledgeResp> {
    return this.post("/api/patronage/pledge", req);
  }

  async honourPatronage(req: PatronageActionReq): Promise<PatronageHonourResp> {
    return this.post("/api/patronage/honour", req);
  }

  async revokePatronage(req: PatronageActionReq): Promise<PatronageRevokeResp> {
    return this.post("/api/patronage/revoke", req);
  }

  // ── Refresh pool ──

  async getRefreshPool(): Promise<RefreshPoolStatus> {
    return this.get("/api/refresh_pool");
  }

  // ── Fee controller ──

  async getFeeControllerStatus(): Promise<FeeControllerStatus> {
    return this.get("/api/fee_controller/status");
  }

  async stepFeeController(gasUsed: number, epochsElapsed: number): Promise<FeeControllerStepResp> {
    return this.post("/api/fee_controller/step", {
      gas_used: gasUsed,
      epochs_elapsed: epochsElapsed,
    });
  }

  // ── Demurrage ──

  async getDemurrageOwed(req: DemurrageOwedReq): Promise<DemurrageOwedResp> {
    return this.post("/api/demurrage/owed", req);
  }

  async settleDemurrage(from: string, signature: string, publicKey: string): Promise<SettleDemurrageResp> {
    return this.post("/api/tx/settle_demurrage", {
      from,
      signature,
      public_key: publicKey,
    });
  }

  // ── Governance: fork-choice ──

  async getForkChoiceMode(): Promise<ForkChoiceModeStatus> {
    return this.get("/api/governance/fork_choice_mode");
  }

  async amendForkChoiceMode(req: ForkChoiceAmendReq): Promise<ForkChoiceAmendResp> {
    return this.post("/api/governance/fork_choice_mode", req);
  }

  // ── HLWA ──

  async getHlwaEffectiveSupply(req: HlwaSupplyReq): Promise<HlwaSupplyResp> {
    return this.post("/api/hlwa/effective_supply", req);
  }

  async reAttestHlwa(req: HlwaSupplyReq): Promise<HlwaReAttestResp> {
    return this.post("/api/hlwa/re_attest", req);
  }

  // ── DSN — Decay-Stamped Nullifiers ──

  async getDsnStatus(): Promise<DsnStatus> {
    return this.get("/api/dsn/status");
  }

  async foldDsnNullifier(req: DsnFoldReq): Promise<DsnStatus> {
    return this.post("/api/dsn/fold_nullifier", req);
  }

  async advanceDsnWindow(): Promise<DsnStatus> {
    return this.post("/api/dsn/advance_window", {});
  }

  // ── LAD-VM ──

  async simulateLadVm(req: LadSimReq): Promise<LadSimResp> {
    return this.post("/api/lad_vm/simulate", req);
  }

  // ── Bell-Beacon ──

  async getBellBeacon(req: BellBeaconReq): Promise<BellBeaconResp> {
    return this.post("/api/bell_beacon", req);
  }

  async getBellBeaconLatest(): Promise<BellBeaconLatestResp> {
    return this.get("/api/bell/latest");
  }

  // ── Hot/Cold Stake ──
  //
  // Two-temperature equilibrium per validator (research/INVENTION_STACK.md
  // §4.2). Hot stake decays fast and earns refresh credit; cold stake
  // decays slow and acts as long-term skin in the game. Validators
  // promote cold→hot (instant) and demote hot→cold.
  //
  // Endpoints (api.rs §"Hot/Cold Stake", L4921-4987):
  //   POST /api/hot_cold_stake/decay    (~L4948) — apply both decays from
  //        last_touched_epoch to current_epoch.
  //   POST /api/hot_cold_stake/promote  (~L4959) — simulate cold→hot move
  //        of `amount` (after decay). Returns "error" on insufficient cold.
  //   POST /api/hot_cold_stake/demote   (~L4974) — simulate hot→cold move
  //        of `amount` (after decay). Returns "error" on insufficient hot.
  //
  // All three are PURE COMPUTE — no chain state mutated, no signature
  // accepted. The wallet treats them as a what-if simulator over a
  // validator's hot/cold pools; the real promote/demote that mutates
  // chain state would land via a signed validator-stake transaction
  // once that's wired through evaporchain-execution. Until then the
  // card surfaces the substrate's behaviour visibly.
  //
  // HotColdStakeReq (api.rs §HotColdStakeReq, L4923-4935): hot, cold,
  // hot_lambda_epochs (default 100), cold_lambda_epochs (default 10000),
  // last_touched_epoch, current_epoch, optional `amount` for promote/
  // demote. Hot/cold-after responses return Energy units as u64.

  async hotColdStakeDecay(req: HotColdStakeReq): Promise<HotColdStakeDecayResp> {
    return this.post("/api/hot_cold_stake/decay", req);
  }

  async hotColdStakePromote(req: HotColdStakeReq): Promise<HotColdStakeMoveResp> {
    return this.post("/api/hot_cold_stake/promote", req);
  }

  async hotColdStakeDemote(req: HotColdStakeReq): Promise<HotColdStakeMoveResp> {
    return this.post("/api/hot_cold_stake/demote", req);
  }

  // ── Sharding ──
  //
  // /api/shards (api.rs L8034) reports num_shards + pending message
  // count. /api/shards/health (api.rs L8047) reports per-shard health.
  // We merge them into one ShardsHealth so callers don't have to.
  // When sharding is disabled the node returns `{active:false}`; we
  // surface that as a degraded-but-valid response with total_shards=1
  // and no per-shard rows.

  async getShards(): Promise<ShardInfo[]> {
    const merged = await this.getShardsHealth();
    return merged.shards;
  }

  async getShardsHealth(): Promise<ShardsHealth> {
    const [statusRaw, healthRaw] = await Promise.all([
      this.get<Record<string, unknown>>("/api/shards").catch(() => ({ active: false } as Record<string, unknown>)),
      this.get<Record<string, unknown>>("/api/shards/health").catch(() => ({ active: false } as Record<string, unknown>)),
    ]);

    const active = statusRaw["active"] === true;
    const numShards = typeof statusRaw["num_shards"] === "number"
      ? (statusRaw["num_shards"] as number)
      : (active ? 0 : 1);
    const pending = typeof statusRaw["pending_cross_shard_messages"] === "number"
      ? (statusRaw["pending_cross_shard_messages"] as number)
      : 0;

    const rawShards = Array.isArray(healthRaw["shards"])
      ? (healthRaw["shards"] as Record<string, unknown>[])
      : [];
    const shards: ShardInfo[] = rawShards.map((s) => ({
      shard_id: Number(s["shard_id"] ?? 0),
      total_objects: Number(s["total_objects"] ?? 0),
      live_objects: Number(s["live_objects"] ?? 0),
      total_energy: Number(s["total_energy"] ?? 0),
      liveness_ratio: Number(s["liveness_ratio"] ?? 0),
      is_dead: Boolean(s["is_dead"] ?? false),
    }));

    const compactionCandidates = typeof healthRaw["compaction_candidates"] === "number"
      ? (healthRaw["compaction_candidates"] as number)
      : 0;

    const healthy = shards.filter((s) => !s.is_dead).length;
    const unhealthy = shards.filter((s) => s.is_dead).length;

    return {
      active,
      total_shards: numShards,
      healthy,
      unhealthy,
      pending_cross_shard_messages: pending,
      compaction_candidates: compactionCandidates,
      shards,
    };
  }

  /**
   * Compute the shard for a 20-byte hex address.
   *
   * The node does NOT expose an `/api/address/:addr/shard` endpoint —
   * we mirror `evaporchain_sharding::shard_for_object` from the
   * sharding crate (shard_assignment.rs L42-L49):
   *
   *     shard = u16_be(id[0..2]) & (num_shards - 1)   when num_shards > 1
   *     shard = 0                                     otherwise
   *
   * Returns null when sharding is disabled (num_shards === 0) or the
   * address is malformed. Caller is expected to pass num_shards from
   * the cached ShardsHealth.
   */
  computeShardForAddress(address: string, numShards: number): ShardAssignment | null {
    if (numShards <= 0) return null;
    if (numShards === 1) return { address, shard_id: 0 };
    // Strip optional 0x prefix; require at least 4 hex chars (2 bytes).
    const hex = address.startsWith("0x") || address.startsWith("0X")
      ? address.slice(2)
      : address;
    if (hex.length < 4) return null;
    const b0 = parseInt(hex.slice(0, 2), 16);
    const b1 = parseInt(hex.slice(2, 4), 16);
    if (Number.isNaN(b0) || Number.isNaN(b1)) return null;
    const raw = (b0 << 8) | b1;
    // num_shards is required to be a power of 2 by ShardConfig; mask
    // is num_shards - 1.
    const mask = (numShards - 1) & 0xffff;
    return { address, shard_id: raw & mask };
  }

  /**
   * Address → shard. There is no node endpoint for this, so we read
   * num_shards from /api/shards and compute locally. Returns null when
   * sharding is disabled or the address is invalid.
   */
  async getShardForAddress(address: string): Promise<ShardAssignment | null> {
    try {
      const status = await this.get<Record<string, unknown>>("/api/shards");
      if (status["active"] !== true) return null;
      const numShards = typeof status["num_shards"] === "number"
        ? (status["num_shards"] as number)
        : 0;
      return this.computeShardForAddress(address, numShards);
    } catch {
      return null;
    }
  }
}

export const api = new EvaporChainAPI();
export { EvaporChainAPI };

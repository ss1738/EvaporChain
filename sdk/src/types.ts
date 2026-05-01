/** Chain status from /api/status */
export interface ChainStatus {
  chain_name: string;
  version: string;
  block_height: number;
  epoch: number;
  active_objects: number;
  ghost_count: number;
  total_evaporated: number;
  peer_count: number;
  state_root: string;
  proving_enabled: boolean;
  uptime_seconds: number;
}

/** A state object on chain */
export interface StateObject {
  id: string;
  name: string;
  owner: string;
  owner_name: string;
  energy: number;
  max_energy: number;
  half_life: number;
  state: "Active" | "Grace" | "Ghost" | "Risen";
  created_epoch: number;
  last_refreshed: number;
  grace_epoch: number | null;
  current_energy: number;
  decay_percentage: number;
}

/** An account with balance */
export interface Account {
  address: string;
  name: string;
  balance: number;
  nonce: number;
}

/** A block record */
export interface Block {
  number: number;
  epoch: number;
  parent_hash: string;
  state_root: string;
  tx_count: number;
  evaporations: number;
  entered_grace: number;
  timestamp: number;
  active_objects: number;
  ghost_count: number;
  transactions: TxRecord[];
}

/** Transaction record inside a block */
export interface TxRecord {
  type: string;
  detail: string;
}

/** A ghost (evaporated object) */
export interface Ghost {
  id: string;
  original_owner: string;
  evaporated_epoch: number;
  data_hash: string;
}

/** A deployed contract */
export interface Contract {
  id: number;
  template: string;
  creator: string;
  energy: number;
  half_life: number;
  created_epoch: number;
  evaporated: boolean;
}

/** Transaction submission result */
export interface TxResult {
  success: boolean;
  message: string;
}

/** Faucet claim result */
export interface FaucetResult {
  success: boolean;
  balance: number;
  message?: string;
}

/** Energy decay estimate for an object */
export interface DecayEstimate {
  current_energy: number;
  max_energy: number;
  half_life: number;
  epochs_elapsed: number;
  decay_percentage: number;
  estimated_epochs_remaining: number;
  will_enter_grace_at: number;
  will_evaporate_at: number;
}

/** Stats summary from /api/stats/summary */
export interface StatsSummary {
  total_created: number;
  total_evaporated: number;
  total_resurrected: number;
  total_refreshed: number;
  avg_lifetime_epochs: number;
  total_transactions: number;
}

/** Epoch snapshot from timeline */
export interface EpochSnapshot {
  epoch: number;
  active_count: number;
  ghost_count: number;
  total_energy: number;
}

/** Stats timeline from /api/stats/timeline */
export interface StatsTimeline {
  epochs: EpochSnapshot[];
}

/** Network info */
export interface NetworkInfo {
  peer_count: number;
}

/** Events response */
export interface EventRecord {
  epoch: number;
  event_type: string;
  message: string;
  timestamp_ms: number;
}

// ── Batch Transaction Types ──

export type BatchTxItem =
  | { type: "transfer"; from: number; to: number; amount: number; nonce: number }
  | { type: "create_object"; creator: number; object_id: number; energy: number; half_life: number }
  | { type: "refresh"; object_id: number; energy_deposit: number }
  | { type: "resurrect"; object_id: number; energy_deposit: number };

export interface BatchItemResult {
  index: number;
  success: boolean;
  message: string;
  tx_hash?: string;
}

export interface BatchResponse {
  submitted: number;
  failed: number;
  results: BatchItemResult[];
}

/** Options for the EvaporChain client */
export interface ClientOptions {
  /** Base URL of the EvaporChain node (default: https://testnet.evaporchain.com) */
  baseUrl?: string;
  /** Request timeout in ms (default: 10000) */
  timeout?: number;
  /** Max retries for failed requests (default: 3, 0 = no retries) */
  maxRetries?: number;
  /** Initial retry delay in ms, doubles each attempt (default: 500) */
  retryDelay?: number;
  /** WebSocket reconnect delay in ms (default: 3000) */
  wsReconnectDelay?: number;
  /** Maximum WebSocket reconnect attempts (default: 10, 0 = infinite) */
  wsMaxReconnects?: number;
}

// ── WebSocket Event Types ──

export interface WsNewBlock {
  type: "new_block";
  number: number;
  epoch: number;
  tx_count: number;
  timestamp: number;
  state_root: string;
  producer: string | null;
}

export interface WsNewTransaction {
  type: "new_transaction";
  hash: string;
  tx_type: string;
  from: string;
  to: string | null;
  amount: number | null;
}

export interface WsEvaporation {
  type: "evaporation";
  object_id: string;
  energy: number;
  block_number: number;
}

export interface WsGracePeriod {
  type: "grace_period";
  object_id: string;
  remaining_energy: number;
  block_number: number;
}

export interface WsChainEvent {
  type: "chain_event";
  event_type: string;
  message: string;
  epoch: number;
  timestamp_ms: number;
}

export interface WsPeerUpdate {
  type: "peer_update";
  connected: number;
}

export interface WsConnected {
  type: "connected";
  message: string;
  subscribers: number;
}

export interface WsWarning {
  type: "warning";
  message: string;
}

export interface WsContractLog {
  type: "contract_event";
  contract_id: number;
  block_number: number;
  event_name: string;
  topics: string[];
  data: string[];
}

export type WsEvent =
  | WsNewBlock
  | WsNewTransaction
  | WsEvaporation
  | WsGracePeriod
  | WsChainEvent
  | WsPeerUpdate
  | WsConnected
  | WsWarning
  | WsContractLog;

export type WsTopic = "blocks" | "transactions" | "evaporations" | "events" | "peers" | "contract_events" | "all";

/** Verkle state proof for light client verification */
export interface StateProofResponse {
  type: "account" | "object";
  state_root: string;
  exists: boolean;
  account?: { balance: number; nonce: number };
  object?: { energy: number; half_life: number; state: string; created_at: number; last_refreshed: number };
  proof: {
    key: string;
    value: string | null;
    depth: number;
    commitments: string[];
    path_indices: number[];
    siblings: Array<Array<{ index: number; hash: string }>>;
    hit_compressed: boolean;
  };
}

/** Transaction inclusion proof */
export interface TxInclusionProof {
  tx_hash: string;
  tx_index: number;
  block_number: number;
  merkle_root: string;
  siblings: Array<{ hash: string; position: "left" | "right" }>;
}

/** Compact block header for light client sync */
export interface CompactHeader {
  number: number;
  epoch: number;
  parent_hash: string;
  state_root: string;
  tx_count: number;
  tx_merkle_root: string;
  timestamp: number;
  has_nova_proof: boolean;
}

/** Contract ABI — describes the contract's typed interface */
export interface ContractAbi {
  name: string;
  methods: AbiMethod[];
  state: AbiStateField[];
  lifecycle_hooks: string[];
}

export interface AbiMethod {
  name: string;
  params: AbiParam[];
  return_type: string | null;
  mutates_state: boolean;
}

export interface AbiParam {
  name: string;
  ty: string;
}

export interface AbiStateField {
  name: string;
  ty: string;
  has_default: boolean;
}

// ── Substrate primitives ──
//
// Typed shapes mirror crates/evaporchain-node/src/api.rs byte-for-byte
// (snake_case wire format preserved — these endpoints predate the
// TS-side camelCase convention).

/** GET /api/patronage/status */
export interface PatronageStatus {
  active_covenants: number;
  total_pre_funded: number;
  total_active_score: number;
  patronage_ns_hex: string;
}

/** GET /api/patronage/immune */
export interface PatronageImmunity {
  object_id_hex: string;
  epoch: number;
  immune: boolean;
  patronage_score: number;
}

/** POST /api/patronage/pledge body */
export interface PatronagePledgeRequest {
  object_id_hex: string;
  namespace_id_hex: string;
  donation_per_epoch: number;
  epochs: number;
  current_epoch: number;
}

/** POST /api/patronage/pledge response */
export interface PatronagePledgeResponse {
  status: string;
  object_id_hex: string;
  pre_funded: number;
  expires_epoch: number;
  detail: string;
}

/** Shared body for /api/patronage/{honour,revoke} */
export interface PatronageActionRequest {
  object_id_hex: string;
  epoch: number;
}

/** POST /api/patronage/honour response */
export interface PatronageHonourResponse {
  status: string;
  donated: number;
  patronage_score: number;
  detail: string;
}

/** POST /api/patronage/revoke envelope (free-form) */
export interface PatronageRevokeResponse {
  status: string;
  object_id_hex?: string;
  patronage_score_archived?: number;
  refunded?: number;
  detail: string;
}

/** Attractor entry inside fork-choice amendment */
export interface AttractorSpec {
  center: number;
  basin_radius: number;
}

/** GET /api/governance/fork_choice_mode */
export interface ForkChoiceModeStatus {
  fork_choice_mode: string;
  attractors: AttractorSpec[];
  detail: string;
}

/**
 * POST /api/governance/fork_choice_mode body. Authorised by stake
 * quorum (`endorser_stakes` summing to >= `required_stake`); no
 * per-tx signature on the body (mirrors api.rs::ForkChoiceAmendReq).
 */
export interface ForkChoiceAmendRequest {
  /** "mcc" or "singh_attractor" */
  mode: string;
  /** Required when mode === "singh_attractor" */
  attractors?: AttractorSpec[];
  endorser_stakes: number[];
  required_stake: number;
}

/** Refresh-pool credit row */
export interface RefreshPoolCredit {
  namespace_hex: string;
  accrued: number;
  last_touched_epoch: number;
}

/** GET /api/refresh_pool */
export interface RefreshPoolStatus {
  total_accrued: number;
  credits: RefreshPoolCredit[];
}

/** GET /api/fee_controller/status */
export interface FeeControllerStatus {
  status: string;
  energy: number;
  base_fee: number;
  target_energy: number;
  target_gas: number;
  fee_response_ppm: number;
}

/** POST /api/fee_controller/step body */
export interface FeeControllerStepRequest {
  gas_used: number;
  epochs_elapsed: number;
}

/** POST /api/fee_controller/step response */
export interface FeeControllerStepResponse {
  status: string;
  energy_after?: number;
  base_fee?: number;
  lyapunov_v_before?: number;
  lyapunov_v_after?: number;
  lyapunov_delta?: number;
  gas_used?: number;
  detail?: string;
}

/** POST /api/demurrage/owed body */
export interface DemurrageOwedRequest {
  balance: number;
  last_touched_epoch: number;
  current_epoch: number;
  /** λ_base in ppm/epoch (0 = disabled) */
  lambda_base_ppm: number;
  threshold: number;
}

/** POST /api/demurrage/owed response */
export interface DemurrageOwedResponse {
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

/**
 * POST /api/tx/settle_demurrage body. ML-DSA signed; canonical signing
 * payload: `JSON({type:"settle_demurrage",from,current_epoch})` —
 * exactly the byte sequence verified in api.rs::post_settle_demurrage.
 */
export interface SettleDemurrageRequest {
  from: string;
  /** Hex-encoded ML-DSA signature over the canonical payload */
  signature: string;
  /** Hex-encoded ML-DSA public key */
  public_key: string;
}

/** POST /api/tx/settle_demurrage response */
export interface SettleDemurrageResponse {
  /** "settled" | "nothing_owed" | "error" */
  status: string;
  settled: number;
  new_balance: number;
  new_last_touched_epoch: number;
  detail: string;
}

/** Shared body for /api/hlwa/{effective_supply,re_attest} */
export interface HlwaEffectiveSupplyRequest {
  current_supply: number;
  origin_attested_supply: number;
  last_attested_epoch: number;
  /** Half-life of attestation freshness, in epochs */
  attestation_lambda_epochs: number;
  current_epoch: number;
}

/** POST /api/dsn/fold_nullifier body */
export interface DsnFoldRequest {
  /** 32-byte nullifier as 64 hex chars */
  nullifier_hex: string;
}

/** GET /api/dsn/status / POST /api/dsn/{fold_nullifier,advance_window} */
export interface DsnStatus {
  status?: string;
  total_count: number;
  aggregate_root_hex: string;
}

export type LadMode = "linear" | "affine" | "decaying";
export type LadAction = "use" | "drop" | "tick";

/** POST /api/lad_vm/simulate body */
export interface LadSimulateRequest {
  mode: LadMode;
  value: number;
  created_at_epoch: number;
  /** Required iff mode === "decaying" */
  decay_window?: number;
  current_epoch: number;
  action: LadAction;
}

/** POST /api/lad_vm/simulate response */
export interface LadSimulateResponse {
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

/** GET /api/bell/latest */
export interface BellBeaconLatest {
  /** "ok" | "no_data" | "error" */
  status: string;
  s_value_milli: number;
  threshold_milli: number;
  bell_certified: boolean;
  block_height: number;
  epoch: number;
  detail: string;
}

/** POST /api/bell_beacon (worked-example simulator) body */
export interface BellBeaconRequest {
  e_ab: number;
  e_ab_prime: number;
  e_a_prime_b: number;
  e_a_prime_b_prime: number;
  /** Defaults to LOCAL_REALISM_S_MILLI = 2000 if omitted */
  threshold_milli?: number;
}

/** POST /api/bell_beacon response */
export interface BellBeaconResponse {
  status: string;
  s_value_milli: number;
  threshold_milli: number;
  bell_certified: boolean;
  detail: string;
}

/** Contract event log from the indexer */
export interface ContractEventLog {
  contract_id: number;
  block_number: number;
  log_index: number;
  epoch: number;
  timestamp: number;
  tx_hash: string;
  event_name: string;
  topics: string[];
  data: string[];
}

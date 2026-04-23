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

export type WsEvent =
  | WsNewBlock
  | WsNewTransaction
  | WsEvaporation
  | WsGracePeriod
  | WsChainEvent
  | WsPeerUpdate
  | WsConnected
  | WsWarning;

export type WsTopic = "blocks" | "transactions" | "evaporations" | "events" | "peers" | "all";

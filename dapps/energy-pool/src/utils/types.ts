export type DistributionStrategy = "equal" | "priority-low-energy";

export interface Pool {
  id: string;
  name: string;
  description: string;
  creator: string;
  strategy: DistributionStrategy;
  total_energy: number;
  contributor_count: number;
  protected_objects: ProtectedObject[];
  health_pct: number;
  created_epoch: number;
  last_distribution_epoch: number;
}

export interface ProtectedObject {
  object_id: string;
  name: string;
  object_type: string;
  current_energy: number;
  max_energy: number;
  state: "Active" | "Grace" | "Ghost";
  half_life: number;
  last_refreshed: number;
  energy_received: number;
}

export interface Contributor {
  address: string;
  staked_energy: number;
  guardian_points: number;
  objects_saved: number;
  joined_epoch: number;
  rank: number;
}

export interface PoolActivity {
  id: string;
  pool_id: string;
  action: "stake" | "unstake" | "distribute" | "object_saved" | "object_lost";
  address: string;
  amount: number;
  object_id?: string;
  epoch: number;
  timestamp: number;
}

export interface TxResult {
  success: boolean;
  message: string;
  tx_hash?: string;
}

export interface UserDashboard {
  total_staked: number;
  total_guardian_points: number;
  objects_saved: number;
  pools: UserPoolStake[];
}

export interface UserPoolStake {
  pool_id: string;
  pool_name: string;
  staked_energy: number;
  guardian_points: number;
  health_pct: number;
}

export interface ChainStatus {
  block_height: number;
  epoch: number;
  active_objects: number;
  ghost_count: number;
}

// ── Substrate primitives (snake_case wire shape) ──

export interface RefreshPoolCredit {
  namespace_hex: string;
  accrued: number;
  last_touched_epoch: number;
}

export interface RefreshPoolStatus {
  total_accrued: number;
  credits: RefreshPoolCredit[];
}

export interface RefreshPoolContributeRequest {
  from: string;
  amount: number;
  pool_id?: string;
  memo?: string;
}

export interface FeeControllerStatus {
  status: string;
  energy: number;
  base_fee: number;
  target_energy: number;
  target_gas: number;
  fee_response_ppm: number;
}

export interface FeeControllerStepRequest {
  gas_used: number;
  epochs_elapsed: number;
}

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

export interface PatronageStatusResponse {
  active_covenants: number;
  total_pre_funded: number;
  total_active_score: number;
  patronage_ns_hex: string;
}

export interface Nft {
  id: number;
  name: string;
  collection: string;
  owner: string;
  metadata_hash: string;
  energy: number;
  max_energy: number;
  current_energy: number;
  half_life: number;
  minted_epoch: number;
  last_refreshed: number;
  state: "Active" | "Grace" | "Ghost";
  decay_percentage: number;
  epochs_remaining: number;
  grace_epoch: number | null;
  evaporated_epoch: number | null;
  ghost_proof: string | null;
}

export interface ChainStatus {
  block_height: number;
  epoch: number;
  active_objects: number;
  ghost_count: number;
}

export interface TxResult {
  success: boolean;
  message: string;
  tx_hash?: string;
}

export interface NftCollection {
  name: string;
  count: number;
  nft_ids: number[];
}

// ── Substrate primitives (snake_case wire shape — matches api.rs) ──

export interface PatronagePledgeRequest {
  object_id_hex: string;
  namespace_id_hex: string;
  donation_per_epoch: number;
  epochs: number;
  current_epoch: number;
}

export interface PatronagePledgeResponse {
  status: string;
  object_id_hex: string;
  pre_funded: number;
  expires_epoch: number;
  detail: string;
}

export interface PatronageStatusResponse {
  active_covenants: number;
  total_pre_funded: number;
  total_active_score: number;
  patronage_ns_hex: string;
}

export interface PatronageImmunityResponse {
  object_id_hex: string;
  epoch: number;
  immune: boolean;
  patronage_score: number;
}

export interface RefreshPoolCredit {
  namespace_hex: string;
  accrued: number;
  last_touched_epoch: number;
}

export interface RefreshPoolStatus {
  total_accrued: number;
  credits: RefreshPoolCredit[];
}

export interface DemurrageOwedRequest {
  balance: number;
  last_touched_epoch: number;
  current_epoch: number;
  lambda_base_ppm: number;
  threshold: number;
}

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

export interface HlwaEffectiveSupplyRequest {
  current_supply: number;
  origin_attested_supply: number;
  last_attested_epoch: number;
  attestation_lambda_epochs: number;
  current_epoch: number;
}

export interface HlwaEffectiveSupplyResponse {
  status: string;
  effective_supply: number;
  current_supply: number;
  excess_to_burn: number;
  current_epoch: number;
  detail?: string;
}

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

/** Node status and chain statistics. */
export interface ChainStatus {
  block_height: number;
  epoch: number;
  active_objects: number;
  ghost_count: number;
  peer_count: number;
  state_root: string;
  uptime_secs: number;
  proving_mode: string;
  total_transactions: number;
  total_evaporated: number;
  total_objects_created: number;
  total_refreshed: number;
}

/** Block information. */
export interface BlockInfo {
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
  transactions: TransactionRecord[];
}

/** Transaction record within a block. */
export interface TransactionRecord {
  type: string;
  from?: string;
  to?: string;
  amount?: number;
  object_id?: string;
  energy?: number;
}

/** Account with balance and nonce. */
export interface AccountInfo {
  address: string;
  balance: number;
  nonce: number;
}

/** State object with energy decay. */
export interface StateObject {
  id: string;
  owner: string;
  energy: number;
  half_life: number;
  created_at: number;
  last_refreshed: number;
  state: 'Active' | 'Grace' | 'Ghost';
  grace_epoch: number | null;
  current_energy: number;
  decay_percentage: number;
  data: string;
  name: string;
}

/** Ghost record for evaporated objects. */
export interface GhostRecord {
  object_id: string;
  evaporated_at: number;
  nullifier: string;
  data_hash: string;
}

/** Event from the chain. */
export interface EventRecord {
  epoch: number;
  event_type: string;
  message: string;
  timestamp: number;
}

/** Transfer transaction parameters. */
export interface TransferParams {
  from: string;
  to: string;
  amount: number;
}

/** Create object parameters. */
export interface CreateObjectParams {
  creator: string;
  energy: number;
  half_life: number;
  data?: string;
}

/** Refresh object parameters. */
export interface RefreshParams {
  object_id: string;
  energy_deposit: number;
}

/** NFT token. */
export interface NftToken {
  id: number;
  name: string;
  collection: string;
  owner: string;
  energy: number;
  max_energy: number;
  half_life: number;
  state: string;
  minted_epoch: number;
  metadata_hash: string;
}

/** Deployed fungible token. */
export interface DeployedToken {
  id: number;
  name: string;
  symbol: string;
  total_supply: number;
  decay_half_life: number;
  deployed_epoch: number;
  deployer: string;
}

/** Staking pool. */
export interface StakingPool {
  id: number;
  name: string;
  reward_rate: number;
  reward_decay_hl: number;
  total_staked: number;
  created_epoch: number;
  stakers: Array<{
    address: string;
    amount: number;
    staked_epoch: number;
    pending_rewards: number;
  }>;
}

/** DAO proposal. */
export interface DAOProposal {
  id: number;
  title: string;
  description: string;
  options: string[];
  votes: Array<{
    voter: string;
    option: string;
    weight: number;
    epoch: number;
  }>;
  status: string;
  created_epoch: number;
  voting_period: number;
  creator: string;
}

/** Faucet request result. */
export interface FaucetResult {
  success: boolean;
  amount?: number;
  message: string;
  new_balance?: number;
}

/** Address detail response. */
export interface AddressDetail {
  address: string;
  balance: number;
  nonce: number;
  objects: StateObject[];
  nfts: NftToken[];
  tokens: Array<{ name: string; symbol: string; balance: number }>;
}

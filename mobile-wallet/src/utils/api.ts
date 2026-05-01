/**
 * EvaporChain REST API Client for Mobile Wallet
 *
 * Uses the same REST endpoints as the browser extension and testnet explorer.
 * Transactions are sent unsigned — the node signs with its keypair for mobile.
 * Future: bundle a Dilithium3 JS implementation for client-side signing.
 */

export interface ChainStatus {
  chainName: string;
  version: string;
  blockHeight: number;
  epoch: number;
  activeObjects: number;
  ghostCount: number;
  peerCount: number;
}

export interface Balance {
  address: string;
  balance: number;
  nonce: number;
}

export interface Transaction {
  hash: string;
  type: string;
  detail: string;
  from: string;
  to: string;
  amount: string;
  timestamp: number;
}

export interface TxResult {
  success: boolean;
  message: string;
  tx_hash?: string;
}

export type ObjectState = 'Active' | 'Grace' | 'Ghost' | 'Risen';

export interface ChainObject {
  id: string;
  name: string;
  owner: string;
  energy: number;
  maxEnergy: number;
  state: ObjectState;
  halfLife: number;
  currentEnergy: number;
  decayPercentage: number;
  estimatedGhostTime: number;
}

export interface NFT {
  id: string;
  name: string;
  collection: string;
  collectionName: string;
  owner: string;
  imageUri?: string;
  energy: number;
  maxEnergy: number;
  currentEnergy: number;
  state: ObjectState;
  decayPercentage: number;
  estimatedGhostTime: number;
}

export interface StakingInfo {
  staked: number;
  rewards: number;
  isValidator: boolean;
  epoch: number;
  stakingStartEpoch?: number;
  unbondingAmount?: number;
  unbondingCompleteEpoch?: number;
}

export interface Validator {
  address: string;
  name: string;
  stake: number;
  commission: number;
  uptime: number;
  status: 'active' | 'jailed' | 'inactive';
}

export interface SwapQuote {
  from_token: string;
  to_token: string;
  amount_in: number;
  amount_out: number;
  rate: number;
  price_impact: number;
}

// ── Patronage Covenant types (api.rs §PatronagePledgeReq L1510 etc.) ──

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
  status: string;
  object_id_hex: string;
  pre_funded: number;
  expires_epoch: number;
  detail: string;
}

export interface PatronageHonourResp {
  status: string;
  donated: number;
  patronage_score: number;
  detail: string;
}

export interface PatronageRevokeResp {
  status: string;
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

// ── Refresh pool (api.rs §RefreshPoolResp L2844) ──

export interface RefreshPoolCredit {
  namespace_hex: string;
  accrued: number;
  last_touched_epoch: number;
}

export interface RefreshPoolStatus {
  total_accrued: number;
  credits: RefreshPoolCredit[];
}

// ── Fee controller (api.rs §get_fee_controller_status L4672) ──

export interface FeeControllerStatus {
  status: string;
  energy: number;
  base_fee: number;
  target_energy: number;
  target_gas: number;
  fee_response_ppm: number;
}

// ── Demurrage (api.rs §post_demurrage_owed / §post_settle_demurrage) ──

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

export interface SettleDemurrageResp {
  status: 'settled' | 'nothing_owed' | 'error';
  settled: number;
  new_balance: number;
  new_last_touched_epoch: number;
  detail: string;
}

// ── Account detail (api.rs §AddressDetailResponse L7647) ──

export interface AccountDetail {
  address: string;
  balance: number;
  nonce: number;
  /**
   * Epoch the account was last "touched" by an outbound tx.
   * Anchors per-account demurrage. New accounts default to 0.
   */
  last_touched_epoch: number;
}

// ── Governance: fork-choice (api.rs §post_governance_fork_choice_mode) ──

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
  status: string;
  fork_choice_mode?: string;
  attractor_count?: number;
  detail: string;
}

// ── HLWA (api.rs §post_hlwa_effective_supply / §post_hlwa_re_attest) ──

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

// ── DSN (api.rs §post_dsn_fold_nullifier / §get_dsn_status) ──

export interface DsnStatus {
  status: string;
  total_count: number;
  aggregate_root_hex: string;
}

export interface DsnFoldReq {
  nullifier_hex: string;
}

// ── Bell-Beacon (api.rs §post_bell_beacon / §get_bell_latest) ──

export interface BellBeaconLatestResp {
  status: 'ok' | 'no_data' | 'error';
  s_value_milli: number;
  threshold_milli: number;
  bell_certified: boolean;
  block_height: number;
  epoch: number;
  detail: string;
}

const DEFAULT_BASE_URL = 'https://testnet.evaporchain.com';

/**
 * Convert snake_case REST API responses to camelCase for React consumption.
 */
function toCamelCase<T>(obj: unknown): T {
  if (Array.isArray(obj)) return obj.map((item) => toCamelCase(item)) as T;
  if (obj !== null && typeof obj === 'object') {
    const result: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(obj as Record<string, unknown>)) {
      const camelKey = key.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
      result[camelKey] = toCamelCase(value);
    }
    return result as T;
  }
  return obj as T;
}

class EvaporChainAPI {
  private baseUrl: string;
  private network: 'testnet' | 'mainnet' = 'testnet';

  constructor(baseUrl: string = DEFAULT_BASE_URL) {
    this.baseUrl = baseUrl.replace(/\/+$/, '');
  }

  getNetwork(): string {
    return this.network;
  }

  setNetwork(network: 'testnet' | 'mainnet'): void {
    this.network = network;
    this.baseUrl = network === 'mainnet'
      ? 'https://rpc.evaporchain.io'
      : DEFAULT_BASE_URL;
  }

  private async get<T>(path: string): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`);
    if (!res.ok) throw new Error(`API ${res.status}`);
    const json = await res.json();
    return toCamelCase<T>(json);
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`API ${res.status}`);
    const json = await res.json();
    return toCamelCase<T>(json);
  }

  /**
   * Snake_case-preserving GET. Substrate endpoints (patronage, refresh-pool,
   * fee-controller, demurrage, governance, HLWA, DSN, Bell-Beacon) expose
   * field names that match api.rs byte-for-byte and must NOT be camel-cased
   * because the wire shape is what the screens read directly.
   */
  private async getRaw<T>(path: string): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`);
    if (!res.ok) throw new Error(`API ${res.status}: ${await res.text()}`);
    return res.json() as Promise<T>;
  }

  private async postRaw<T>(path: string, body: unknown): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`API ${res.status}: ${await res.text()}`);
    return res.json() as Promise<T>;
  }

  // ── Chain ──

  async getChainStatus(): Promise<ChainStatus> {
    return this.get('/api/status');
  }

  // ── Account ──

  async getBalance(address: string): Promise<Balance> {
    return this.get(`/api/address/${address}`);
  }

  // ── Transactions ──

  async transfer(from: string, to: string, amount: number, nonce: number): Promise<TxResult> {
    return this.post('/api/tx/transfer', { from, to, amount, nonce });
  }

  async getTransactions(address?: string, limit?: number): Promise<Transaction[]> {
    const params = new URLSearchParams();
    if (address) params.set('address', address);
    if (limit) params.set('limit', limit.toString());
    const query = params.toString();
    return this.get(`/api/transactions${query ? `?${query}` : ''}`);
  }

  // ── Faucet ──

  async claimFaucet(address: string): Promise<{ success: boolean; balance: number; message?: string }> {
    return this.post('/api/faucet', { address });
  }

  // ── Objects ──

  async getObjects(owner?: string): Promise<ChainObject[]> {
    const path = owner ? `/api/objects?owner=${owner}` : '/api/objects';
    return this.get(path);
  }

  async refreshObject(objectId: string, energyDeposit: number): Promise<TxResult> {
    return this.post('/api/tx/refresh', { object_id: objectId, energy_deposit: energyDeposit });
  }

  // ── NFTs ──

  async getNFTs(owner?: string): Promise<NFT[]> {
    const path = owner ? `/api/nfts?owner=${owner}` : '/api/nfts';
    return this.get(path);
  }

  async refreshNFT(nftId: string, energy: number): Promise<TxResult> {
    return this.post('/api/nft/refresh', { nft_id: nftId, energy_deposit: energy });
  }

  // ── Swap ──

  async getSwapQuote(fromToken: string, toToken: string, amount: number): Promise<SwapQuote> {
    return this.post('/api/swap/quote', { from_token: fromToken, to_token: toToken, amount });
  }

  async executeSwap(fromToken: string, toToken: string, amount: number, slippage: number): Promise<TxResult> {
    return this.post('/api/swap/execute', { from_token: fromToken, to_token: toToken, amount, slippage });
  }

  // ── Staking ──

  async getStakingInfo(address: string): Promise<StakingInfo> {
    return this.get(`/api/staking/${address}`);
  }

  async getValidators(): Promise<Validator[]> {
    return this.get('/api/validators');
  }

  async stake(from: string, amount: number, nonce: number): Promise<TxResult> {
    return this.post('/api/tx/stake', { from, amount, nonce });
  }

  async unstake(from: string, amount: number, nonce: number): Promise<TxResult> {
    return this.post('/api/tx/unstake', { from, amount, nonce });
  }

  async claimRewards(from: string, nonce: number): Promise<TxResult> {
    return this.post('/api/tx/claim-rewards', { from, nonce });
  }

  // ── Substrate visibility surfaces (snake_case preserved) ──
  //
  // The eight substrate endpoints below mirror the browser extension
  // 1:1. They use `getRaw`/`postRaw` because every field name (e.g.
  // `total_accrued`, `last_touched_epoch`, `s_value_milli`) is consumed
  // directly by the substrate screens and must match api.rs byte-for-byte.

  // Account detail with last_touched_epoch (api.rs §AddressDetailResponse)
  async getAccountDetail(address: string): Promise<AccountDetail> {
    return this.getRaw(`/api/address/${address}`);
  }

  // Patronage Covenants (api.rs L1510-1730)
  async getPatronageStatus(): Promise<PatronageStatusResp> {
    return this.getRaw('/api/patronage/status');
  }

  async getPatronageImmunity(objectIdHex: string, epoch: number): Promise<PatronageImmuneResp> {
    const q = new URLSearchParams({ object_id_hex: objectIdHex, epoch: String(epoch) });
    return this.getRaw(`/api/patronage/immune?${q.toString()}`);
  }

  async pledgePatronage(req: PatronagePledgeReq): Promise<PatronagePledgeResp> {
    return this.postRaw('/api/patronage/pledge', req);
  }

  async honourPatronage(req: PatronageActionReq): Promise<PatronageHonourResp> {
    return this.postRaw('/api/patronage/honour', req);
  }

  async revokePatronage(req: PatronageActionReq): Promise<PatronageRevokeResp> {
    return this.postRaw('/api/patronage/revoke', req);
  }

  // Refresh pool (api.rs §RefreshPoolResp)
  async getRefreshPool(): Promise<RefreshPoolStatus> {
    return this.getRaw('/api/refresh_pool');
  }

  // Fee controller (api.rs §get_fee_controller_status)
  async getFeeControllerStatus(): Promise<FeeControllerStatus> {
    return this.getRaw('/api/fee_controller/status');
  }

  // Demurrage (api.rs §post_demurrage_owed / §post_settle_demurrage)
  async getDemurrageOwed(req: DemurrageOwedReq): Promise<DemurrageOwedResp> {
    return this.postRaw('/api/demurrage/owed', req);
  }

  async settleDemurrage(
    from: string,
    signature: string,
    publicKey: string,
  ): Promise<SettleDemurrageResp> {
    return this.postRaw('/api/tx/settle_demurrage', {
      from,
      signature,
      public_key: publicKey,
    });
  }

  // Governance — fork-choice (api.rs §post_governance_fork_choice_mode)
  async getForkChoiceMode(): Promise<ForkChoiceModeStatus> {
    return this.getRaw('/api/governance/fork_choice_mode');
  }

  async amendForkChoiceMode(req: ForkChoiceAmendReq): Promise<ForkChoiceAmendResp> {
    return this.postRaw('/api/governance/fork_choice_mode', req);
  }

  // HLWA (api.rs §post_hlwa_effective_supply / §post_hlwa_re_attest)
  async getHlwaEffectiveSupply(req: HlwaSupplyReq): Promise<HlwaSupplyResp> {
    return this.postRaw('/api/hlwa/effective_supply', req);
  }

  async reAttestHlwa(req: HlwaSupplyReq): Promise<HlwaReAttestResp> {
    return this.postRaw('/api/hlwa/re_attest', req);
  }

  // DSN — Decay-Stamped Nullifiers (api.rs §get_dsn_status / §fold_nullifier)
  async getDsnStatus(): Promise<DsnStatus> {
    return this.getRaw('/api/dsn/status');
  }

  async foldDsnNullifier(req: DsnFoldReq): Promise<DsnStatus> {
    return this.postRaw('/api/dsn/fold_nullifier', req);
  }

  async advanceDsnWindow(): Promise<DsnStatus> {
    return this.postRaw('/api/dsn/advance_window', {});
  }

  // Bell-Beacon (api.rs §get_bell_latest)
  async getBellBeaconLatest(): Promise<BellBeaconLatestResp> {
    return this.getRaw('/api/bell/latest');
  }
}

export const api = new EvaporChainAPI();
export default EvaporChainAPI;

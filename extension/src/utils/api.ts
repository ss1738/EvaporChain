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

export interface TransactionRecord {
  type: string;
  detail: string;
  hash?: string;
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

  async executeSwap(fromToken: string, toToken: string, amount: number, slippage: number): Promise<SwapResult> {
    return this.post("/api/swap/execute", { from_token: fromToken, to_token: toToken, amount, slippage });
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

  async batchRefresh(objects: Array<{ id: string; energy: number }>): Promise<TxResult> {
    return this.post("/api/tx/batch-refresh", { objects });
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

  async resurrectObject(id: string, energy: number): Promise<TxResult> {
    return this.post("/api/tx/resurrect", { object_id: id, energy_deposit: energy });
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
}

export const api = new EvaporChainAPI();
export { EvaporChainAPI };

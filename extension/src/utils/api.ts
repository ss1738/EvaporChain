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

  // ── Simulation ──

  async simulateTransaction(tx: SimulateTransactionRequest): Promise<SimulationResult> {
    return this.post("/api/tx/simulate", tx);
  }

  async getDecayForecast(objectId: string): Promise<DecayForecastResult> {
    return this.get(`/api/object/${objectId}/forecast`);
  }
}

export const api = new EvaporChainAPI();
export { EvaporChainAPI };

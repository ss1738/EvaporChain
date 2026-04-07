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
}

export const api = new EvaporChainAPI();
export { EvaporChainAPI };

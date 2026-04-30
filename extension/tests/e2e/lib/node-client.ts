/**
 * Node REST client used by tests for setup/assertions outside the popup.
 *
 * Field names mirror the wallet's own `utils/api.ts` (balance, nonce,
 * block_height etc.) so cross-references in assertions stay obvious.
 *
 * Intentionally narrow — only the endpoints currently used by specs.
 * Add more as new specs need them; do not pull in the full wallet
 * `api.ts` (it imports from `@/...` and pulls react/zustand transitively).
 */

import { E2E_PATHS } from "../playwright.config";

export interface NodeStatus {
  chain_name: string;
  block_height: number;
  epoch: number;
  peer_count: number;
}

export interface AccountDetail {
  address: string;
  balance: number;
  nonce: number;
}

export interface FaucetResp {
  success: boolean;
  balance: number;
  message?: string;
}

export interface TxRecord {
  hash: string;
  status: string; // "success" | "failed" | ...
  block_number?: number;
  epoch?: number;
}

export class NodeClient {
  constructor(private readonly baseUrl = E2E_PATHS.nodeUrl) {
    this.baseUrl = this.baseUrl.replace(/\/+$/, "");
  }

  async getStatus(): Promise<NodeStatus> {
    return this.get("/api/status");
  }

  async getBalance(address: string): Promise<AccountDetail> {
    return this.get(`/api/address/${address}`);
  }

  async claimFaucet(address: string): Promise<FaucetResp> {
    return this.post("/api/faucet", { address });
  }

  /**
   * Returns null while the node has no record yet (404). Returns the
   * record once finalised. Specs should poll this with their own
   * timeout — the node only emits records from finalised blocks, so
   * a non-null return means the tx is on-chain.
   */
  async getTransactionRecord(hash: string): Promise<TxRecord | null> {
    const res = await fetch(`${this.baseUrl}/api/tx/${hash}`);
    if (res.status === 404) return null;
    if (!res.ok) throw new Error(`getTransactionRecord ${res.status}: ${await res.text()}`);
    return res.json() as Promise<TxRecord>;
  }

  async pollUntilFinalised(hash: string, timeoutMs = 60_000, intervalMs = 1_000): Promise<TxRecord> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const rec = await this.getTransactionRecord(hash);
      if (rec && rec.status) return rec;
      await new Promise((r) => setTimeout(r, intervalMs));
    }
    throw new Error(`pollUntilFinalised: tx ${hash} did not finalise within ${timeoutMs}ms`);
  }

  private async get<T>(path: string): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`);
    if (!res.ok) throw new Error(`GET ${path} ${res.status}: ${await res.text()}`);
    return res.json() as Promise<T>;
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`POST ${path} ${res.status}: ${await res.text()}`);
    return res.json() as Promise<T>;
  }
}

export const nodeClient = new NodeClient();

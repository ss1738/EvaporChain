/**
 * EvaporChain REST API Client for Mobile Wallet
 *
 * Uses the same REST endpoints as the browser extension and testnet explorer.
 * Transactions are sent unsigned — the node signs with its keypair for mobile.
 * Future: bundle a Dilithium3 JS implementation for client-side signing.
 */

export interface ChainStatus {
  chain_name: string;
  version: string;
  block_height: number;
  epoch: number;
  active_objects: number;
  ghost_count: number;
  peer_count: number;
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
  max_energy: number;
  state: ObjectState;
  half_life: number;
  current_energy: number;
  decay_percentage: number;
}

export interface NFT {
  id: string;
  name: string;
  collection: string;
  owner: string;
  image_url?: string;
  energy: number;
  max_energy: number;
  current_energy: number;
  state: ObjectState;
  decay_percentage: number;
}

export interface SwapQuote {
  from_token: string;
  to_token: string;
  amount_in: number;
  amount_out: number;
  rate: number;
  price_impact: number;
}

const DEFAULT_BASE_URL = 'https://testnet.evaporchain.com';

class EvaporChainAPI {
  private baseUrl: string;

  constructor(baseUrl: string = DEFAULT_BASE_URL) {
    this.baseUrl = baseUrl.replace(/\/+$/, '');
  }

  setNetwork(network: 'testnet' | 'mainnet'): void {
    this.baseUrl = network === 'mainnet'
      ? 'https://rpc.evaporchain.io'
      : DEFAULT_BASE_URL;
  }

  private async get<T>(path: string): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`);
    if (!res.ok) throw new Error(`API ${res.status}`);
    return res.json();
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`API ${res.status}`);
    return res.json();
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

  async getTransactions(): Promise<Transaction[]> {
    return this.get('/api/transactions');
  }

  // ── Faucet ──

  async claimFaucet(address: string): Promise<{ success: boolean; balance: number; message?: string }> {
    return this.post('/api/faucet', { address });
  }

  // ── Objects ──

  async getObjects(owner?: string): Promise<ChainObject[]> {
    const all = await this.get<ChainObject[]>('/api/objects');
    return owner ? all.filter(o => o.owner === owner) : all;
  }

  async refreshObject(objectId: string, energyDeposit: number): Promise<TxResult> {
    return this.post('/api/tx/refresh', { object_id: objectId, energy_deposit: energyDeposit });
  }

  // ── NFTs ──

  async getNFTs(owner?: string): Promise<NFT[]> {
    const all = await this.get<NFT[]>('/api/nfts');
    return owner ? all.filter(n => n.owner === owner) : all;
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
}

export const api = new EvaporChainAPI();
export default EvaporChainAPI;

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
}

export const api = new EvaporChainAPI();
export default EvaporChainAPI;

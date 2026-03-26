/**
 * EvaporChain client — typed HTTP client for the EvaporChain node API.
 *
 * @example
 * ```typescript
 * import { EvaporClient } from '@evaporchain/sdk';
 *
 * const client = new EvaporClient('https://testnet.evaporchain.com');
 * const status = await client.getStatus();
 * console.log(`Block height: ${status.block_height}`);
 * ```
 */

import type {
  ChainStatus,
  BlockInfo,
  AccountInfo,
  StateObject,
  GhostRecord,
  EventRecord,
  TransferParams,
  CreateObjectParams,
  RefreshParams,
  NftToken,
  DeployedToken,
  StakingPool,
  DAOProposal,
  FaucetResult,
  AddressDetail,
} from './types';

export class EvaporClient {
  private baseUrl: string;
  private headers: Record<string, string>;

  constructor(baseUrl: string = 'http://localhost:8080', headers?: Record<string, string>) {
    // Remove trailing slash
    this.baseUrl = baseUrl.replace(/\/+$/, '');
    this.headers = {
      'Content-Type': 'application/json',
      ...headers,
    };
  }

  // ─────────────────── Internal ──────────────────────────────────────

  private async request<T>(path: string, options?: RequestInit): Promise<T> {
    const url = `${this.baseUrl}${path}`;
    const response = await fetch(url, {
      ...options,
      headers: { ...this.headers, ...options?.headers },
    });

    if (!response.ok) {
      const body = await response.text().catch(() => '');
      throw new EvaporError(
        `HTTP ${response.status}: ${response.statusText}`,
        response.status,
        body,
      );
    }

    return response.json() as Promise<T>;
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    return this.request<T>(path, {
      method: 'POST',
      body: JSON.stringify(body),
    });
  }

  // ─────────────────── Chain Queries ─────────────────────────────────

  /** Get node status: block height, epoch, peer count, state root. */
  async getStatus(): Promise<ChainStatus> {
    return this.request<ChainStatus>('/api/status');
  }

  /** Get recent blocks. */
  async getBlocks(limit: number = 50): Promise<BlockInfo[]> {
    return this.request<BlockInfo[]>(`/api/blocks?limit=${limit}`);
  }

  /** Get all accounts with balances. */
  async getAccounts(): Promise<AccountInfo[]> {
    return this.request<AccountInfo[]>('/api/accounts');
  }

  /** Get a specific account by address. */
  async getAccount(address: string): Promise<AccountInfo> {
    return this.request<AccountInfo>(`/api/accounts/${encodeURIComponent(address)}`);
  }

  /** Get all active state objects. */
  async getObjects(): Promise<StateObject[]> {
    return this.request<StateObject[]>('/api/objects');
  }

  /** Get all ghost records (evaporated objects). */
  async getGhosts(): Promise<GhostRecord[]> {
    return this.request<GhostRecord[]>('/api/ghosts');
  }

  /** Get recent events. */
  async getEvents(limit: number = 100): Promise<EventRecord[]> {
    return this.request<EventRecord[]>(`/api/events?limit=${limit}`);
  }

  /** Get address detail (balance, objects, NFTs, tokens). */
  async getAddress(address: string): Promise<AddressDetail> {
    return this.request<AddressDetail>(`/api/address/${encodeURIComponent(address)}`);
  }

  // ─────────────────── Transactions ──────────────────────────────────

  /** Submit a transfer transaction. */
  async transfer(params: TransferParams): Promise<{ success: boolean; message: string }> {
    return this.post('/api/transfer', params);
  }

  /** Create a new state object with energy and half-life. */
  async createObject(params: CreateObjectParams): Promise<{ success: boolean; object_id: string }> {
    return this.post('/api/create-object', params);
  }

  /** Refresh (add energy to) an existing object. */
  async refreshObject(params: RefreshParams): Promise<{ success: boolean; message: string }> {
    return this.post('/api/refresh', params);
  }

  // ─────────────────── Faucet ────────────────────────────────────────

  /** Request testnet tokens from the faucet. */
  async faucet(address: string): Promise<FaucetResult> {
    return this.post<FaucetResult>('/api/faucet', { address });
  }

  // ─────────────────── NFTs (EVR-721) ────────────────────────────────

  /** Get all NFTs. */
  async getNfts(): Promise<NftToken[]> {
    return this.request<NftToken[]>('/api/nft/list');
  }

  /** Get a specific NFT by ID. */
  async getNft(id: number): Promise<NftToken> {
    return this.request<NftToken>(`/api/nft/${id}`);
  }

  /** Mint a new NFT. */
  async mintNft(params: {
    name: string;
    collection: string;
    energy: number;
    half_life: number;
    owner: string;
  }): Promise<NftToken> {
    return this.post<NftToken>('/api/nft/mint', params);
  }

  /** Transfer an NFT to a new owner. */
  async transferNft(id: number, to: string): Promise<{ success: boolean }> {
    return this.post(`/api/nft/${id}/transfer`, { to });
  }

  /** Refresh (add energy to) an NFT. */
  async refreshNft(id: number, energy: number): Promise<{ success: boolean }> {
    return this.post(`/api/nft/${id}/refresh`, { energy });
  }

  // ─────────────────── Tokens (EVR-20) ───────────────────────────────

  /** Get all deployed tokens. */
  async getTokens(): Promise<DeployedToken[]> {
    return this.request<DeployedToken[]>('/api/tokens/list');
  }

  /** Deploy a new decaying token. */
  async deployToken(params: {
    name: string;
    symbol: string;
    initial_supply: number;
    decay_half_life: number;
    deployer: string;
  }): Promise<DeployedToken> {
    return this.post<DeployedToken>('/api/tokens/deploy', params);
  }

  /** Transfer tokens. */
  async transferToken(
    tokenId: number,
    from: string,
    to: string,
    amount: number,
  ): Promise<{ success: boolean }> {
    return this.post(`/api/tokens/${tokenId}/transfer`, { from, to, amount });
  }

  // ─────────────────── Staking ───────────────────────────────────────

  /** Get all staking pools. */
  async getStakingPools(): Promise<StakingPool[]> {
    return this.request<StakingPool[]>('/api/staking/pools');
  }

  /** Stake tokens in a pool. */
  async stake(poolId: number, address: string, amount: number): Promise<{ success: boolean }> {
    return this.post(`/api/staking/${poolId}/stake`, { address, amount });
  }

  /** Unstake tokens from a pool. */
  async unstake(poolId: number, address: string, amount: number): Promise<{ success: boolean }> {
    return this.post(`/api/staking/${poolId}/unstake`, { address, amount });
  }

  /** Claim staking rewards. */
  async claimRewards(poolId: number, address: string): Promise<{ success: boolean; amount: number }> {
    return this.post(`/api/staking/${poolId}/claim`, { address });
  }

  // ─────────────────── DAO Governance ────────────────────────────────

  /** Get all DAO proposals. */
  async getProposals(): Promise<DAOProposal[]> {
    return this.request<DAOProposal[]>('/api/dao/proposals');
  }

  /** Create a new DAO proposal. */
  async createProposal(params: {
    title: string;
    description: string;
    options: string[];
    voting_period: number;
    creator: string;
  }): Promise<DAOProposal> {
    return this.post<DAOProposal>('/api/dao/proposals', params);
  }

  /** Vote on a DAO proposal. */
  async vote(
    proposalId: number,
    voter: string,
    option: string,
    weight: number,
  ): Promise<{ success: boolean }> {
    return this.post(`/api/dao/proposals/${proposalId}/vote`, { voter, option, weight });
  }

  // ─────────────────── Utilities ─────────────────────────────────────

  /** Wait for a specific block height. Polls every `intervalMs`. */
  async waitForBlock(height: number, intervalMs: number = 1000, timeoutMs: number = 60000): Promise<ChainStatus> {
    const start = Date.now();
    while (Date.now() - start < timeoutMs) {
      const status = await this.getStatus();
      if (status.block_height >= height) {
        return status;
      }
      await new Promise(r => setTimeout(r, intervalMs));
    }
    throw new EvaporError(`Timeout waiting for block ${height}`, 0, '');
  }

  /** Calculate current energy of an object given decay parameters. */
  static calculateEnergy(initialEnergy: number, halfLife: number, elapsedEpochs: number): number {
    if (halfLife <= 0) return 0;
    return Math.floor(initialEnergy * Math.pow(2, -elapsedEpochs / halfLife));
  }

  /** Estimate when an object's energy will reach zero (enter grace period). */
  static estimateEvaporationEpoch(
    initialEnergy: number,
    halfLife: number,
    createdAt: number,
  ): number {
    if (halfLife <= 0 || initialEnergy <= 0) return createdAt;
    // Energy reaches 0 when 2^(-t/h) * E < 1, i.e., t > h * log2(E)
    const epochsUntilZero = Math.ceil(halfLife * Math.log2(initialEnergy));
    return createdAt + epochsUntilZero;
  }
}

/** Error type for EvaporChain API errors. */
export class EvaporError extends Error {
  constructor(
    message: string,
    public readonly statusCode: number,
    public readonly responseBody: string,
  ) {
    super(message);
    this.name = 'EvaporError';
  }
}

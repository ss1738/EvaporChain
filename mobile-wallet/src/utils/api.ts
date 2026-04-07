/**
 * EvaporChain API Client
 *
 * Communicates with the same RPC endpoints used by the CLI wallet
 * and browser extension.
 */

export interface ChainStatus {
  blockHeight: number;
  epoch: number;
  networkId: string;
  peerCount: number;
}

export interface Balance {
  available: string;
  staked: string;
  total: string;
}

export interface Transaction {
  hash: string;
  from: string;
  to: string;
  amount: string;
  fee: string;
  timestamp: number;
  status: 'confirmed' | 'pending' | 'failed';
  blockHeight?: number;
}

export interface TransactionSimulation {
  estimatedFee: string;
  estimatedEnergyCost: number;
  willSucceed: boolean;
  reason?: string;
}

export type ObjectState = 'Active' | 'Grace' | 'Ghost';

export interface ChainObject {
  id: string;
  name: string;
  owner: string;
  energy: number;
  maxEnergy: number;
  state: ObjectState;
  decayRate: number;
  lastRefreshed: number;
  estimatedGhostTime: number;
}

export interface NFT {
  id: string;
  name: string;
  imageUri: string;
  collectionName: string;
  energy: number;
  maxEnergy: number;
  state: ObjectState;
  decayRate: number;
  estimatedGhostTime: number;
}

const DEFAULT_ENDPOINTS: Record<string, string> = {
  testnet: 'https://testnet-rpc.evaporchain.io',
  mainnet: 'https://rpc.evaporchain.io',
};

class EvaporChainAPI {
  private baseUrl: string;
  private network: string;

  constructor(network: string = 'testnet') {
    this.network = network;
    this.baseUrl = DEFAULT_ENDPOINTS[network] || DEFAULT_ENDPOINTS.testnet;
  }

  setNetwork(network: string): void {
    this.network = network;
    this.baseUrl = DEFAULT_ENDPOINTS[network] || DEFAULT_ENDPOINTS.testnet;
  }

  getNetwork(): string {
    return this.network;
  }

  private async request<T>(method: string, params: unknown[] = []): Promise<T> {
    const response = await fetch(this.baseUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: Date.now(),
        method,
        params,
      }),
    });

    if (!response.ok) {
      throw new Error(`RPC error: ${response.status} ${response.statusText}`);
    }

    const data = await response.json();
    if (data.error) {
      throw new Error(`RPC error: ${data.error.message}`);
    }

    return data.result as T;
  }

  async getChainStatus(): Promise<ChainStatus> {
    return this.request<ChainStatus>('evap_chainStatus');
  }

  async getBalance(address: string): Promise<Balance> {
    return this.request<Balance>('evap_getBalance', [address]);
  }

  async getTransactions(address: string, limit: number = 20): Promise<Transaction[]> {
    return this.request<Transaction[]>('evap_getTransactions', [address, limit]);
  }

  async simulateTransaction(
    from: string,
    to: string,
    amount: string
  ): Promise<TransactionSimulation> {
    return this.request<TransactionSimulation>('evap_simulateTransaction', [
      { from, to, amount },
    ]);
  }

  async sendTransaction(signedTx: string): Promise<{ hash: string }> {
    return this.request<{ hash: string }>('evap_sendRawTransaction', [signedTx]);
  }

  async getObjects(owner: string): Promise<ChainObject[]> {
    return this.request<ChainObject[]>('evap_getObjects', [owner]);
  }

  async refreshObject(objectId: string, signedTx: string): Promise<{ hash: string }> {
    return this.request<{ hash: string }>('evap_refreshObject', [objectId, signedTx]);
  }

  async getNFTs(owner: string): Promise<NFT[]> {
    return this.request<NFT[]>('evap_getNFTs', [owner]);
  }

  async refreshNFT(nftId: string, signedTx: string): Promise<{ hash: string }> {
    return this.request<{ hash: string }>('evap_refreshNFT', [nftId, signedTx]);
  }
}

export const api = new EvaporChainAPI();
export default EvaporChainAPI;

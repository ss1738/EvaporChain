/**
 * EvaporChain REST API client for browser dApps.
 *
 * Provides typed access to all chain endpoints so dApps don't need
 * their own fetch wrappers. Works alongside the wallet provider:
 * - Provider: wallet connection, signing, sending transactions
 * - API client: reading chain data, submitting unsigned operations
 */

import type {
  NetworkId,
  NetworkConfig,
  ChainStatus,
  Balance,
  Transaction,
  TxResult,
  EvaporObject,
  Nft,
  NftCollection,
  MintNftParams,
  StakingInfo,
  Validator,
  SwapQuote,
  EnergyPool,
  PoolContribution,
  MortalMessage,
  ContractInfo,
  ScriptInfo,
  DeployContractParams,
  CallContractParams,
  DeployScriptParams,
  CallScriptParams,
  ScriptAbi,
  ContractEvent,
} from "./types";

const NETWORKS: Record<NetworkId, NetworkConfig> = {
  testnet: {
    id: "testnet",
    name: "EvaporChain Testnet",
    rpcUrl: "https://testnet.evaporchain.com",
  },
  mainnet: {
    id: "mainnet",
    name: "EvaporChain Mainnet",
    rpcUrl: "https://rpc.evaporchain.io",
  },
};

/**
 * Convert snake_case API responses to camelCase for JS consumption.
 */
function toCamelCase<T>(obj: unknown): T {
  if (Array.isArray(obj)) return obj.map((item) => toCamelCase(item)) as T;
  if (obj !== null && typeof obj === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(obj as Record<string, unknown>)) {
      const camelKey = key.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
      result[camelKey] = toCamelCase(value);
    }
    return result as T;
  }
  return obj as T;
}

export interface ApiClientOptions {
  /** Network to connect to. Default: "testnet" */
  network?: NetworkId;
  /** Custom RPC URL (overrides network default). */
  rpcUrl?: string;
  /** Request timeout in milliseconds. Default: 15000 */
  timeout?: number;
}

export class EvaporChainAPI {
  private _baseUrl: string;
  private _network: NetworkId;
  private _timeout: number;

  constructor(options: ApiClientOptions = {}) {
    this._network = options.network ?? "testnet";
    this._baseUrl = (options.rpcUrl ?? NETWORKS[this._network].rpcUrl).replace(/\/+$/, "");
    this._timeout = options.timeout ?? 15_000;
  }

  /** Current network ID. */
  get network(): NetworkId {
    return this._network;
  }

  /** Current network configuration. */
  get networkConfig(): NetworkConfig {
    return { ...NETWORKS[this._network], rpcUrl: this._baseUrl };
  }

  /** Switch networks. */
  setNetwork(network: NetworkId, customUrl?: string): void {
    this._network = network;
    this._baseUrl = (customUrl ?? NETWORKS[network].rpcUrl).replace(/\/+$/, "");
  }

  // ── Internal fetch helpers ──

  private async _get<T>(path: string): Promise<T> {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this._timeout);
    try {
      const res = await fetch(`${this._baseUrl}${path}`, { signal: controller.signal });
      if (!res.ok) throw new Error(`API ${res.status}: ${res.statusText}`);
      const json = await res.json();
      return toCamelCase<T>(json);
    } finally {
      clearTimeout(timer);
    }
  }

  private async _post<T>(path: string, body: unknown): Promise<T> {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this._timeout);
    try {
      const res = await fetch(`${this._baseUrl}${path}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal: controller.signal,
      });
      if (!res.ok) throw new Error(`API ${res.status}: ${res.statusText}`);
      const json = await res.json();
      return toCamelCase<T>(json);
    } finally {
      clearTimeout(timer);
    }
  }

  // ── Chain ──

  /** Get current chain status (block height, epoch, peer count, etc.). */
  async getChainStatus(): Promise<ChainStatus> {
    return this._get("/api/status");
  }

  // ── Accounts ──

  /** Get balance and nonce for an address. */
  async getBalance(address: string): Promise<Balance> {
    return this._get(`/api/address/${address}`);
  }

  // ── Transactions ──

  /** Submit a transfer transaction. */
  async transfer(from: string, to: string, amount: number, nonce: number): Promise<TxResult> {
    return this._post("/api/tx/transfer", { from, to, amount, nonce });
  }

  /** Get transaction history, optionally filtered by address. */
  async getTransactions(address?: string, limit?: number): Promise<Transaction[]> {
    const params = new URLSearchParams();
    if (address) params.set("address", address);
    if (limit) params.set("limit", limit.toString());
    const query = params.toString();
    return this._get(`/api/transactions${query ? `?${query}` : ""}`);
  }

  // ── Faucet ──

  /** Claim testnet tokens from the faucet. */
  async claimFaucet(address: string): Promise<{ success: boolean; balance: number; message?: string }> {
    return this._post("/api/faucet", { address });
  }

  // ── Objects ──

  /** Get all objects, optionally filtered by owner. */
  async getObjects(owner?: string): Promise<EvaporObject[]> {
    const path = owner ? `/api/objects?owner=${owner}` : "/api/objects";
    return this._get(path);
  }

  /** Get a single object by ID. */
  async getObject(objectId: string): Promise<EvaporObject> {
    return this._get(`/api/object/${objectId}`);
  }

  /** Refresh an object's energy to prevent decay. */
  async refreshObject(objectId: string, energyDeposit: number): Promise<TxResult> {
    return this._post("/api/tx/refresh", { object_id: objectId, energy_deposit: energyDeposit });
  }

  /** Batch refresh multiple objects at once. */
  async batchRefresh(items: Array<{ id: string; energy: number }>): Promise<TxResult[]> {
    return this._post("/api/tx/batch-refresh", { items });
  }

  /** Get objects filtered by state (Active, Grace, Ghost, Risen). */
  async getObjectsByState(owner: string, state: string): Promise<EvaporObject[]> {
    return this._get(`/api/objects?owner=${owner}&state=${state}`);
  }

  // ── NFTs ──

  /** Get all NFTs, optionally filtered by owner. */
  async getNFTs(owner?: string): Promise<Nft[]> {
    const path = owner ? `/api/nfts?owner=${owner}` : "/api/nfts";
    return this._get(path);
  }

  /** Get a single NFT by ID. */
  async getNFT(nftId: string): Promise<Nft> {
    return this._get(`/api/nft/${nftId}`);
  }

  /** Refresh an NFT's energy. */
  async refreshNFT(nftId: string, energy: number): Promise<TxResult> {
    return this._post("/api/nft/refresh", { nft_id: nftId, energy_deposit: energy });
  }

  /** Mint a new NFT. */
  async mintNFT(params: MintNftParams): Promise<TxResult & { nftId?: string }> {
    return this._post("/api/nft/mint", {
      name: params.name,
      collection: params.collection,
      image_uri: params.imageUri,
      energy: params.energy,
      half_life: params.halfLife,
      data: params.data,
    });
  }

  /** Transfer an NFT to another address. */
  async transferNFT(nftId: string, to: string): Promise<TxResult> {
    return this._post("/api/nft/transfer", { nft_id: nftId, to });
  }

  /** Get all NFT collections. */
  async getCollections(): Promise<NftCollection[]> {
    return this._get("/api/nft/collections");
  }

  // ── Contracts (template-based) ──

  /** List all deployed contracts. */
  async getContracts(): Promise<{ contracts: ContractInfo[] }> {
    return this._get("/api/contracts");
  }

  /** Get a single contract by ID (includes state). */
  async getContract(contractId: number): Promise<ContractInfo> {
    return this._get(`/api/contract/${contractId}`);
  }

  /** Deploy a new template contract. */
  async deployContract(deployer: string, params: DeployContractParams): Promise<TxResult> {
    return this._post("/api/tx/deploy-contract", {
      deployer,
      template: params.template,
      init_args: params.initArgs,
      energy: params.energy,
      half_life: params.halfLife,
      rules: params.rules,
    });
  }

  /** Call a method on a deployed contract. */
  async callContract(caller: string, params: CallContractParams): Promise<TxResult> {
    return this._post("/api/tx/call-contract", {
      caller,
      contract_id: params.contractId,
      method: params.method,
      args: params.args,
      epoch: params.epoch,
    });
  }

  /** Get event logs for a contract. */
  async getContractEvents(contractId: number): Promise<ContractEvent[]> {
    return this._get(`/api/contract/${contractId}/events`);
  }

  // ── Scripts (EvaporScript VM) ──

  /** List all deployed scripts. */
  async getScripts(): Promise<{ scripts: ScriptInfo[] }> {
    return this._get("/api/scripts");
  }

  /** Get a single script by ID. */
  async getScript(scriptId: number): Promise<ScriptInfo> {
    return this._get(`/api/script/${scriptId}`);
  }

  /** Get a script's ABI (methods, state fields, lifecycle hooks). */
  async getScriptAbi(scriptId: number): Promise<ScriptAbi> {
    return this._get(`/api/script/${scriptId}/abi`);
  }

  /** Deploy an EvaporScript program. */
  async deployScript(deployer: string, params: DeployScriptParams): Promise<TxResult> {
    return this._post("/api/tx/deploy-script", {
      deployer,
      source_code: params.sourceCode,
      energy: params.energy,
      half_life: params.halfLife,
    });
  }

  /** Call a method on a deployed script. */
  async callScript(caller: string, params: CallScriptParams): Promise<TxResult> {
    return this._post("/api/tx/call-script", {
      caller,
      contract_id: params.contractId,
      method: params.method,
      args: params.args,
      epoch: params.epoch,
    });
  }

  // ── Swap ──

  /** Get a quote for a token swap. */
  async getSwapQuote(fromToken: string, toToken: string, amount: number): Promise<SwapQuote> {
    return this._post("/api/swap/quote", { from_token: fromToken, to_token: toToken, amount });
  }

  /** Execute a token swap. */
  async executeSwap(fromToken: string, toToken: string, amount: number, slippage: number): Promise<TxResult> {
    return this._post("/api/swap/execute", { from_token: fromToken, to_token: toToken, amount, slippage });
  }

  // ── Staking ──

  /** Get staking info for an address. */
  async getStakingInfo(address: string): Promise<StakingInfo> {
    return this._get(`/api/staking/${address}`);
  }

  /** Get all validators. */
  async getValidators(): Promise<Validator[]> {
    return this._get("/api/validators");
  }

  /** Stake EVAP tokens. */
  async stake(from: string, amount: number, nonce: number): Promise<TxResult> {
    return this._post("/api/tx/stake", { from, amount, nonce });
  }

  /** Unstake EVAP tokens (begins unbonding period). */
  async unstake(from: string, amount: number, nonce: number): Promise<TxResult> {
    return this._post("/api/tx/unstake", { from, amount, nonce });
  }

  /** Claim staking rewards. */
  async claimRewards(from: string, nonce: number): Promise<TxResult> {
    return this._post("/api/tx/claim-rewards", { from, nonce });
  }

  // ── Energy Pools ──

  /** Get all energy pools. */
  async getPools(): Promise<EnergyPool[]> {
    return this._get("/api/pools");
  }

  /** Get a single energy pool by ID. */
  async getPool(poolId: string): Promise<EnergyPool> {
    return this._get(`/api/pool/${poolId}`);
  }

  /** Get contributors for an energy pool. */
  async getPoolContributors(poolId: string): Promise<PoolContribution[]> {
    return this._get(`/api/pool/${poolId}/contributors`);
  }

  /** Get activity log for an energy pool. */
  async getPoolActivity(poolId: string): Promise<Array<{ type: string; address: string; amount: number; timestamp: number }>> {
    return this._get(`/api/pool/${poolId}/activity`);
  }

  /** Create a new energy pool. */
  async createPool(name: string, creator: string, targetObject?: string): Promise<TxResult & { poolId?: string }> {
    return this._post("/api/pool/create", { name, creator, target_object: targetObject });
  }

  /** Stake energy into a pool. */
  async stakeToPool(poolId: string, address: string, amount: number): Promise<TxResult> {
    return this._post("/api/pool/stake", { pool_id: poolId, address, amount });
  }

  /** Unstake energy from a pool. */
  async unstakeFromPool(poolId: string, address: string, amount: number): Promise<TxResult> {
    return this._post("/api/pool/unstake", { pool_id: poolId, address, amount });
  }

  // ── Messages ──

  /** Send a mortal message. */
  async sendMessage(from: string, to: string, content: string, energy: number): Promise<TxResult & { messageId?: string }> {
    return this._post("/api/messages/send", { from, to, content, energy });
  }

  /** Get inbox messages for an address. */
  async getInbox(address: string): Promise<MortalMessage[]> {
    return this._get(`/api/messages/inbox/${address}`);
  }

  /** Get sent messages for an address. */
  async getSentMessages(address: string): Promise<MortalMessage[]> {
    return this._get(`/api/messages/sent/${address}`);
  }

  /** Get a single message by ID. */
  async getMessage(messageId: string): Promise<MortalMessage> {
    return this._get(`/api/message/${messageId}`);
  }

  /** Boost a message's energy to extend its life. */
  async boostMessage(messageId: string, energy: number): Promise<TxResult> {
    return this._post("/api/message/boost", { message_id: messageId, energy });
  }

  /** Get messaging stats for an address. */
  async getMessageStats(address: string): Promise<{ sent: number; received: number; active: number; ghosted: number }> {
    return this._get(`/api/messages/stats/${address}`);
  }
}

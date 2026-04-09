/**
 * TypeScript types for the EvaporChain wallet SDK.
 * Zero-dependency — no external imports.
 */

// ── Object states ──

export type ObjectState = "Active" | "Grace" | "Ghost" | "Risen";

// ── Chain objects ──

/** A decaying state object on the EvaporChain network. */
export interface EvaporObject {
  id: string;
  name: string;
  owner: string;
  energy: number;
  maxEnergy: number;
  halfLife: number;
  state: ObjectState;
  currentEnergy: number;
  decayPercentage: number;
  estimatedGhostTime: number;
  createdEpoch: number;
  lastRefreshed: number;
}

/** An NFT on EvaporChain — decays like all objects. */
export interface Nft {
  id: string;
  name: string;
  collection: string;
  collectionName: string;
  owner: string;
  imageUri?: string;
  energy: number;
  maxEnergy: number;
  currentEnergy: number;
  halfLife: number;
  decayPercentage: number;
  state: ObjectState;
  estimatedGhostTime: number;
  epochsRemaining: number;
  createdEpoch: number;
}

/** Account balance and nonce. */
export interface Balance {
  address: string;
  balance: number;
  nonce: number;
}

/** Chain status snapshot. */
export interface ChainStatus {
  chainName: string;
  version: string;
  blockHeight: number;
  epoch: number;
  activeObjects: number;
  ghostCount: number;
  totalEvaporated: number;
  peerCount: number;
}

// ── Transactions ──

/** Transaction request to be signed and submitted via wallet. */
export interface TransactionRequest {
  to: string;
  amount: number;
  /** Optional memo / data field. */
  data?: string;
}

/** Request to sign an arbitrary message. */
export interface SignMessageRequest {
  message: string;
  /** Optional: display label in the wallet popup. */
  label?: string;
}

/** Result of a successful wallet connection. */
export interface ConnectResult {
  address: string;
  publicKey: string;
}

/** Result of a submitted transaction (wallet-signed). */
export interface TransactionResult {
  hash: string;
  status: "pending" | "confirmed" | "failed";
}

/** Result of an API-submitted transaction. */
export interface TxResult {
  success: boolean;
  message: string;
  txHash?: string;
}

/** A historical transaction record from the chain. */
export interface Transaction {
  hash: string;
  type: string;
  detail: string;
  from: string;
  to: string;
  amount: string;
  timestamp: number;
}

/** Parameters for creating a new decaying object. */
export interface CreateObjectParams {
  name: string;
  energy: number;
  halfLife: number;
  data?: Record<string, unknown>;
}

// ── Staking ──

/** Staking status for an address. */
export interface StakingInfo {
  staked: number;
  rewards: number;
  isValidator: boolean;
  epoch: number;
  stakingStartEpoch?: number;
  unbondingAmount?: number;
  unbondingCompleteEpoch?: number;
}

/** A validator on the network. */
export interface Validator {
  address: string;
  name: string;
  stake: number;
  commission: number;
  uptime: number;
  status: "active" | "jailed" | "inactive";
}

// ── Swap ──

/** Quote for a token swap. */
export interface SwapQuote {
  fromToken: string;
  toToken: string;
  amountIn: number;
  amountOut: number;
  rate: number;
  priceImpact: number;
}

// ── Energy Pools ──

/** A community energy pool. */
export interface EnergyPool {
  id: string;
  name: string;
  creator: string;
  totalEnergy: number;
  contributors: number;
  targetObject?: string;
  createdEpoch: number;
}

/** A contribution to an energy pool. */
export interface PoolContribution {
  address: string;
  amount: number;
  timestamp: number;
}

// ── Messages ──

/** A mortal message (decays over time). */
export interface MortalMessage {
  id: string;
  from: string;
  to: string;
  content: string;
  energy: number;
  maxEnergy: number;
  currentEnergy: number;
  state: ObjectState;
  timestamp: number;
}

// ── NFT Collections ──

/** An NFT collection. */
export interface NftCollection {
  id: string;
  name: string;
  creator: string;
  count: number;
  floorEnergy: number;
}

/** Parameters for minting an NFT. */
export interface MintNftParams {
  name: string;
  collection: string;
  imageUri?: string;
  energy: number;
  halfLife: number;
  data?: Record<string, unknown>;
}

// ── Errors ──

/** Error codes returned by the EvaporChain wallet SDK. */
export enum EvaporChainErrorCode {
  /** The EvaporChain wallet extension is not installed. */
  NOT_INSTALLED = "NOT_INSTALLED",
  /** The user rejected the request in the wallet popup. */
  USER_REJECTED = "USER_REJECTED",
  /** Network or RPC error communicating with the chain. */
  NETWORK_ERROR = "NETWORK_ERROR",
  /** Insufficient EVAP balance for the transaction. */
  INSUFFICIENT_BALANCE = "INSUFFICIENT_BALANCE",
  /** The requested object was not found on-chain. */
  OBJECT_NOT_FOUND = "OBJECT_NOT_FOUND",
}

/** Typed error thrown by the EvaporChain wallet SDK. */
export class EvaporChainError extends Error {
  public code: EvaporChainErrorCode;
  public details?: unknown;

  constructor(message: string, code: EvaporChainErrorCode, details?: unknown) {
    super(message);
    this.name = "EvaporChainError";
    this.code = code;
    this.details = details;
  }
}

// ── Provider interface (injected by the extension) ──

/**
 * The provider interface injected into `window.evaporchain` by the
 * EvaporChain browser extension. The SDK wraps this into a cleaner API.
 *
 * Canonical shape — all dApps should use the SDK instead of accessing
 * window.evaporchain directly to avoid interface mismatches.
 */
export interface InjectedProvider {
  isEvaporChain: true;
  connect(): Promise<ConnectResult>;
  disconnect(): Promise<void>;
  getAccounts(): Promise<string[]>;
  getBalance(address?: string): Promise<Balance>;
  getObjects(address?: string): Promise<EvaporObject[]>;
  getNfts(address?: string): Promise<Nft[]>;
  sendTransaction(tx: TransactionRequest): Promise<TransactionResult>;
  signMessage(request: SignMessageRequest): Promise<{ signature: string }>;
  refreshObject(objectId: string, energy: number): Promise<TransactionResult>;
  createObject(params: CreateObjectParams): Promise<TransactionResult & { objectId: string }>;
  getChainStatus(): Promise<ChainStatus>;
  on(event: string, handler: (...args: unknown[]) => void): void;
  off(event: string, handler: (...args: unknown[]) => void): void;
}

/** Extend the Window interface so TypeScript knows about window.evaporchain. */
declare global {
  interface Window {
    evaporchain?: InjectedProvider;
  }
}

// ── SDK events ──

export type EvaporChainEvent =
  | "connect"
  | "disconnect"
  | "accountsChanged"
  | "chainChanged";

// ── Network configuration ──

export type NetworkId = "testnet" | "mainnet";

export interface NetworkConfig {
  id: NetworkId;
  name: string;
  rpcUrl: string;
}

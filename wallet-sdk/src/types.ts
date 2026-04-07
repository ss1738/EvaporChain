/**
 * TypeScript types for the EvaporChain wallet SDK.
 * Zero-dependency — no external imports.
 */

// ── Chain objects ──

/** A decaying state object on the EvaporChain network. */
export interface EvaporObject {
  id: string;
  name: string;
  owner: string;
  energy: number;
  maxEnergy: number;
  halfLife: number;
  state: "Active" | "Grace" | "Ghost" | "Risen";
  currentEnergy: number;
  decayPercentage: number;
  createdEpoch: number;
  lastRefreshed: number;
}

/** An NFT on EvaporChain — decays like all objects. */
export interface Nft {
  id: string;
  name: string;
  collection: string;
  owner: string;
  imageUrl?: string;
  energy: number;
  maxEnergy: number;
  currentEnergy: number;
  halfLife: number;
  decayPercentage: number;
  state: "Active" | "Grace" | "Ghost";
  epochsRemaining: number;
  createdEpoch: number;
}

/** Account balance and nonce. */
export interface Balance {
  balance: number;
  nonce: number;
}

/** Chain status snapshot. */
export interface ChainStatus {
  blockHeight: number;
  epoch: number;
  activeObjects: number;
  ghostCount: number;
  totalEvaporated: number;
  peerCount: number;
}

// ── Transactions ──

/** Transaction request to be signed and submitted. */
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

/** Result of a submitted transaction. */
export interface TransactionResult {
  hash: string;
  status: "pending" | "confirmed" | "failed";
}

/** Parameters for creating a new decaying object. */
export interface CreateObjectParams {
  name: string;
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

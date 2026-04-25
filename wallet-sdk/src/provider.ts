/**
 * EvaporChainProvider — main entry point for dApps to interact with the
 * EvaporChain browser wallet extension.
 *
 * Detects window.evaporchain automatically and wraps it into a typed,
 * event-driven API.
 *
 * @example
 * ```ts
 * import { EvaporChainProvider } from "@evaporchain/wallet-sdk";
 *
 * const provider = new EvaporChainProvider();
 * const { address } = await provider.connect();
 * const { balance } = await provider.getBalance();
 * ```
 */

import {
  EvaporChainError,
  EvaporChainErrorCode,
  type InjectedProvider,
  type ConnectResult,
  type TransactionResult,
  type TransactionRequest,
  type SignMessageRequest,
  type Balance,
  type EvaporObject,
  type Nft,
  type ChainStatus,
  type CreateObjectParams,
  type EvaporChainEvent,
  type DeployContractParams,
  type CallContractParams,
  type DeployScriptParams,
  type CallScriptParams,
  type TxResult,
  type ContractInfo,
  type ScriptInfo,
  type ScriptAbi,
  type ContractEvent,
} from "./types";

import { EvaporChainAPI } from "./api";

type EventHandler = (...args: unknown[]) => void;

export class EvaporChainProvider {
  private _provider: InjectedProvider | null = null;
  private _address: string | null = null;
  private _publicKey: string | null = null;
  private _connected = false;
  private _listeners: Map<EvaporChainEvent, Set<EventHandler>> = new Map();
  private _api: EvaporChainAPI;

  constructor(apiOptions?: { rpcUrl?: string; network?: "testnet" | "mainnet" }) {
    this._detectProvider();
    this._api = new EvaporChainAPI(apiOptions);
  }

  // ── Connection ──

  /**
   * Connect to the EvaporChain wallet.
   * Opens the wallet popup for user approval.
   *
   * @throws {EvaporChainError} NOT_INSTALLED if wallet is not detected
   * @throws {EvaporChainError} USER_REJECTED if user denies the request
   */
  async connect(): Promise<ConnectResult> {
    const provider = this._requireProvider();

    try {
      const result = await provider.connect();
      this._address = result.address;
      this._publicKey = result.publicKey;
      this._connected = true;
      this._emit("connect", result);
      return result;
    } catch (err: unknown) {
      if (this._isUserRejection(err)) {
        throw new EvaporChainError(
          "User rejected the connection request",
          EvaporChainErrorCode.USER_REJECTED,
        );
      }
      throw this._wrapError(err);
    }
  }

  /**
   * Disconnect from the wallet.
   * Clears local state and notifies listeners.
   */
  async disconnect(): Promise<void> {
    if (this._provider && this._connected) {
      try {
        await this._provider.disconnect();
      } catch {
        // Best-effort disconnect
      }
    }
    this._address = null;
    this._publicKey = null;
    this._connected = false;
    this._emit("disconnect");
  }

  /** Whether the provider is currently connected. */
  get connected(): boolean {
    return this._connected;
  }

  /** The connected address, or null if not connected. */
  get address(): string | null {
    return this._address;
  }

  /** The connected public key, or null if not connected. */
  get publicKey(): string | null {
    return this._publicKey;
  }

  // ── Accounts ──

  /**
   * Get all accounts available in the wallet.
   * @throws {EvaporChainError} NOT_INSTALLED if wallet is not detected
   */
  async getAccounts(): Promise<string[]> {
    const provider = this._requireProvider();
    try {
      return await provider.getAccounts();
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  // ── Balances ──

  /**
   * Get balance and nonce for an address.
   * If no address is provided, uses the connected account.
   *
   * @throws {EvaporChainError} NOT_INSTALLED if wallet is not detected
   * @throws {EvaporChainError} NETWORK_ERROR on RPC failure
   */
  async getBalance(address?: string): Promise<Balance> {
    const provider = this._requireProvider();
    try {
      return await provider.getBalance(address);
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  // ── Objects ──

  /**
   * Get all decaying state objects owned by an address.
   * If no address is provided, uses the connected account.
   */
  async getObjects(address?: string): Promise<EvaporObject[]> {
    const provider = this._requireProvider();
    try {
      return await provider.getObjects(address);
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  /**
   * Get all NFTs owned by an address.
   * If no address is provided, uses the connected account.
   */
  async getNfts(address?: string): Promise<Nft[]> {
    const provider = this._requireProvider();
    try {
      return await provider.getNfts(address);
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  // ── Transactions ──

  /**
   * Send a transaction through the wallet.
   * Opens the wallet popup for user approval and signing.
   *
   * @throws {EvaporChainError} USER_REJECTED if user denies
   * @throws {EvaporChainError} INSUFFICIENT_BALANCE if balance too low
   */
  async sendTransaction(tx: TransactionRequest): Promise<TransactionResult> {
    const provider = this._requireProvider();
    try {
      return await provider.sendTransaction(tx);
    } catch (err) {
      if (this._isUserRejection(err)) {
        throw new EvaporChainError(
          "User rejected the transaction",
          EvaporChainErrorCode.USER_REJECTED,
        );
      }
      throw this._wrapError(err);
    }
  }

  /**
   * Sign an arbitrary message with the wallet's private key.
   * Uses ML-DSA post-quantum signatures.
   *
   * @throws {EvaporChainError} USER_REJECTED if user denies
   */
  async signMessage(message: string): Promise<{ signature: string }> {
    const provider = this._requireProvider();
    try {
      return await provider.signMessage({ message });
    } catch (err) {
      if (this._isUserRejection(err)) {
        throw new EvaporChainError(
          "User rejected the signature request",
          EvaporChainErrorCode.USER_REJECTED,
        );
      }
      throw this._wrapError(err);
    }
  }

  // ── Object operations (unique to EvaporChain) ──

  /**
   * Refresh a decaying object by depositing energy.
   * Prevents the object from entering grace period / evaporating.
   *
   * @param objectId - The hex ID of the object to refresh
   * @param energy - Amount of energy (EVAP) to deposit
   * @throws {EvaporChainError} OBJECT_NOT_FOUND if object does not exist
   * @throws {EvaporChainError} INSUFFICIENT_BALANCE if balance too low
   */
  async refreshObject(objectId: string, energy: number): Promise<TransactionResult> {
    const provider = this._requireProvider();
    try {
      return await provider.refreshObject(objectId, energy);
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  /**
   * Create a new decaying state object.
   *
   * @param params - Object creation parameters (name, energy, halfLife)
   * @returns Transaction result including the new object's ID
   * @throws {EvaporChainError} INSUFFICIENT_BALANCE if balance too low
   */
  async createObject(params: CreateObjectParams): Promise<TransactionResult & { objectId: string }> {
    const provider = this._requireProvider();
    try {
      return await provider.createObject(params);
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  // ── Contracts ──

  /** List all deployed contracts. */
  async getContracts(): Promise<ContractInfo[]> {
    try {
      const result = await this._api.getContracts();
      return result.contracts;
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  /** Get a single contract by ID (includes on-chain state). */
  async getContract(contractId: number): Promise<ContractInfo> {
    try {
      return await this._api.getContract(contractId);
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  /**
   * Deploy a template contract.
   * Requires wallet connection for the deployer address.
   */
  async deployContract(params: DeployContractParams): Promise<TxResult> {
    this._requireConnected();
    try {
      return await this._api.deployContract(this._address!, params);
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  /**
   * Call a method on a deployed contract.
   * Requires wallet connection for the caller address.
   */
  async callContract(params: CallContractParams): Promise<TxResult> {
    this._requireConnected();
    try {
      return await this._api.callContract(this._address!, params);
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  /** Get event logs for a contract. */
  async getContractEvents(contractId: number): Promise<ContractEvent[]> {
    try {
      return await this._api.getContractEvents(contractId);
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  // ── Scripts (EvaporScript VM) ──

  /** List all deployed EvaporScript programs. */
  async getScripts(): Promise<ScriptInfo[]> {
    try {
      const result = await this._api.getScripts();
      return result.scripts;
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  /** Get a script's ABI (methods, state, lifecycle hooks). */
  async getScriptAbi(scriptId: number): Promise<ScriptAbi> {
    try {
      return await this._api.getScriptAbi(scriptId);
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  /**
   * Deploy an EvaporScript program.
   * Requires wallet connection for the deployer address.
   */
  async deployScript(params: DeployScriptParams): Promise<TxResult> {
    this._requireConnected();
    try {
      return await this._api.deployScript(this._address!, params);
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  /**
   * Call a method on a deployed EvaporScript program.
   * Requires wallet connection for the caller address.
   */
  async callScript(params: CallScriptParams): Promise<TxResult> {
    this._requireConnected();
    try {
      return await this._api.callScript(this._address!, params);
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  // ── Chain status ──

  /**
   * Get the current chain status (block height, epoch, etc.).
   */
  async getChainStatus(): Promise<ChainStatus> {
    const provider = this._requireProvider();
    try {
      return await provider.getChainStatus();
    } catch (err) {
      throw this._wrapError(err);
    }
  }

  // ── Events ──

  /**
   * Subscribe to provider events.
   *
   * @param event - Event name: "connect", "disconnect", "accountsChanged", "chainChanged"
   * @param handler - Callback function
   */
  on(event: EvaporChainEvent, handler: EventHandler): void {
    let handlers = this._listeners.get(event);
    if (!handlers) {
      handlers = new Set();
      this._listeners.set(event, handlers);
    }
    handlers.add(handler);

    // Also register on the injected provider for forwarding
    if (this._provider) {
      this._provider.on(event, handler);
    }
  }

  /**
   * Unsubscribe from provider events.
   *
   * @param event - Event name
   * @param handler - The same callback reference passed to `on()`
   */
  off(event: EvaporChainEvent, handler: EventHandler): void {
    const handlers = this._listeners.get(event);
    if (handlers) {
      handlers.delete(handler);
    }

    if (this._provider) {
      this._provider.off(event, handler);
    }
  }

  // ── Internal ──

  private _requireConnected(): void {
    if (!this._connected || !this._address) {
      throw new EvaporChainError(
        "Wallet not connected. Call connect() first.",
        EvaporChainErrorCode.NOT_INSTALLED,
      );
    }
  }

  private _detectProvider(): void {
    if (typeof window !== "undefined" && window.evaporchain?.isEvaporChain) {
      this._provider = window.evaporchain;
    }
  }

  private _requireProvider(): InjectedProvider {
    // Re-detect in case extension loaded after SDK
    if (!this._provider) {
      this._detectProvider();
    }

    if (!this._provider) {
      throw new EvaporChainError(
        "EvaporChain wallet extension is not installed. Download it at https://evaporchain.com/wallet",
        EvaporChainErrorCode.NOT_INSTALLED,
      );
    }

    return this._provider;
  }

  private _emit(event: EvaporChainEvent, ...args: unknown[]): void {
    const handlers = this._listeners.get(event);
    if (handlers) {
      for (const handler of handlers) {
        try {
          handler(...args);
        } catch {
          // Don't let listener errors break the provider
        }
      }
    }
  }

  private _isUserRejection(err: unknown): boolean {
    if (err instanceof Error) {
      const msg = err.message.toLowerCase();
      return msg.includes("rejected") || msg.includes("denied") || msg.includes("cancelled");
    }
    return false;
  }

  private _wrapError(err: unknown): EvaporChainError {
    if (err instanceof EvaporChainError) return err;

    const message = err instanceof Error ? err.message : String(err);

    if (message.toLowerCase().includes("insufficient")) {
      return new EvaporChainError(message, EvaporChainErrorCode.INSUFFICIENT_BALANCE, err);
    }
    if (message.toLowerCase().includes("not found")) {
      return new EvaporChainError(message, EvaporChainErrorCode.OBJECT_NOT_FOUND, err);
    }

    return new EvaporChainError(message, EvaporChainErrorCode.NETWORK_ERROR, err);
  }
}

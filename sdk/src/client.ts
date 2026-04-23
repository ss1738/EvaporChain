import type {
  ChainStatus,
  StateObject,
  Account,
  Block,
  Ghost,
  Contract,
  TxResult,
  FaucetResult,
  DecayEstimate,
  StatsSummary,
  StatsTimeline,
  NetworkInfo,
  EventRecord,
  ClientOptions,
  BatchTxItem,
  BatchResponse,
  WsEvent,
  WsTopic,
  ContractEventLog,
  ContractAbi,
  StateProofResponse,
  TxInclusionProof,
  CompactHeader,
} from "./types";
import { EventEmitter } from "events";

/** Error thrown by the EvaporChain client. */
export class EvaporChainError extends Error {
  public status: number;
  constructor(message: string, status: number) {
    super(message);
    this.name = "EvaporChainError";
    this.status = status;
  }
}

/**
 * EvaporChain SDK client.
 *
 * Wraps the EvaporChain HTTP API into clean TypeScript methods.
 *
 * ```ts
 * const chain = new EvaporChain("https://testnet.evaporchain.com");
 * const status = await chain.getStatus();
 * console.log("Block height:", status.block_height);
 * ```
 */
export class EvaporChain {
  private baseUrl: string;
  private timeout: number;
  private maxRetries: number;
  private retryDelay: number;
  private wsReconnectDelay: number;
  private wsMaxReconnects: number;
  private ws: WebSocket | null = null;
  private wsReconnectCount = 0;
  private wsReconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private wsShouldReconnect = false;
  private wsTopics: WsTopic[] = ["all"];
  private emitter = new EventEmitter();

  constructor(urlOrOptions?: string | ClientOptions) {
    if (typeof urlOrOptions === "string") {
      this.baseUrl = urlOrOptions.replace(/\/+$/, "");
      this.timeout = 10_000;
      this.maxRetries = 3;
      this.retryDelay = 500;
      this.wsReconnectDelay = 3_000;
      this.wsMaxReconnects = 10;
    } else {
      this.baseUrl = (urlOrOptions?.baseUrl ?? "https://testnet.evaporchain.com").replace(/\/+$/, "");
      this.timeout = urlOrOptions?.timeout ?? 10_000;
      this.maxRetries = urlOrOptions?.maxRetries ?? 3;
      this.retryDelay = urlOrOptions?.retryDelay ?? 500;
      this.wsReconnectDelay = urlOrOptions?.wsReconnectDelay ?? 3_000;
      this.wsMaxReconnects = urlOrOptions?.wsMaxReconnects ?? 10;
    }
  }

  // ── Internal helpers ──

  private async request<T>(path: string, init?: RequestInit): Promise<T> {
    let lastError: Error | null = null;

    for (let attempt = 0; attempt <= this.maxRetries; attempt++) {
      if (attempt > 0) {
        const delay = this.retryDelay * Math.pow(2, attempt - 1);
        await new Promise((r) => setTimeout(r, delay));
      }

      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), this.timeout);

      try {
        const res = await fetch(`${this.baseUrl}${path}`, {
          ...init,
          signal: controller.signal,
          headers: {
            "Content-Type": "application/json",
            ...init?.headers,
          },
        });

        if (!res.ok) {
          const text = await res.text().catch(() => "");
          const err = new EvaporChainError(
            `HTTP ${res.status}: ${text || res.statusText}`,
            res.status
          );
          if (res.status === 429 || res.status >= 500) {
            lastError = err;
            continue;
          }
          throw err;
        }

        return (await res.json()) as T;
      } catch (err) {
        if (err instanceof EvaporChainError && err.status > 0 && err.status < 500 && err.status !== 429) {
          throw err;
        }
        lastError = err as Error;
        if (attempt === this.maxRetries) break;
      } finally {
        clearTimeout(timer);
      }
    }

    throw lastError ?? new EvaporChainError("Request failed after retries", 0);
  }

  private get<T>(path: string): Promise<T> {
    return this.request<T>(path);
  }

  private post<T>(path: string, body: unknown): Promise<T> {
    return this.request<T>(path, {
      method: "POST",
      body: JSON.stringify(body),
    });
  }

  // ── Chain Status ──

  /** Get current chain status (block height, epoch, object counts, uptime). */
  async getStatus(): Promise<ChainStatus> {
    return this.get<ChainStatus>("/api/status");
  }

  // ── Accounts ──

  /** List all accounts sorted by balance descending. */
  async getAccounts(): Promise<Account[]> {
    return this.get<Account[]>("/api/accounts");
  }

  // ── Objects ──

  /** List all state objects (active, grace, risen). */
  async getObjects(): Promise<StateObject[]> {
    return this.get<StateObject[]>("/api/objects");
  }

  /** Get a single state object by hex ID (64 character hex string). */
  async getObject(id: string): Promise<StateObject> {
    return this.get<StateObject>(`/api/object/${id}`);
  }

  /** List all ghost records (evaporated objects). */
  async getGhosts(): Promise<Ghost[]> {
    return this.get<Ghost[]>("/api/ghosts");
  }

  // ── Blocks ──

  /** Get recent blocks, most recent first. */
  async getBlocks(limit?: number): Promise<Block[]> {
    const q = limit ? `?limit=${limit}` : "";
    return this.get<Block[]>(`/api/blocks${q}`);
  }

  /** Get a specific block by height. */
  async getBlock(height: number): Promise<Block> {
    return this.get<Block>(`/api/block/${height}`);
  }

  // ── Events ──

  /** Get recent chain events (grace, evaporation, transfer, etc.). */
  async getEvents(limit?: number): Promise<EventRecord[]> {
    const q = limit ? `?limit=${limit}` : "";
    const res = await this.get<{ events: EventRecord[] }>(`/api/events${q}`);
    return res.events;
  }

  // ── Stats ──

  /** Get aggregate stats summary. */
  async getStatsSummary(): Promise<StatsSummary> {
    return this.get<StatsSummary>("/api/stats/summary");
  }

  /** Get epoch-by-epoch timeline (active count, ghost count, total energy). */
  async getStatsTimeline(): Promise<StatsTimeline> {
    return this.get<StatsTimeline>("/api/stats/timeline");
  }

  // ── Network ──

  /** Get network peer count. */
  async getNetwork(): Promise<NetworkInfo> {
    return this.get<NetworkInfo>("/api/network");
  }

  // ── Transactions ──

  /**
   * Submit a transfer transaction.
   * @param from - Sender address byte (0-255)
   * @param to - Recipient address byte (0-255)
   * @param amount - Amount to transfer
   * @param nonce - Sender nonce (default 0)
   */
  async transfer(from: number, to: number, amount: number, nonce: number = 0): Promise<TxResult> {
    return this.post<TxResult>("/api/tx/transfer", { from, to, amount, nonce });
  }

  /**
   * Create a new decaying state object.
   * @param creator - Creator address byte (0-255)
   * @param objectId - Object ID byte (0-255)
   * @param energy - Initial energy
   * @param halfLife - Half-life in epochs
   */
  async createObject(creator: number, objectId: number, energy: number, halfLife: number): Promise<TxResult> {
    return this.post<TxResult>("/api/tx/create-object", {
      creator,
      object_id: objectId,
      energy,
      half_life: halfLife,
    });
  }

  /**
   * Refresh an object's energy (prevents evaporation).
   * @param objectId - Object ID byte (0-255)
   * @param energy - Energy to deposit
   */
  async refreshObject(objectId: number, energy: number): Promise<TxResult> {
    return this.post<TxResult>("/api/tx/refresh", {
      object_id: objectId,
      energy_deposit: energy,
    });
  }

  /**
   * Resurrect a ghost object by depositing energy.
   * @param objectId - Object ID byte (0-255)
   * @param energy - Energy to deposit for resurrection
   */
  async resurrectObject(objectId: number, energy: number): Promise<TxResult> {
    return this.post<TxResult>("/api/tx/resurrect", {
      object_id: objectId,
      energy_deposit: energy,
    });
  }

  // ── Contracts ──

  /** List all deployed contracts. */
  async getContracts(): Promise<Contract[]> {
    const res = await this.get<{ contracts: Contract[] }>("/api/contracts");
    return res.contracts;
  }

  /** Get a single contract by ID. */
  async getContract(contractId: number): Promise<Contract> {
    return this.get<Contract>(`/api/contract/${contractId}`);
  }

  /**
   * Deploy a smart contract.
   * @param deployer - Deployer address byte (0-255)
   * @param template - Contract template name (e.g. "DecayingToken")
   * @param initArgs - Initialization arguments
   * @param energy - Initial energy
   * @param halfLife - Half-life in epochs
   */
  async deployContract(
    deployer: number,
    template: string,
    initArgs: Record<string, unknown>,
    energy: number,
    halfLife: number
  ): Promise<TxResult> {
    return this.post<TxResult>("/api/tx/deploy-contract", {
      deployer,
      template,
      init_args: initArgs,
      energy,
      half_life: halfLife,
    });
  }

  /**
   * Call a method on a deployed contract.
   * @param caller - Caller address byte (0-255)
   * @param contractId - Contract ID
   * @param method - Method name
   * @param args - Method arguments
   * @param epoch - Current epoch
   */
  async callContract(
    caller: number,
    contractId: number,
    method: string,
    args: Record<string, unknown>,
    epoch: number
  ): Promise<TxResult> {
    return this.post<TxResult>("/api/tx/call-contract", {
      caller,
      contract_id: contractId,
      method,
      args,
      epoch,
    });
  }

  // ── Batch Transactions ──

  /**
   * Submit multiple transactions in a single request (max 100).
   *
   * ```ts
   * const result = await chain.batch([
   *   { type: "transfer", from: 0, to: 1, amount: 500, nonce: 0 },
   *   { type: "create_object", creator: 0, object_id: 10, energy: 1000, half_life: 50 },
   *   { type: "refresh", object_id: 5, energy_deposit: 200 },
   * ]);
   * console.log(`${result.submitted} submitted, ${result.failed} failed`);
   * ```
   */
  async batch(transactions: BatchTxItem[]): Promise<BatchResponse> {
    return this.post<BatchResponse>("/api/tx/batch", { transactions });
  }

  // ── Contract Event Logs ──

  /**
   * Query contract event logs by contract ID.
   * @param contractId - Contract ID
   * @param options - Optional filters: eventName, fromBlock, toBlock, limit
   */
  async getContractEvents(
    contractId: number,
    options?: { eventName?: string; fromBlock?: number; toBlock?: number; limit?: number },
  ): Promise<{ contract_id: number; count: number; events: ContractEventLog[] }> {
    const params = new URLSearchParams();
    if (options?.eventName) params.set("event_name", options.eventName);
    if (options?.fromBlock !== undefined) params.set("from_block", String(options.fromBlock));
    if (options?.toBlock !== undefined) params.set("to_block", String(options.toBlock));
    if (options?.limit !== undefined) params.set("limit", String(options.limit));
    const qs = params.toString();
    return this.request<{ contract_id: number; count: number; events: ContractEventLog[] }>(
      `/api/contract/${contractId}/events${qs ? `?${qs}` : ""}`,
    );
  }

  /**
   * Get all contract events in a specific block.
   */
  async getBlockEvents(blockNumber: number): Promise<{ block_number: number; count: number; events: ContractEventLog[] }> {
    return this.request<{ block_number: number; count: number; events: ContractEventLog[] }>(
      `/api/block/${blockNumber}/events`,
    );
  }

  /**
   * Get event index stats.
   */
  async getEventIndexStats(): Promise<{ indexed_events: number; indexed_transactions: number }> {
    return this.request<{ indexed_events: number; indexed_transactions: number }>("/api/event-index/stats");
  }

  /**
   * Get the ABI (typed interface) of a deployed EvaporScript contract.
   */
  async getScriptAbi(contractId: number): Promise<ContractAbi> {
    return this.request<ContractAbi>(`/api/script/${contractId}/abi`);
  }

  // ── Light Client Verification ──

  /**
   * Get a Verkle state proof for an account.
   */
  async getAccountStateProof(addressHex: string): Promise<StateProofResponse> {
    return this.request<StateProofResponse>(`/api/light/state-proof/account/${addressHex}`);
  }

  /**
   * Get a Verkle state proof for a state object.
   */
  async getObjectStateProof(objectIdHex: string): Promise<StateProofResponse> {
    return this.request<StateProofResponse>(`/api/light/state-proof/object/${objectIdHex}`);
  }

  /**
   * Get a transaction inclusion proof (Merkle proof within block).
   */
  async getTxInclusionProof(blockNumber: number, txIndex: number): Promise<{ proof: TxInclusionProof; valid: boolean }> {
    return this.request<{ proof: TxInclusionProof; valid: boolean }>(
      `/api/light/tx-proof/${blockNumber}/${txIndex}`,
    );
  }

  /**
   * Verify a transaction inclusion proof.
   */
  async verifyTxProof(proof: TxInclusionProof): Promise<{ valid: boolean }> {
    return this.post<{ valid: boolean }>("/api/light/verify-tx-proof", proof);
  }

  /**
   * Get compact block headers for light client sync.
   */
  async getLightHeaders(options?: { from?: number; to?: number; limit?: number }): Promise<{
    count: number;
    from: number;
    to: number;
    headers: CompactHeader[];
  }> {
    const params = new URLSearchParams();
    if (options?.from !== undefined) params.set("from", String(options.from));
    if (options?.to !== undefined) params.set("to", String(options.to));
    if (options?.limit !== undefined) params.set("limit", String(options.limit));
    const qs = params.toString();
    return this.request(`/api/light/headers${qs ? `?${qs}` : ""}`);
  }

  // ── Faucet ──

  /**
   * Claim testnet tokens from the faucet.
   * @param address - Address byte (0-255). Rate limited to once per hour.
   */
  async claimFaucet(address: number): Promise<FaucetResult> {
    return this.post<FaucetResult>("/api/faucet", { address });
  }

  // ── Utilities (unique to EvaporChain) ──

  /**
   * Wait for the chain to reach a target block height.
   * Polls every 2 seconds until the target is reached or timeout expires.
   * @param targetHeight - Block height to wait for
   * @param timeout - Timeout in ms (default: 30000)
   */
  async waitForBlock(targetHeight: number, timeout: number = 30_000): Promise<Block> {
    const start = Date.now();
    while (Date.now() - start < timeout) {
      const status = await this.getStatus();
      if (status.block_height >= targetHeight) {
        return this.getBlock(targetHeight);
      }
      await new Promise((r) => setTimeout(r, 2000));
    }
    throw new EvaporChainError(
      `Timed out waiting for block ${targetHeight}`,
      408
    );
  }

  /**
   * Watch an object's energy decay in real-time.
   * Calls the callback every `interval` ms with updated object state.
   * Returns a function to stop watching.
   *
   * ```ts
   * const stop = chain.watchObject("0x1000...", (obj) => {
   *   console.log(`Energy: ${obj.current_energy}/${obj.max_energy}`);
   *   if (obj.state === "Ghost") stop();
   * });
   * ```
   *
   * @param objectId - 64-char hex object ID
   * @param callback - Called with updated object on each poll
   * @param interval - Poll interval in ms (default: 2000)
   * @returns Stop function
   */
  watchObject(
    objectId: string,
    callback: (obj: StateObject) => void,
    interval: number = 2000
  ): () => void {
    let running = true;

    const poll = async () => {
      while (running) {
        try {
          const obj = await this.getObject(objectId);
          callback(obj);
        } catch {
          // Object may have been evaporated — stop watching
          break;
        }
        await new Promise((r) => setTimeout(r, interval));
      }
    };

    poll();

    return () => {
      running = false;
    };
  }

  /**
   * Calculate energy decay estimates for a state object.
   * This is unique to EvaporChain — no other blockchain has this concept.
   *
   * Uses the same exponential decay formula as the chain:
   * energy(t) = initial_energy * 2^(-t / half_life)
   *
   * @param objectId - 64-char hex object ID
   */
  async getEnergyDecayEstimate(objectId: string): Promise<DecayEstimate> {
    const [obj, status] = await Promise.all([
      this.getObject(objectId),
      this.getStatus(),
    ]);

    const currentEpoch = status.epoch;
    const epochsElapsed = currentEpoch - obj.last_refreshed;

    // Estimate epochs until energy reaches ~1 (effectively zero)
    // energy(t) = max * 2^(-t/hl) => t = hl * log2(max)
    const epochsRemaining = obj.half_life > 0
      ? Math.max(0, Math.ceil(obj.half_life * Math.log2(Math.max(1, obj.current_energy))) - epochsElapsed)
      : 0;

    // Grace entry: when energy first hits 0
    // max * 2^(-t/hl) < 1 => t > hl * log2(max)
    const epochsToZero = obj.half_life > 0
      ? Math.ceil(obj.half_life * Math.log2(Math.max(1, obj.max_energy)))
      : 0;

    const willEnterGraceAt = obj.last_refreshed + epochsToZero;
    // Evaporation happens after grace period (typically a few epochs after)
    const willEvaporateAt = willEnterGraceAt + 3;

    return {
      current_energy: obj.current_energy,
      max_energy: obj.max_energy,
      half_life: obj.half_life,
      epochs_elapsed: epochsElapsed,
      decay_percentage: obj.decay_percentage,
      estimated_epochs_remaining: epochsRemaining,
      will_enter_grace_at: willEnterGraceAt,
      will_evaporate_at: willEvaporateAt,
    };
  }

  /**
   * Watch for new chain events in real-time.
   * Returns a stop function.
   */
  watchEvents(
    callback: (events: EventRecord[]) => void,
    interval: number = 3000
  ): () => void {
    let running = true;
    let lastTimestamp = 0;

    const poll = async () => {
      while (running) {
        try {
          const events = await this.getEvents(50);
          const newEvents = events.filter((e) => e.timestamp_ms > lastTimestamp);
          if (newEvents.length > 0) {
            lastTimestamp = Math.max(...newEvents.map((e) => e.timestamp_ms));
            callback(newEvents);
          }
        } catch {
          // Ignore transient errors
        }
        await new Promise((r) => setTimeout(r, interval));
      }
    };

    poll();
    return () => { running = false; };
  }

  // ── WebSocket Real-Time Subscriptions ──

  private get wsUrl(): string {
    const url = this.baseUrl.replace(/^http/, "ws");
    const topics = this.wsTopics.join(",");
    return `${url}/ws?subscribe=${topics}`;
  }

  /**
   * Connect to the WebSocket for real-time event streaming.
   * Listen for events with `.on()`.
   *
   * ```ts
   * const chain = new EvaporChain("http://localhost:9944");
   * chain.subscribe(["blocks", "evaporations"]);
   *
   * chain.on("new_block", (block) => {
   *   console.log(`Block #${block.number} with ${block.tx_count} txs`);
   * });
   *
   * chain.on("evaporation", (ev) => {
   *   console.log(`Object ${ev.object_id} evaporated`);
   * });
   *
   * // Later:
   * chain.unsubscribe();
   * ```
   *
   * @param topics - Topics to subscribe to (default: ["all"])
   */
  subscribe(topics: WsTopic[] = ["all"]): void {
    this.wsTopics = topics;
    this.wsShouldReconnect = true;
    this.wsReconnectCount = 0;
    this.connectWs();
  }

  /** Disconnect the WebSocket and stop reconnecting. */
  unsubscribe(): void {
    this.wsShouldReconnect = false;
    if (this.wsReconnectTimer) {
      clearTimeout(this.wsReconnectTimer);
      this.wsReconnectTimer = null;
    }
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
  }

  /** Whether the WebSocket is currently connected. */
  get connected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  /**
   * Register an event handler for WebSocket events.
   *
   * Event names match the `type` field: `new_block`, `new_transaction`,
   * `evaporation`, `grace_period`, `chain_event`, `peer_update`,
   * `connected`, `warning`.
   *
   * Special events: `ws_error`, `ws_close`, `ws_reconnect`.
   */
  on(event: string, listener: (...args: unknown[]) => void): this {
    this.emitter.on(event, listener);
    return this;
  }

  /** Remove a specific event handler. */
  off(event: string, listener: (...args: unknown[]) => void): this {
    this.emitter.off(event, listener);
    return this;
  }

  /** Register a one-time event handler. */
  once(event: string, listener: (...args: unknown[]) => void): this {
    this.emitter.once(event, listener);
    return this;
  }

  private connectWs(): void {
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }

    const ws = new WebSocket(this.wsUrl);

    ws.onopen = () => {
      this.wsReconnectCount = 0;
    };

    ws.onmessage = (event) => {
      try {
        const data = JSON.parse(typeof event.data === "string" ? event.data : "") as WsEvent;
        this.emitter.emit(data.type, data);
      } catch {
        // Ignore malformed messages
      }
    };

    ws.onerror = (event) => {
      this.emitter.emit("ws_error", event);
    };

    ws.onclose = () => {
      this.ws = null;
      this.emitter.emit("ws_close");
      this.scheduleReconnect();
    };

    this.ws = ws;
  }

  private scheduleReconnect(): void {
    if (!this.wsShouldReconnect) return;
    if (this.wsMaxReconnects > 0 && this.wsReconnectCount >= this.wsMaxReconnects) {
      this.emitter.emit("ws_error", new Error(`Max reconnect attempts (${this.wsMaxReconnects}) reached`));
      return;
    }

    this.wsReconnectCount++;
    const delay = this.wsReconnectDelay * Math.min(this.wsReconnectCount, 5);
    this.emitter.emit("ws_reconnect", { attempt: this.wsReconnectCount, delay });

    this.wsReconnectTimer = setTimeout(() => {
      this.wsReconnectTimer = null;
      this.connectWs();
    }, delay);
  }
}

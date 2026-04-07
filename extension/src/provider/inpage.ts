/**
 * EvaporChain in-page provider — injected into every web page.
 *
 * Exposes `window.evaporchain` for dApps to interact with the wallet.
 *
 * Usage by dApps:
 *
 *   const accounts = await window.evaporchain.connect();
 *   const result = await window.evaporchain.sendTransaction({
 *     to: "0xabc...",
 *     amount: 1000,
 *   });
 */

interface EvaporChainProvider {
  isEvaporChain: true;
  isConnected: boolean;

  /** Request wallet connection. Returns array of addresses. */
  connect(): Promise<string[]>;

  /** Disconnect from the dApp. */
  disconnect(): Promise<void>;

  /** Get connected accounts. */
  getAccounts(): Promise<string[]>;

  /** Get chain status. */
  getChainStatus(): Promise<unknown>;

  /** Request a transaction to be signed and submitted. Opens approval popup. */
  sendTransaction(params: {
    to: string;
    amount: number;
  }): Promise<{ success: boolean; message: string }>;

  /** Sign a message (returns hex signature). */
  signMessage(message: string): Promise<string>;

  /** Listen for events. */
  on(event: string, callback: (...args: unknown[]) => void): void;
  off(event: string, callback: (...args: unknown[]) => void): void;
}

class EvaporChainProviderImpl implements EvaporChainProvider {
  isEvaporChain = true as const;
  isConnected = false;

  private listeners = new Map<string, Set<(...args: unknown[]) => void>>();
  private pendingRequests = new Map<string, {
    resolve: (value: unknown) => void;
    reject: (reason: unknown) => void;
  }>();

  constructor() {
    // Listen for responses from content script
    window.addEventListener("message", (event) => {
      if (event.source !== window) return;

      if (event.data?.type === "EVAPORCHAIN_CONNECT_RESPONSE") {
        this.isConnected = event.data.connected ?? false;
        this.emit("connect", event.data);
      }

      if (event.data?.type === "EVAPORCHAIN_REQUEST_RESPONSE") {
        const pending = this.pendingRequests.get(event.data.requestId);
        if (pending) {
          if (event.data.error) {
            pending.reject(new Error(event.data.error));
          } else {
            pending.resolve(event.data.result);
          }
          this.pendingRequests.delete(event.data.requestId);
        }
      }
    });
  }

  async connect(): Promise<string[]> {
    return new Promise((resolve, reject) => {
      window.postMessage({ type: "EVAPORCHAIN_CONNECT" }, "*");

      const handler = (event: MessageEvent) => {
        if (event.data?.type === "EVAPORCHAIN_CONNECT_RESPONSE") {
          window.removeEventListener("message", handler);
          if (event.data.connected) {
            this.isConnected = true;
            resolve(event.data.accounts ?? []);
          } else {
            reject(new Error("Connection rejected"));
          }
        }
      };
      window.addEventListener("message", handler);

      // Timeout after 30s
      setTimeout(() => {
        window.removeEventListener("message", handler);
        reject(new Error("Connection timed out"));
      }, 30_000);
    });
  }

  async disconnect(): Promise<void> {
    this.isConnected = false;
    this.emit("disconnect");
  }

  async getAccounts(): Promise<string[]> {
    if (!this.isConnected) return [];
    return this.request("getAccounts", {});
  }

  async getChainStatus(): Promise<unknown> {
    return this.request("getChainStatus", {});
  }

  async sendTransaction(params: { to: string; amount: number }): Promise<{ success: boolean; message: string }> {
    return this.request("sendTransaction", params);
  }

  async signMessage(message: string): Promise<string> {
    return this.request("signMessage", { message });
  }

  on(event: string, callback: (...args: unknown[]) => void): void {
    if (!this.listeners.has(event)) this.listeners.set(event, new Set());
    this.listeners.get(event)!.add(callback);
  }

  off(event: string, callback: (...args: unknown[]) => void): void {
    this.listeners.get(event)?.delete(callback);
  }

  private emit(event: string, ...args: unknown[]) {
    this.listeners.get(event)?.forEach(cb => cb(...args));
  }

  private request<T>(method: string, params: unknown): Promise<T> {
    return new Promise((resolve, reject) => {
      const requestId = Math.random().toString(36).slice(2);
      this.pendingRequests.set(requestId, {
        resolve: resolve as (value: unknown) => void,
        reject,
      });
      window.postMessage({
        type: "EVAPORCHAIN_REQUEST",
        method,
        params,
        requestId,
      }, "*");

      // Timeout after 60s
      setTimeout(() => {
        if (this.pendingRequests.has(requestId)) {
          this.pendingRequests.delete(requestId);
          reject(new Error("Request timed out"));
        }
      }, 60_000);
    });
  }
}

// Inject provider into page
const provider = new EvaporChainProviderImpl();
(window as any).evaporchain = provider;

// Announce provider availability
window.dispatchEvent(new CustomEvent("evaporchain#initialized"));

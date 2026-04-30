/**
 * WalletConnect v2 client wrapper for EvaporChain browser extension.
 *
 * Wraps `@walletconnect/web3wallet` (the wallet-side WC v2 SDK) and exposes
 * a small façade tailored to the popup UI (`WalletConnectScreen`,
 * `WcApprovalModal`).
 *
 * Storage: WC v2's `Core` writes its own state to `localStorage` by default —
 * fine inside the MV3 popup process. The service worker does NOT need any WC
 * code; all WC traffic runs in the popup.
 *
 * Required env var:
 *   `VITE_WALLETCONNECT_PROJECT_ID` — your project ID from
 *   https://cloud.reown.com (formerly cloud.walletconnect.com). If unset, the
 *   manager falls back to a placeholder string and emits a console warning;
 *   pairing will fail at the relay handshake until a real ID is provided.
 *
 * CAIP-2 chain IDs (see EIP-155-style `<namespace>:<reference>`):
 *   `evap:1`  — testnet (default during pre-mainnet)
 *   `evap:0`  — mainnet (reserved)
 *
 * The `evap` namespace is local to EvaporChain (not yet registered with the
 * CAIP registry). dApps must advertise it explicitly in `requiredNamespaces`.
 */

import { Core } from "@walletconnect/core";
import { Web3Wallet, type IWeb3Wallet, type Web3WalletTypes } from "@walletconnect/web3wallet";
import type { ProposalTypes, SessionTypes } from "@walletconnect/types";
import { getSdkError } from "@walletconnect/utils";

// ── Types (kept stable for UI consumers) ──

export interface WcPeerMeta {
  name: string;
  description: string;
  url: string;
  icons: string[];
}

export interface WcNamespace {
  chains: string[];
  methods: string[];
  events: string[];
}

/**
 * Session proposal shape — a thin re-export of the SDK's
 * `Web3WalletTypes.SessionProposal` payload, kept in this module so
 * `WcApprovalModal.tsx` and `WalletConnectScreen.tsx` don't need to import
 * SDK types directly.
 */
export type WcSessionProposal = Web3WalletTypes.SessionProposal;

export interface WcSession {
  topic: string;
  peer: WcPeerMeta;
  namespaces: Record<string, WcNamespace>;
  expiry: number;
  acknowledged: boolean;
  connectedAt: number;
}

export interface WcRequest {
  id: number;
  topic: string;
  params: {
    request: {
      method: string;
      params: unknown;
    };
    chainId: string;
  };
}

export type WcEventHandler<T = unknown> = (payload: T) => void;

/** Handler the manager calls when a dApp invokes an EvaporChain RPC method. */
export interface WcRequestHandler {
  /** Returns the user's currently active EvaporChain accounts (full CAIP IDs preferred, plain addresses also accepted). */
  getAccounts: () => string[];
  /** Sign a transaction payload — should call into the keystore (real ML-DSA-65). */
  signTransaction: (payload: string) => Promise<{ signature: string; publicKey: string }>;
  /** Sign an arbitrary message — defaults to `signTransaction` if not supplied. */
  signMessage?: (message: string) => Promise<{ signature: string; publicKey: string }>;
  /** Broadcast a signed transaction; returns a chain hash. */
  sendTransaction?: (tx: unknown) => Promise<{ hash: string }>;
}

// ── Error codes ──

export enum WcErrorCode {
  USER_REJECTED = 5000,
  UNSUPPORTED_METHOD = 5001,
  INVALID_URI = 5002,
  SESSION_NOT_FOUND = 5003,
  NOT_INITIALIZED = 5004,
  WALLET_LOCKED = 5005,
  SDK_ERROR = 5006,
}

export class WalletConnectError extends Error {
  public code: WcErrorCode;
  constructor(message: string, code: WcErrorCode) {
    super(message);
    this.name = "WalletConnectError";
    this.code = code;
  }
}

// ── Constants ──

/**
 * EvaporChain testnet CAIP-2 chain ID. Picked `evap:1` so testnet is `1` and
 * mainnet (reserved) is `0`, matching how a number of L1s expose their first
 * test network. The `evap` namespace prefix is the project's own — short,
 * lowercase, ASCII, fits CAIP-2 grammar (`[-a-z0-9]{3,8}`).
 */
export const EVAP_CHAIN_TESTNET = "evap:1";
export const EVAP_CHAIN_MAINNET = "evap:0";

/** Methods the wallet advertises support for over WalletConnect. */
export const EVAP_WC_METHODS = [
  "evap_signTransaction",
  "evap_signMessage",
  "evap_sendTransaction",
  "evap_getAccounts",
] as const;

/** Events the wallet emits over WalletConnect. */
export const EVAP_WC_EVENTS = ["accountsChanged", "chainChanged"] as const;

const WC_PROJECT_ID_ENV = "VITE_WALLETCONNECT_PROJECT_ID";
const WC_PROJECT_ID_PLACEHOLDER = "REPLACE_ME_WALLETCONNECT_PROJECT_ID";

/** Resolve the WC Cloud project ID from Vite env, with a placeholder fallback. */
export function resolveWcProjectId(): string {
  const fromEnv = (import.meta.env?.[WC_PROJECT_ID_ENV] as string | undefined)?.trim();
  if (fromEnv) return fromEnv;
  // eslint-disable-next-line no-console
  console.warn(
    `[walletconnect] ${WC_PROJECT_ID_ENV} is not set — using placeholder. ` +
      `Pairing will fail until you set a real project ID from cloud.reown.com.`,
  );
  return WC_PROJECT_ID_PLACEHOLDER;
}

// ── Helpers ──

function mapSdkError(e: unknown, fallbackCode: WcErrorCode = WcErrorCode.SDK_ERROR): WalletConnectError {
  if (e instanceof WalletConnectError) return e;
  const msg = e instanceof Error ? e.message : String(e);
  return new WalletConnectError(msg, fallbackCode);
}

function structToWcSession(struct: SessionTypes.Struct): WcSession {
  const peerMeta = struct.peer.metadata;
  return {
    topic: struct.topic,
    peer: {
      name: peerMeta.name,
      description: peerMeta.description,
      url: peerMeta.url,
      icons: peerMeta.icons ?? [],
    },
    namespaces: Object.fromEntries(
      Object.entries(struct.namespaces).map(([key, ns]) => [
        key,
        {
          chains: ns.chains ?? [],
          methods: ns.methods,
          events: ns.events,
        } satisfies WcNamespace,
      ]),
    ),
    // SessionTypes.Expiry is a unix-seconds timestamp — convert to ms for the UI.
    expiry: struct.expiry * 1000,
    acknowledged: struct.acknowledged,
    // WC sessions don't expose a "connectedAt" — derive a best-effort value
    // from `expiry - 7d` (the default session lifetime).
    connectedAt: Math.max(0, struct.expiry * 1000 - 7 * 24 * 60 * 60 * 1000),
  };
}

// ── Manager ──

/**
 * WalletConnectManager — wallet-side WC v2 façade for the EvaporChain extension.
 *
 * Lifecycle:
 *   const wc = new WalletConnectManager();
 *   await wc.init();                              // lazy SDK init
 *   wc.onProposal = (p) => showApprovalCard(p);   // wire UI
 *   wc.onRequest  = (r) => routeToHandler(r);
 *   await wc.pair("wc:abc123...");                // start a pairing
 *   await wc.approveProposal(proposal, [address]);
 *
 * All async methods throw `WalletConnectError` on failure.
 */
export class WalletConnectManager {
  private initialized = false;
  private initPromise: Promise<void> | null = null;
  private client: IWeb3Wallet | null = null;

  // Event handlers exposed to the UI.
  private _onProposal: WcEventHandler<WcSessionProposal> | null = null;
  private _onRequest: WcEventHandler<WcRequest> | null = null;
  private _onDisconnect: WcEventHandler<{ topic: string }> | null = null;

  /** Optional handler set the manager calls when a dApp invokes an RPC method. */
  private requestHandler: WcRequestHandler | null = null;

  // ── Lifecycle ──

  /**
   * Initialize the WC SDK with a real `Core` instance. Idempotent and lazy:
   * concurrent callers share a single init promise. The project ID is read
   * from `import.meta.env.VITE_WALLETCONNECT_PROJECT_ID`; pass `projectId`
   * explicitly to override (used in tests).
   */
  async init(projectId?: string): Promise<void> {
    if (this.initialized) return;
    if (this.initPromise) return this.initPromise;

    const id = projectId ?? resolveWcProjectId();

    this.initPromise = (async () => {
      try {
        // Cast the Core instance — `@walletconnect/web3wallet` re-bundles
        // its own copy of `@walletconnect/types`, which makes the structural
        // `ICore` interface technically incompatible across the package
        // boundary even though the runtime classes are identical. The cast
        // is safe and matches what every WC v2 wallet integration does.
        const core = new Core({ projectId: id });
        this.client = await Web3Wallet.init({
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          core: core as any,
          metadata: {
            name: "EvaporChain Wallet",
            description: "Post-quantum wallet for the blockchain that evaporates",
            url: "https://evaporchain.com",
            icons: ["https://evaporchain.com/icon.png"],
          },
        });

        // Wire SDK events through to the UI handlers.
        this.client.on("session_proposal", (proposal) => {
          this._onProposal?.(proposal);
        });
        this.client.on("session_request", (request) => {
          this._onRequest?.(request as unknown as WcRequest);
          if (this.requestHandler) {
            void this._dispatchRequest(request as unknown as WcRequest);
          }
        });
        this.client.on("session_delete", ({ topic }) => {
          this._onDisconnect?.({ topic });
        });

        this.initialized = true;
      } catch (e) {
        this.initPromise = null;
        throw mapSdkError(e, WcErrorCode.NOT_INITIALIZED);
      }
    })();

    return this.initPromise;
  }

  // ── Pairing ──

  /**
   * Pair with a dApp using a WalletConnect URI (`wc:...`). The SDK will
   * subsequently emit `session_proposal`, which the manager forwards to
   * `onProposal`.
   */
  async pair(uri: string): Promise<void> {
    this._ensureInitialized();
    const trimmed = uri.trim();
    if (!trimmed.startsWith("wc:")) {
      throw new WalletConnectError(
        "Invalid WalletConnect URI — must start with wc:",
        WcErrorCode.INVALID_URI,
      );
    }
    try {
      await this.client!.pair({ uri: trimmed });
    } catch (e) {
      throw mapSdkError(e, WcErrorCode.INVALID_URI);
    }
  }

  // ── Sessions ──

  /** List all currently active WC sessions. */
  getActiveSessions(): WcSession[] {
    if (!this.client) return [];
    const active = this.client.getActiveSessions();
    return Object.values(active).map(structToWcSession);
  }

  /** Backwards-compatible alias for {@link getActiveSessions}. */
  getSessions(): WcSession[] {
    return this.getActiveSessions();
  }

  /**
   * Approve a session proposal for a list of EvaporChain accounts. Accounts
   * are converted to CAIP-10 form (`<namespace>:<reference>:<address>`)
   * automatically.
   */
  async approveProposal(proposal: WcSessionProposal, accounts: string[]): Promise<WcSession> {
    this._ensureInitialized();
    if (accounts.length === 0) {
      throw new WalletConnectError("No accounts to approve", WcErrorCode.USER_REJECTED);
    }

    const required = proposal.params.requiredNamespaces ?? {};
    const optional = proposal.params.optionalNamespaces ?? {};

    // Build namespaces manually — `buildApprovedNamespaces` is strict and
    // rejects any required-namespace key the wallet doesn't explicitly
    // support, which is awkward when dApps under-specify chains.
    const namespaces: Record<string, SessionTypes.Namespace> = {};

    const merged: Record<string, ProposalTypes.RequiredNamespace> = {
      ...optional,
      ...required, // required wins on conflict
    };

    for (const [key, ns] of Object.entries(merged)) {
      const chains = ns.chains && ns.chains.length > 0 ? ns.chains : [EVAP_CHAIN_TESTNET];
      const caipAccounts = chains.flatMap((chain) =>
        accounts.map((addr) => (addr.includes(":") ? addr : `${chain}:${addr}`)),
      );
      namespaces[key] = {
        chains,
        accounts: caipAccounts,
        // Advertise both whatever the dApp asked for AND our supported set,
        // de-duplicated. WC requires every required method/event to appear.
        methods: Array.from(new Set([...ns.methods, ...EVAP_WC_METHODS])),
        events: Array.from(new Set([...ns.events, ...EVAP_WC_EVENTS])),
      };
    }

    try {
      const struct = await this.client!.approveSession({
        id: proposal.id,
        namespaces,
      });
      return structToWcSession(struct);
    } catch (e) {
      throw mapSdkError(e);
    }
  }

  /** Reject a pending session proposal with USER_REJECTED. */
  async rejectProposal(proposal: WcSessionProposal): Promise<void> {
    this._ensureInitialized();
    try {
      await this.client!.rejectSession({
        id: proposal.id,
        reason: getSdkError("USER_REJECTED"),
      });
    } catch (e) {
      throw mapSdkError(e);
    }
  }

  /** Disconnect a single session by topic. */
  async disconnect(topic: string): Promise<void> {
    this._ensureInitialized();
    const active = this.client!.getActiveSessions();
    if (!active[topic]) {
      throw new WalletConnectError(
        `Session not found: ${topic}`,
        WcErrorCode.SESSION_NOT_FOUND,
      );
    }
    try {
      await this.client!.disconnectSession({
        topic,
        reason: getSdkError("USER_DISCONNECTED"),
      });
    } catch (e) {
      throw mapSdkError(e);
    }
    this._onDisconnect?.({ topic });
  }

  /** Disconnect all active sessions. */
  async disconnectAll(): Promise<void> {
    if (!this.client) return;
    const topics = Object.keys(this.client.getActiveSessions());
    for (const topic of topics) {
      try {
        await this.disconnect(topic);
      } catch {
        // Best-effort — keep going on individual failures.
      }
    }
  }

  // ── Backwards-compatible aliases ──

  /** Alias for {@link approveProposal}. */
  async approveSession(proposal: WcSessionProposal, accounts: string[]): Promise<WcSession> {
    return this.approveProposal(proposal, accounts);
  }

  /** Alias for {@link rejectProposal}. */
  async rejectSession(proposal: WcSessionProposal): Promise<void> {
    return this.rejectProposal(proposal);
  }

  /** Alias for {@link disconnect}. */
  async disconnectSession(topic: string): Promise<void> {
    return this.disconnect(topic);
  }

  // ── Request handling ──

  /**
   * Register a wallet-side handler for incoming `session_request` events.
   * If set, the manager will auto-dispatch RPC calls to the handler and
   * respond on the SDK's behalf. Set to `null` to disable.
   */
  setRequestHandler(handler: WcRequestHandler | null): void {
    this.requestHandler = handler;
  }

  private async _dispatchRequest(request: WcRequest): Promise<void> {
    const handler = this.requestHandler;
    if (!handler || !this.client) return;

    const { id, topic } = request;
    const { method, params } = request.params.request;

    try {
      let result: unknown;
      switch (method) {
        case "evap_getAccounts":
          result = handler.getAccounts();
          break;

        case "evap_signTransaction": {
          const payload = typeof params === "string" ? params : JSON.stringify(params);
          result = await handler.signTransaction(payload);
          break;
        }

        case "evap_signMessage": {
          const message =
            Array.isArray(params) && typeof params[0] === "string"
              ? (params[0] as string)
              : typeof params === "string"
                ? params
                : JSON.stringify(params);
          if (handler.signMessage) {
            result = await handler.signMessage(message);
          } else {
            // Fall back to signing the message bytes as a transaction payload.
            result = await handler.signTransaction(message);
          }
          break;
        }

        case "evap_sendTransaction": {
          if (!handler.sendTransaction) {
            throw new WalletConnectError(
              "evap_sendTransaction not supported by this wallet build",
              WcErrorCode.UNSUPPORTED_METHOD,
            );
          }
          const txParam = Array.isArray(params) ? params[0] : params;
          result = await handler.sendTransaction(txParam);
          break;
        }

        default:
          throw new WalletConnectError(
            `Unsupported method: ${method}`,
            WcErrorCode.UNSUPPORTED_METHOD,
          );
      }

      await this.client.respondSessionRequest({
        topic,
        response: { id, jsonrpc: "2.0", result },
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      // Surface "Wallet locked" cleanly — the keystore throws this verbatim.
      const sdkErr =
        msg.toLowerCase().includes("wallet locked") || msg.toLowerCase().includes("locked")
          ? { code: WcErrorCode.WALLET_LOCKED, message: msg }
          : getSdkError("USER_REJECTED");
      try {
        await this.client.respondSessionRequest({
          topic,
          response: { id, jsonrpc: "2.0", error: sdkErr },
        });
      } catch {
        // Last-resort: swallow — the relay may already have torn down the topic.
      }
    }
  }

  // ── Event handler setters ──

  set onProposal(handler: WcEventHandler<WcSessionProposal> | null) {
    this._onProposal = handler;
  }

  set onRequest(handler: WcEventHandler<WcRequest> | null) {
    this._onRequest = handler;
  }

  set onDisconnect(handler: WcEventHandler<{ topic: string }> | null) {
    this._onDisconnect = handler;
  }

  // ── Internals ──

  private _ensureInitialized(): void {
    if (!this.initialized || !this.client) {
      throw new WalletConnectError(
        "WalletConnectManager not initialized — call init() first",
        WcErrorCode.NOT_INITIALIZED,
      );
    }
  }
}

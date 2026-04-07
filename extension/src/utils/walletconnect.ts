/**
 * WalletConnect v2 client wrapper for EvaporChain browser extension.
 *
 * Stub implementation — structured to wire up @walletconnect/web3wallet
 * once the npm dependency is installed. All public methods follow the
 * WalletConnect v2 Sign API patterns.
 */

// ── Types ──

export interface WcSessionProposal {
  id: number;
  params: {
    proposer: {
      metadata: WcPeerMeta;
    };
    requiredNamespaces: Record<string, WcNamespace>;
    optionalNamespaces?: Record<string, WcNamespace>;
  };
}

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
      params: unknown[];
    };
    chainId: string;
  };
}

export type WcEventHandler<T = unknown> = (payload: T) => void;

// ── Error codes ──

export enum WcErrorCode {
  USER_REJECTED = 5000,
  UNSUPPORTED_METHOD = 5001,
  INVALID_URI = 5002,
  SESSION_NOT_FOUND = 5003,
  NOT_INITIALIZED = 5004,
}

export class WalletConnectError extends Error {
  public code: WcErrorCode;
  constructor(message: string, code: WcErrorCode) {
    super(message);
    this.name = "WalletConnectError";
    this.code = code;
  }
}

// ── Manager ──

/**
 * WalletConnectManager — manages WC v2 sessions for the EvaporChain wallet.
 *
 * Usage:
 * ```ts
 * const wc = new WalletConnectManager();
 * await wc.init("your-wc-project-id");
 * await wc.pair("wc:abc123...");
 * ```
 *
 * NOTE: This is a stub implementation. Replace the internal logic with
 * @walletconnect/web3wallet once npm installed:
 *   import { Web3Wallet } from "@walletconnect/web3wallet";
 */
export class WalletConnectManager {
  private initialized = false;
  private projectId: string | null = null;
  private sessions: Map<string, WcSession> = new Map();

  // Event handlers
  private _onProposal: WcEventHandler<WcSessionProposal> | null = null;
  private _onRequest: WcEventHandler<WcRequest> | null = null;
  private _onDisconnect: WcEventHandler<{ topic: string }> | null = null;

  /** EvaporChain namespace identifier */
  static readonly CHAIN_ID = "evaporchain:testnet";
  static readonly NAMESPACE = "evaporchain";

  // ── Lifecycle ──

  /**
   * Initialize the WalletConnect client with a WC Cloud project ID.
   * In production, this creates a Web3Wallet instance with Core.
   */
  async init(projectId: string): Promise<void> {
    if (this.initialized) return;

    this.projectId = projectId;

    // STUB: In production, initialize like this:
    // const core = new Core({ projectId });
    // this.client = await Web3Wallet.init({
    //   core,
    //   metadata: {
    //     name: "EvaporChain Wallet",
    //     description: "Post-quantum wallet for the blockchain that evaporates",
    //     url: "https://evaporchain.com",
    //     icons: ["https://evaporchain.com/icon.png"],
    //   },
    // });
    // this.client.on("session_proposal", (p) => this._onProposal?.(p));
    // this.client.on("session_request", (r) => this._onRequest?.(r));
    // this.client.on("session_delete", (d) => this._onDisconnect?.(d));

    // Restore persisted sessions
    this._restoreSessions();
    this.initialized = true;
  }

  // ── Pairing ──

  /**
   * Pair with a dApp using a WalletConnect URI (wcv2:... or wc:...).
   * This triggers the onProposal callback with the session proposal.
   */
  async pair(uri: string): Promise<void> {
    this._ensureInitialized();

    if (!uri.startsWith("wc:") && !uri.startsWith("wcv2:")) {
      throw new WalletConnectError(
        "Invalid WalletConnect URI — must start with wc: or wcv2:",
        WcErrorCode.INVALID_URI,
      );
    }

    // STUB: In production:
    // await this.client.core.pairing.pair({ uri });
    // The session_proposal event fires automatically after pairing.

    // Simulate a proposal for development/testing
    const stubProposal: WcSessionProposal = {
      id: Date.now(),
      params: {
        proposer: {
          metadata: {
            name: this._extractDappName(uri),
            description: "dApp connected via WalletConnect",
            url: "https://dapp.example.com",
            icons: [],
          },
        },
        requiredNamespaces: {
          [WalletConnectManager.NAMESPACE]: {
            chains: [WalletConnectManager.CHAIN_ID],
            methods: ["evaporchain_sendTransaction", "evaporchain_signMessage", "evaporchain_getAccounts"],
            events: ["accountsChanged", "chainChanged"],
          },
        },
      },
    };

    this._onProposal?.(stubProposal);
  }

  // ── Sessions ──

  /** List all active WalletConnect sessions. */
  getSessions(): WcSession[] {
    return Array.from(this.sessions.values());
  }

  /** Approve a session proposal and establish connection. */
  async approveSession(
    proposal: WcSessionProposal,
    accounts: string[],
  ): Promise<WcSession> {
    this._ensureInitialized();

    const topic = `topic_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
    const namespaces: Record<string, WcNamespace> = {};

    for (const [key, ns] of Object.entries(proposal.params.requiredNamespaces)) {
      namespaces[key] = {
        chains: ns.chains,
        methods: ns.methods,
        events: ns.events,
      };
    }

    // STUB: In production:
    // const session = await this.client.approveSession({
    //   id: proposal.id,
    //   namespaces: { evaporchain: { accounts: accounts.map(a => `${CHAIN_ID}:${a}`), ... } },
    // });

    const session: WcSession = {
      topic,
      peer: proposal.params.proposer.metadata,
      namespaces,
      expiry: Date.now() + 7 * 24 * 60 * 60 * 1000, // 7 days
      acknowledged: true,
      connectedAt: Date.now(),
    };

    this.sessions.set(topic, session);
    this._persistSessions();
    return session;
  }

  /** Reject a session proposal. */
  async rejectSession(proposal: WcSessionProposal): Promise<void> {
    this._ensureInitialized();

    // STUB: In production:
    // await this.client.rejectSession({
    //   id: proposal.id,
    //   reason: getSdkError("USER_REJECTED"),
    // });
    // No-op in stub — proposal is simply not stored.
  }

  /** Disconnect an active session by topic. */
  async disconnectSession(topic: string): Promise<void> {
    this._ensureInitialized();

    if (!this.sessions.has(topic)) {
      throw new WalletConnectError(
        `Session not found: ${topic}`,
        WcErrorCode.SESSION_NOT_FOUND,
      );
    }

    // STUB: In production:
    // await this.client.disconnectSession({
    //   topic,
    //   reason: getSdkError("USER_DISCONNECTED"),
    // });

    this.sessions.delete(topic);
    this._persistSessions();
    this._onDisconnect?.({ topic });
  }

  /** Disconnect all active sessions. */
  async disconnectAll(): Promise<void> {
    const topics = Array.from(this.sessions.keys());
    for (const topic of topics) {
      await this.disconnectSession(topic);
    }
  }

  // ── Request handling ──

  /**
   * Handle an incoming JSON-RPC request from a connected dApp.
   * Routes to the appropriate wallet method.
   */
  async handleRequest(
    request: WcRequest,
    handlers: {
      getAccounts: () => string[];
      signMessage: (message: string) => Promise<string>;
      sendTransaction: (tx: unknown) => Promise<{ hash: string }>;
    },
  ): Promise<unknown> {
    this._ensureInitialized();

    const { method, params } = request.params.request;

    switch (method) {
      case "evaporchain_getAccounts":
        return handlers.getAccounts();

      case "evaporchain_signMessage":
        return handlers.signMessage(params[0] as string);

      case "evaporchain_sendTransaction":
        return handlers.sendTransaction(params[0]);

      default:
        throw new WalletConnectError(
          `Unsupported method: ${method}`,
          WcErrorCode.UNSUPPORTED_METHOD,
        );
    }
  }

  // ── Event handlers ──

  set onProposal(handler: WcEventHandler<WcSessionProposal> | null) {
    this._onProposal = handler;
  }

  set onRequest(handler: WcEventHandler<WcRequest> | null) {
    this._onRequest = handler;
  }

  set onDisconnect(handler: WcEventHandler<{ topic: string }> | null) {
    this._onDisconnect = handler;
  }

  // ── Internal helpers ──

  private _ensureInitialized(): void {
    if (!this.initialized) {
      throw new WalletConnectError(
        "WalletConnectManager not initialized — call init() first",
        WcErrorCode.NOT_INITIALIZED,
      );
    }
  }

  private _persistSessions(): void {
    try {
      const data = JSON.stringify(Array.from(this.sessions.entries()));
      localStorage.setItem("evaporchain_wc_sessions", data);
    } catch {
      // localStorage may not be available in service worker context
    }
  }

  private _restoreSessions(): void {
    try {
      const raw = localStorage.getItem("evaporchain_wc_sessions");
      if (raw) {
        const entries: [string, WcSession][] = JSON.parse(raw);
        const now = Date.now();
        for (const [topic, session] of entries) {
          if (session.expiry > now) {
            this.sessions.set(topic, session);
          }
        }
      }
    } catch {
      // Ignore parse errors — start fresh
    }
  }

  private _extractDappName(uri: string): string {
    // In production the dApp name comes from the proposal metadata.
    // For the stub, derive a placeholder from the URI.
    return `dApp-${uri.slice(3, 11)}`;
  }
}

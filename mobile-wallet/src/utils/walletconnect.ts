/**
 * WalletConnect v2 client wrapper for the EvaporChain mobile wallet.
 *
 * This is the React-Native sibling of `extension/src/utils/walletconnect.ts`
 * — same façade, same CAIP-2 chain IDs, same `evap_*` method set, same
 * approval / request flow. The only deltas are runtime-driven:
 *
 *   1. The project ID is read from `expo-constants`
 *      (`Constants.expoConfig.extra.walletconnectProjectId`) instead of a
 *      Vite env var. Set it in `app.json` under `expo.extra`.
 *   2. The WC v2 SDK depends on a handful of WHATWG primitives that
 *      Hermes / RN doesn't ship by default. The wallet's `App.tsx`
 *      imports the polyfills (`react-native-get-random-values`,
 *      `react-native-url-polyfill/auto`, `text-encoding-polyfill`) before
 *      this module is touched.
 *   3. `init()` resolves the SDK lazily via `require()` so the screen
 *      stays navigable even if the deps haven't been installed yet — the
 *      manager surfaces a clear "Install WC v2 deps" error which the
 *      UI renders inline. Once the deps are present at runtime, the same
 *      code path takes the real SDK and pairs against the production
 *      relay.
 *
 * CAIP-2 chain IDs (must match the extension byte-for-byte so dApp
 * developers only have to know one set):
 *   `evap:1` — testnet (default)
 *   `evap:0` — mainnet (reserved)
 *
 * The `evap` namespace prefix is local to EvaporChain. dApps must
 * advertise it explicitly in `requiredNamespaces`.
 */

import Constants from 'expo-constants';

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
 * Session proposal — kept structurally compatible with the SDK's
 * `Web3WalletTypes.SessionProposal`. We avoid importing the SDK type
 * directly so this file still type-checks before deps are installed.
 */
export interface WcSessionProposal {
  id: number;
  params: {
    id: number;
    pairingTopic: string;
    expiry: number;
    proposer: {
      publicKey: string;
      metadata: WcPeerMeta;
    };
    requiredNamespaces: Record<string, { chains?: string[]; methods: string[]; events: string[] }>;
    optionalNamespaces?: Record<string, { chains?: string[]; methods: string[]; events: string[] }>;
    relays: Array<{ protocol: string }>;
    [k: string]: unknown;
  };
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
      params: unknown;
    };
    chainId: string;
  };
}

export type WcEventHandler<T = unknown> = (payload: T) => void;

/**
 * The manager's request handler returns the JSON-RPC `result` field as
 * an opaque value; the manager forwards it back to the dApp via
 * `respondSessionRequest`. Throwing inside the handler causes the
 * manager to respond with a USER_REJECTED / WALLET_LOCKED error — the
 * exact code is selected from the thrown message.
 */
export type WcRequestHandler = (
  topic: string,
  request: WcRequest,
) => Promise<unknown>;

// ── Error codes ──

export enum WcErrorCode {
  USER_REJECTED = 5000,
  UNSUPPORTED_METHOD = 5001,
  INVALID_URI = 5002,
  SESSION_NOT_FOUND = 5003,
  NOT_INITIALIZED = 5004,
  WALLET_LOCKED = 5005,
  SDK_ERROR = 5006,
  SDK_MISSING = 5007,
}

export class WalletConnectError extends Error {
  public code: WcErrorCode;
  constructor(message: string, code: WcErrorCode) {
    super(message);
    this.name = 'WalletConnectError';
    this.code = code;
  }
}

// ── Constants ──

/**
 * EvaporChain CAIP-2 chain IDs. `evap:1` is testnet (default during
 * pre-mainnet); `evap:0` is reserved for mainnet. Identical to the
 * extension so dApps advertise one namespace across both wallets.
 */
export const EVAP_CHAIN_TESTNET = 'evap:1';
export const EVAP_CHAIN_MAINNET = 'evap:0';

/** Methods the wallet advertises support for over WalletConnect. */
export const EVAP_WC_METHODS = [
  'evap_signTransaction',
  'evap_signMessage',
  'evap_sendTransaction',
  'evap_getAccounts',
] as const;

/** Events the wallet emits over WalletConnect. */
export const EVAP_WC_EVENTS = ['accountsChanged', 'chainChanged'] as const;

const WC_PROJECT_ID_PLACEHOLDER = 'REPLACE_ME';

/**
 * Resolve the WC Cloud project ID from `app.json`'s `expo.extra` block.
 * Falls back to a placeholder if unset; pairing will fail at the relay
 * handshake until a real ID is provided.
 */
export function resolveWcProjectId(): string {
  // Constants.expoConfig is the EAS / classic-runtime resolved config.
  // Constants.manifest2 is the new manifest format used by EAS Update;
  // we read both to stay forward-compatible.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const cfg: any = Constants.expoConfig ?? (Constants as any).manifest ?? {};
  const fromExtra =
    typeof cfg.extra?.walletconnectProjectId === 'string'
      ? (cfg.extra.walletconnectProjectId as string).trim()
      : '';
  if (fromExtra && fromExtra !== WC_PROJECT_ID_PLACEHOLDER) return fromExtra;
  // eslint-disable-next-line no-console
  console.warn(
    '[walletconnect] expo.extra.walletconnectProjectId is not set in app.json — using placeholder. ' +
      'Pairing will fail until you set a real project ID from cloud.reown.com.',
  );
  return WC_PROJECT_ID_PLACEHOLDER;
}

// ── SDK lazy-load ──
//
// The WC SDK is a heavy dependency that may not be installed yet during
// initial dev iteration. Resolving it via `require()` lets the manager
// surface a clean error instead of crashing at module-load time.

interface ResolvedSdk {
  Core: new (opts: { projectId: string }) => unknown;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  Web3Wallet: { init: (opts: { core: any; metadata: WcPeerMeta }) => Promise<any> };
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  getSdkError: (key: string) => { code: number; message: string };
}

function loadSdk(): ResolvedSdk {
  try {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const core = require('@walletconnect/core');
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const w3w = require('@walletconnect/web3wallet');
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const utils = require('@walletconnect/utils');
    return {
      Core: core.Core,
      Web3Wallet: w3w.Web3Wallet,
      getSdkError: utils.getSdkError,
    };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    throw new WalletConnectError(
      `Install WC v2 deps to use WalletConnect — run \`npm install @walletconnect/web3wallet @walletconnect/core @walletconnect/utils @walletconnect/types\` in mobile-wallet/. (${msg})`,
      WcErrorCode.SDK_MISSING,
    );
  }
}

// ── Helpers ──

function mapSdkError(
  e: unknown,
  fallbackCode: WcErrorCode = WcErrorCode.SDK_ERROR,
): WalletConnectError {
  if (e instanceof WalletConnectError) return e;
  const msg = e instanceof Error ? e.message : String(e);
  return new WalletConnectError(msg, fallbackCode);
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function structToWcSession(struct: any): WcSession {
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
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      Object.entries(struct.namespaces).map(([key, ns]: [string, any]) => [
        key,
        {
          chains: ns.chains ?? [],
          methods: ns.methods,
          events: ns.events,
        } satisfies WcNamespace,
      ]),
    ),
    // SDK expiry is unix-seconds — convert to ms for the UI.
    expiry: struct.expiry * 1000,
    acknowledged: struct.acknowledged,
    // WC sessions don't expose a "connectedAt"; derive a best-effort
    // value from `expiry - 7d` (default session lifetime).
    connectedAt: Math.max(0, struct.expiry * 1000 - 7 * 24 * 60 * 60 * 1000),
  };
}

// ── Manager ──

/**
 * WalletConnectManager — wallet-side WC v2 façade for the EvaporChain
 * mobile wallet. API parity with the extension manager.
 *
 *   const wc = new WalletConnectManager();
 *   await wc.init();
 *   wc.onProposal = (p) => showApprovalSheet(p);
 *   wc.setRequestHandler(async (topic, req) => { ... });
 *   await wc.pair({ uri: 'wc:abc123...' });
 *   await wc.approveProposal(proposal, [address]);
 *
 * All async methods throw `WalletConnectError` on failure.
 */
export class WalletConnectManager {
  private initialized = false;
  private initPromise: Promise<void> | null = null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private client: any = null;

  // Event handlers exposed to the UI.
  private _onProposal: WcEventHandler<WcSessionProposal> | null = null;
  private _onRequest: WcEventHandler<WcRequest> | null = null;
  private _onDisconnect: WcEventHandler<{ topic: string }> | null = null;

  /** Optional handler the manager calls when a dApp invokes an RPC method. */
  private requestHandler: WcRequestHandler | null = null;

  /** SDK error helper, captured at init time so dispatch-error paths can use it. */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private sdkErr: ((key: string) => { code: number; message: string }) | null = null;

  // ── Lifecycle ──

  /**
   * Initialize the WC SDK with a real `Core` instance. Idempotent and
   * lazy: concurrent callers share a single init promise. The project
   * ID is read from `expo.extra.walletconnectProjectId`; pass
   * `projectId` to override (used in tests).
   */
  async init(projectId?: string): Promise<void> {
    if (this.initialized) return;
    if (this.initPromise) return this.initPromise;

    const id = projectId ?? resolveWcProjectId();

    this.initPromise = (async () => {
      try {
        const sdk = loadSdk();
        this.sdkErr = sdk.getSdkError;
        const core = new sdk.Core({ projectId: id });
        this.client = await sdk.Web3Wallet.init({
          core,
          metadata: {
            name: 'EvaporChain Wallet',
            description: 'Post-quantum mobile wallet for the blockchain that evaporates',
            url: 'https://evaporchain.com',
            icons: ['https://evaporchain.com/icon.png'],
          },
        });

        // Wire SDK events through to the UI handlers.
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        this.client.on('session_proposal', (proposal: any) => {
          this._onProposal?.(proposal as WcSessionProposal);
        });
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        this.client.on('session_request', (request: any) => {
          const wcReq = request as WcRequest;
          this._onRequest?.(wcReq);
          if (this.requestHandler) {
            void this._dispatchRequest(wcReq);
          }
        });
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        this.client.on('session_delete', ({ topic }: any) => {
          this._onDisconnect?.({ topic });
        });

        this.initialized = true;
      } catch (e) {
        this.initPromise = null;
        if (e instanceof WalletConnectError) throw e;
        throw mapSdkError(e, WcErrorCode.NOT_INITIALIZED);
      }
    })();

    return this.initPromise;
  }

  // ── Pairing ──

  /**
   * Pair with a dApp using a WalletConnect URI (`wc:...`). The SDK
   * subsequently emits `session_proposal`, which the manager forwards
   * to `onProposal`.
   */
  async pair({ uri }: { uri: string }): Promise<void> {
    this._ensureInitialized();
    const trimmed = uri.trim();
    if (!trimmed.startsWith('wc:')) {
      throw new WalletConnectError(
        'Invalid WalletConnect URI — must start with wc:',
        WcErrorCode.INVALID_URI,
      );
    }
    try {
      await this.client.pair({ uri: trimmed });
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

  /**
   * Approve a session proposal for a list of EvaporChain accounts.
   * Accounts are converted to CAIP-10 form (`<namespace>:<reference>:<address>`)
   * automatically.
   */
  async approveProposal(proposal: WcSessionProposal, accounts: string[]): Promise<WcSession> {
    this._ensureInitialized();
    if (accounts.length === 0) {
      throw new WalletConnectError('No accounts to approve', WcErrorCode.USER_REJECTED);
    }

    const required = proposal.params.requiredNamespaces ?? {};
    const optional = proposal.params.optionalNamespaces ?? {};

    // Build namespaces manually — `buildApprovedNamespaces` is strict and
    // rejects any required-namespace key the wallet doesn't explicitly
    // support, which is awkward when dApps under-specify chains.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const namespaces: Record<string, any> = {};

    const merged: Record<string, { chains?: string[]; methods: string[]; events: string[] }> = {
      ...optional,
      ...required, // required wins on conflict
    };

    for (const [key, ns] of Object.entries(merged)) {
      const chains = ns.chains && ns.chains.length > 0 ? ns.chains : [EVAP_CHAIN_TESTNET];
      const caipAccounts = chains.flatMap((chain) =>
        accounts.map((addr) => (addr.includes(':') ? addr : `${chain}:${addr}`)),
      );
      namespaces[key] = {
        chains,
        accounts: caipAccounts,
        // Advertise both whatever the dApp asked for AND our supported
        // set, de-duplicated. WC requires every required method/event
        // to appear.
        methods: Array.from(new Set([...ns.methods, ...EVAP_WC_METHODS])),
        events: Array.from(new Set([...ns.events, ...EVAP_WC_EVENTS])),
      };
    }

    try {
      const struct = await this.client.approveSession({
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
      await this.client.rejectSession({
        id: proposal.id,
        reason: this.sdkErr?.('USER_REJECTED') ?? {
          code: WcErrorCode.USER_REJECTED,
          message: 'User rejected.',
        },
      });
    } catch (e) {
      throw mapSdkError(e);
    }
  }

  /** Disconnect a single session by topic. */
  async disconnect(topic: string): Promise<void> {
    this._ensureInitialized();
    const active = this.client.getActiveSessions();
    if (!active[topic]) {
      throw new WalletConnectError(
        `Session not found: ${topic}`,
        WcErrorCode.SESSION_NOT_FOUND,
      );
    }
    try {
      await this.client.disconnectSession({
        topic,
        reason: this.sdkErr?.('USER_DISCONNECTED') ?? {
          code: 6000,
          message: 'User disconnected.',
        },
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

  // ── Request handling ──

  /**
   * Register a wallet-side handler for incoming `session_request`
   * events. If set, the manager auto-dispatches RPC calls to the
   * handler and responds on the SDK's behalf. Set to `null` to
   * disable.
   *
   * The handler receives `(topic, request)` and should return the
   * JSON-RPC `result` value (anything JSON-serializable). Throwing
   * causes the manager to respond with a USER_REJECTED error; throwing
   * a message containing "locked" maps to WALLET_LOCKED.
   */
  setRequestHandler(handler: WcRequestHandler | null): void {
    this.requestHandler = handler;
  }

  /**
   * Manually respond to a session request. The screen calls this after
   * collecting user approval and signing via the keystore.
   */
  async respondSessionRequest(topic: string, id: number, result: unknown): Promise<void> {
    this._ensureInitialized();
    try {
      await this.client.respondSessionRequest({
        topic,
        response: { id, jsonrpc: '2.0', result },
      });
    } catch (e) {
      throw mapSdkError(e);
    }
  }

  /** Reject a session request explicitly (e.g. user tapped Reject). */
  async rejectSessionRequest(topic: string, id: number, message = 'User rejected'): Promise<void> {
    this._ensureInitialized();
    try {
      await this.client.respondSessionRequest({
        topic,
        response: {
          id,
          jsonrpc: '2.0',
          error:
            this.sdkErr?.('USER_REJECTED') ?? { code: WcErrorCode.USER_REJECTED, message },
        },
      });
    } catch (e) {
      throw mapSdkError(e);
    }
  }

  private async _dispatchRequest(request: WcRequest): Promise<void> {
    const handler = this.requestHandler;
    if (!handler || !this.client) return;

    const { id, topic } = request;

    try {
      const result = await handler(topic, request);
      await this.client.respondSessionRequest({
        topic,
        response: { id, jsonrpc: '2.0', result },
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      const sdkRejected = this.sdkErr?.('USER_REJECTED') ?? {
        code: WcErrorCode.USER_REJECTED,
        message: 'User rejected.',
      };
      const errorPayload =
        msg.toLowerCase().includes('wallet locked') || msg.toLowerCase().includes('locked')
          ? { code: WcErrorCode.WALLET_LOCKED, message: msg }
          : sdkRejected;
      try {
        await this.client.respondSessionRequest({
          topic,
          response: { id, jsonrpc: '2.0', error: errorPayload },
        });
      } catch {
        // Last resort: swallow — the relay may already have torn down
        // the topic.
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

  isInitialized(): boolean {
    return this.initialized;
  }

  private _ensureInitialized(): void {
    if (!this.initialized || !this.client) {
      throw new WalletConnectError(
        'WalletConnectManager not initialized — call init() first',
        WcErrorCode.NOT_INITIALIZED,
      );
    }
  }
}

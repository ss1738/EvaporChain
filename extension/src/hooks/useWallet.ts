/**
 * Zustand store for wallet state management.
 * Single source of truth for the extension popup.
 */

import { create } from "zustand";
import { BrowserKeyStore, type KeyEntry, signMessage } from "@/crypto/keystore";
import {
  api,
  type AccountDetail,
  type StateObject,
  type ChainStatus,
  type TokenInfo,
  type SwapResult,
  type NftItem,
  type GhostObject,
  type GhostDetail,
  type RefreshCostEstimate,
  type SocialAuthResult,
  type TxStatus,
  type PatronageStatusResp,
  type PatronageImmuneResp,
  type PatronagePledgeReq,
  type PatronageActionReq,
  type PatronagePledgeResp,
  type PatronageHonourResp,
  type PatronageRevokeResp,
  type RefreshPoolStatus,
  type FeeControllerStatus,
  type ForkChoiceModeStatus,
} from "@/utils/api";
import type { WcSession, WcSessionProposal } from "@/utils/walletconnect";
import { ledgerManager, type LedgerAccount } from "@/utils/ledger";
import { type BridgeTransfer } from "@/utils/bridge";
import { loadPreferences, savePreferences, type UserPreferences } from "@/utils/preferences";

export type View =
  | "locked"
  | "create"
  | "import"
  | "home"
  | "send"
  | "receive"
  | "objects"
  | "activity"
  | "settings"
  | "backup"
  | "portfolio"
  | "swap"
  | "nfts"
  | "nft-detail"
  | "buy"
  | "batch-refresh"
  | "ghost-recovery"
  | "energy-dashboard"
  | "social-login"
  | "tutorial"
  | "decay-forecast"
  | "walletconnect"
  | "ledger"
  | "bridge"
  | "plugins"
  | "ai-assistant"
  | "patronage"
  | "refresh-pool"
  | "governance";

export type PendingTxKind = "transfer" | "swap" | "resurrect" | "batch_refresh";

export interface PendingTx {
  hash: string;
  kind: PendingTxKind;
  summary: string;
  submittedAt: number;
  status: TxStatus["state"];
  blockHeight?: number;
  error?: string;
  /** Filled when the tx finalises so we can clear it after a 10s grace. */
  finalisedAt?: number;
}

interface WalletState {
  // Auth
  isUnlocked: boolean;
  password: string | null;

  // Keystore
  keystore: BrowserKeyStore | null;
  accounts: KeyEntry[];
  activeAccount: KeyEntry | null;

  // Chain state
  balance: number;
  nonce: number;
  objects: StateObject[];
  chainStatus: ChainStatus | null;
  tokens: TokenInfo[];
  nfts: NftItem[];
  selectedNft: NftItem | null;

  // Ghost recovery
  ghosts: GhostObject[];
  selectedGhost: GhostDetail | null;

  // Social / Onboarding
  tutorialComplete: boolean;

  // WalletConnect
  wcSessions: WcSession[];
  wcPendingProposal: WcSessionProposal | null;

  // Ledger
  ledgerConnected: boolean;
  ledgerAccounts: LedgerAccount[];

  // Bridge
  bridgeTransfers: BridgeTransfer[];

  // Tx status tracking
  pendingTxs: PendingTx[];

  // Patronage
  patronageStatus: PatronageStatusResp | null;
  patronageImmunities: Record<string, PatronageImmuneResp>;

  // Substrate visibility surfaces
  refreshPool: RefreshPoolStatus | null;
  feeStatus: FeeControllerStatus | null;
  /** Last N Lyapunov deltas for the fee-controller widget sparkline. */
  feeDriftHistory: number[];
  /** Demurrage owed on the active account's idle balance, in EVAP. */
  demurrageOwed: number | null;
  forkChoiceMode: ForkChoiceModeStatus | null;

  // UI
  view: View;
  loading: boolean;
  error: string | null;
  notification: string | null;

  // Network
  nodeUrl: string;

  // Preferences
  preferences: UserPreferences;

  // Actions
  init: () => Promise<void>;
  unlock: (password: string) => Promise<void>;
  lock: () => void;
  createAccount: (name: string, password: string) => Promise<string>;
  switchAccount: (name: string) => void;
  refreshBalance: () => Promise<void>;
  refreshObjects: () => Promise<void>;
  refreshChainStatus: () => Promise<void>;
  sendTransfer: (to: string, amount: number) => Promise<TxSendResult>;
  signTransaction: (txPayload: string) => Promise<{ signature: string; publicKey: string }>;
  claimFaucet: () => Promise<void>;
  refreshTokens: () => Promise<void>;
  swapTokens: (fromToken: string, toToken: string, amount: number, slippage: number) => Promise<SwapResult>;
  refreshNfts: () => Promise<void>;
  selectNft: (nft: NftItem | null) => void;
  refreshGhosts: () => Promise<void>;
  selectGhost: (id: string | null) => Promise<void>;
  resurrectGhost: (id: string, energy: number) => Promise<void>;
  batchRefreshObjects: (objects: Array<{ id: string; energy: number }>) => Promise<void>;
  socialLogin: (provider: "google" | "apple") => Promise<void>;
  completeTutorial: () => void;
  setWcSessions: (sessions: WcSession[]) => void;
  setWcPendingProposal: (proposal: WcSessionProposal | null) => void;
  connectLedger: () => Promise<void>;
  disconnectLedger: () => Promise<void>;
  importLedgerAccounts: (accounts: LedgerAccount[]) => void;
  addBridgeTransfer: (transfer: BridgeTransfer) => void;
  setView: (view: View) => void;
  setError: (error: string | null) => void;
  setNotification: (msg: string | null) => void;
  setNodeUrl: (url: string) => void;
  updatePreferences: (prefs: Partial<UserPreferences>) => Promise<void>;

  // Tx tracking
  trackTx: (hash: string, kind: PendingTxKind, summary: string) => void;
  pollTxStatuses: () => Promise<void>;
  clearTx: (hash: string) => void;

  // Patronage
  refreshPatronage: () => Promise<void>;
  refreshPatronageImmunity: (objectIdHex: string, epoch: number) => Promise<void>;
  pledgePatronage: (req: PatronagePledgeReq) => Promise<PatronagePledgeResp>;
  honourPatronage: (req: PatronageActionReq) => Promise<PatronageHonourResp>;
  revokePatronage: (req: PatronageActionReq) => Promise<PatronageRevokeResp>;

  // Substrate visibility surfaces
  refreshRefreshPool: () => Promise<void>;
  refreshFeeStatus: () => Promise<void>;
  refreshDemurrage: () => Promise<void>;
  refreshForkChoiceMode: () => Promise<void>;
}

interface TxSendResult {
  success: boolean;
  message: string;
}

/**
 * Module-level timer for tx-status polling. Kept outside the store
 * so init() is idempotent — re-entry just resets the interval. Each
 * tick short-circuits when pendingTxs is empty so the interval is
 * effectively idle until trackTx() registers a hash.
 */
let txPollTimer: ReturnType<typeof setInterval> | null = null;

export const useWallet = create<WalletState>((set, get) => ({
  isUnlocked: false,
  password: null,
  keystore: null,
  accounts: [],
  activeAccount: null,
  balance: 0,
  nonce: 0,
  objects: [],
  chainStatus: null,
  tokens: [],
  nfts: [],
  selectedNft: null,
  ghosts: [],
  selectedGhost: null,
  wcSessions: [],
  wcPendingProposal: null,
  ledgerConnected: false,
  ledgerAccounts: [],
  bridgeTransfers: [],
  pendingTxs: [],
  patronageStatus: null,
  patronageImmunities: {},
  refreshPool: null,
  feeStatus: null,
  feeDriftHistory: [],
  demurrageOwed: null,
  forkChoiceMode: null,
  tutorialComplete: (() => { try { return localStorage.getItem("evaporchain_tutorial_complete") === "true"; } catch { return false; } })(),
  view: "locked",
  loading: false,
  error: null,
  notification: null,
  nodeUrl: "https://testnet.evaporchain.com",
  preferences: {
    nodeUrl: "https://testnet.evaporchain.com",
    currency: "USD",
    autoLockMinutes: 15,
    defaultSlippage: 0.5,
    hideSmallBalances: false,
    notificationsEnabled: true,
  },

  init: async () => {
    const [ks, prefs] = await Promise.all([
      BrowserKeyStore.load(),
      loadPreferences(),
    ]);
    const accounts = ks.listAccounts();
    const active = ks.getActiveAccount();
    api.setNode(prefs.nodeUrl);
    // Fresh installs only land on the simulated social-login flow in dev.
    // In prod the OAuth backend isn't wired up yet, so we route to the
    // real `create` flow instead. TODO real OAuth.
    const freshInstallView: View = import.meta.env.DEV ? "social-login" : "create";
    set({
      keystore: ks,
      accounts,
      activeAccount: active,
      view: accounts.length === 0 ? freshInstallView : "locked",
      nodeUrl: prefs.nodeUrl,
      preferences: prefs,
    });

    // Start the tx-status poll once. The tick body short-circuits
    // when pendingTxs is empty so this is idle until a broadcast
    // calls trackTx().
    if (txPollTimer == null) {
      txPollTimer = setInterval(() => {
        if (get().pendingTxs.length > 0) {
          get().pollTxStatuses().catch(() => { /* tolerate transient API errors */ });
        }
      }, 3000);
    }
  },

  unlock: async (password: string) => {
    const { keystore, activeAccount } = get();
    if (!keystore || !activeAccount) throw new Error("No account to unlock");

    set({ loading: true, error: null });
    try {
      // Verify password by attempting decryption
      await keystore.unlockKey(activeAccount.name, password);
      set({ isUnlocked: true, password, view: "home", loading: false });

      // Fetch balance in background
      get().refreshBalance();
      get().refreshChainStatus();
    } catch {
      set({ loading: false, error: "Wrong password" });
    }
  },

  lock: () => {
    set({
      isUnlocked: false,
      password: null,
      view: "locked",
      balance: 0,
      nonce: 0,
      objects: [],
      error: null,
    });
  },

  createAccount: async (name: string, password: string) => {
    set({ loading: true, error: null });
    try {
      let ks = get().keystore;
      if (!ks) {
        ks = new BrowserKeyStore();
      }
      const address = await ks.generateKey(name, password);
      const accounts = ks.listAccounts();
      const active = ks.getActiveAccount();
      set({
        keystore: ks,
        accounts,
        activeAccount: active,
        isUnlocked: true,
        password,
        view: "home",
        loading: false,
      });
      return address;
    } catch (e: any) {
      set({ loading: false, error: e.message });
      throw e;
    }
  },

  switchAccount: (name: string) => {
    const { keystore } = get();
    if (!keystore) return;
    keystore.setActiveAccount(name);
    const active = keystore.getActiveAccount();
    set({ activeAccount: active, balance: 0, nonce: 0, objects: [] });
    keystore.save();
    get().refreshBalance();
  },

  refreshBalance: async () => {
    const { activeAccount } = get();
    if (!activeAccount) return;
    try {
      const detail = await api.getAddressDetail(activeAccount.address);
      set({ balance: detail.balance, nonce: detail.nonce });
    } catch {
      // Node might be unreachable — keep last known balance
    }
  },

  refreshObjects: async () => {
    const { activeAccount } = get();
    if (!activeAccount) return;
    try {
      const objects = await api.getObjectsByOwner(activeAccount.address);
      set({ objects });
    } catch {
      // Ignore
    }
  },

  refreshChainStatus: async () => {
    try {
      const status = await api.getStatus();
      set({ chainStatus: status });
    } catch {
      // Ignore
    }
  },

  sendTransfer: async (to: string, amount: number) => {
    const { activeAccount, nonce, password, keystore } = get();
    if (!activeAccount) throw new Error("No active account");
    if (!password || !keystore) throw new Error("Wallet locked");

    set({ loading: true, error: null });
    try {
      // 1. Decrypt private key
      const secretKey = await keystore.unlockKey(activeAccount.name, password);

      // 2. Build transaction payload to sign
      const txPayload = JSON.stringify({
        type: "transfer",
        from: activeAccount.address,
        to,
        amount,
        nonce,
      });
      const txBytes = new TextEncoder().encode(txPayload);

      // 3. Sign with real ML-DSA-65 via WASM
      const signature = await signMessage(secretKey, txBytes);

      // 4. Encode signature and public key as hex for the API
      const sigHex = Array.from(signature).map(b => b.toString(16).padStart(2, "0")).join("");
      const pubKeyHex = activeAccount.publicKey;

      // 5. Broadcast signed transaction
      const result = await api.transfer(activeAccount.address, to, amount, nonce, sigHex, pubKeyHex);
      if (result.success) {
        set({ nonce: nonce + 1, loading: false, notification: `Sent ${amount} EVAP` });
        if (result.hash) {
          get().trackTx(result.hash, "transfer", `Sent ${amount} EVAP to ${to.slice(0, 10)}…`);
        }
        get().refreshBalance();
      } else {
        set({ loading: false, error: result.message });
      }
      return result;
    } catch (e: any) {
      set({ loading: false, error: e.message });
      return { success: false, message: e.message };
    }
  },

  /**
   * Sign a raw transaction payload and return the hex signature + public key.
   * Used by the dApp approval flow and WalletConnect.
   */
  signTransaction: async (txPayload: string): Promise<{ signature: string; publicKey: string }> => {
    const { activeAccount, password, keystore } = get();
    if (!activeAccount || !password || !keystore) throw new Error("Wallet locked");

    const secretKey = await keystore.unlockKey(activeAccount.name, password);
    const txBytes = new TextEncoder().encode(txPayload);
    const sig = await signMessage(secretKey, txBytes);
    const sigHex = Array.from(sig).map(b => b.toString(16).padStart(2, "0")).join("");
    return { signature: sigHex, publicKey: activeAccount.publicKey };
  },

  refreshTokens: async () => {
    try {
      const tokens = await api.getTokens();
      set({ tokens });
    } catch {
      // Ignore — tokens endpoint may not be available
    }
  },

  swapTokens: async (fromToken: string, toToken: string, amount: number, slippage: number) => {
    const { activeAccount } = get();
    set({ loading: true, error: null });
    try {
      // Sign the swap transaction
      const txPayload = JSON.stringify({ type: "swap", from_token: fromToken, to_token: toToken, amount, slippage, from: activeAccount?.address });
      const { signature, publicKey } = await get().signTransaction(txPayload);
      const result = await api.executeSwap(fromToken, toToken, amount, slippage, signature, publicKey);
      if (result.success) {
        set({ loading: false, notification: `Swapped ${result.amount_in} ${fromToken} for ${result.amount_out} ${toToken}` });
        if (result.hash) {
          get().trackTx(result.hash, "swap", `Swap ${result.amount_in} ${fromToken} → ${result.amount_out} ${toToken}`);
        }
        get().refreshBalance();
        get().refreshTokens();
      } else {
        set({ loading: false, error: result.message });
      }
      return result;
    } catch (e: any) {
      set({ loading: false, error: e.message });
      return { success: false, message: e.message, amount_in: 0, amount_out: 0 };
    }
  },

  claimFaucet: async () => {
    const { activeAccount } = get();
    if (!activeAccount) return;

    set({ loading: true, error: null });
    try {
      const result = await api.claimFaucet(activeAccount.address);
      if (result.success) {
        set({ loading: false, notification: `Claimed! Balance: ${result.balance} EVAP` });
        get().refreshBalance();
      } else {
        set({ loading: false, error: result.message ?? "Faucet cooldown" });
      }
    } catch (e: any) {
      set({ loading: false, error: e.message });
    }
  },

  refreshNfts: async () => {
    const { activeAccount } = get();
    if (!activeAccount) return;
    try {
      const nfts = await api.getNftsByOwner(activeAccount.address);
      set({ nfts });
    } catch {
      // Ignore — NFT endpoint may not be available
    }
  },

  selectNft: (nft: NftItem | null) => {
    set({ selectedNft: nft });
    if (nft) {
      set({ view: "nft-detail" });
    }
  },

  refreshGhosts: async () => {
    const { activeAccount } = get();
    if (!activeAccount) return;
    try {
      const ghosts = await api.getGhosts(activeAccount.address);
      set({ ghosts });
    } catch {
      // Ghost endpoint may not be available
    }
  },

  selectGhost: async (id: string | null) => {
    if (!id) {
      set({ selectedGhost: null });
      return;
    }
    try {
      const detail = await api.getGhostDetail(id);
      set({ selectedGhost: detail });
    } catch {
      // Ignore
    }
  },

  resurrectGhost: async (id: string, energy: number) => {
    const { activeAccount } = get();
    set({ loading: true, error: null });
    try {
      // Sign the resurrect transaction
      const txPayload = JSON.stringify({ type: "resurrect", object_id: id, energy_deposit: energy, from: activeAccount?.address });
      const { signature, publicKey } = await get().signTransaction(txPayload);
      const result = await api.resurrectObject(id, energy, signature, publicKey);
      if (result.success) {
        set({ loading: false, notification: `Resurrected object! Spent ${energy} EVAP` });
        if (result.hash) {
          get().trackTx(result.hash, "resurrect", `Resurrected ${id.slice(0, 12)}… (${energy} EVAP)`);
        }
        get().refreshGhosts();
        get().refreshBalance();
        get().refreshObjects();
      } else {
        set({ loading: false, error: result.message });
      }
    } catch (e: any) {
      set({ loading: false, error: e.message });
    }
  },

  batchRefreshObjects: async (objects: Array<{ id: string; energy: number }>) => {
    const { activeAccount } = get();
    set({ loading: true, error: null });
    try {
      // Sign the batch refresh transaction
      const txPayload = JSON.stringify({ type: "batch_refresh", objects, from: activeAccount?.address });
      const { signature, publicKey } = await get().signTransaction(txPayload);
      const result = await api.batchRefresh(objects, signature, publicKey);
      if (result.success) {
        const totalEnergy = objects.reduce((sum, o) => sum + o.energy, 0);
        set({
          loading: false,
          notification: `Refreshed ${objects.length} objects, spent ${totalEnergy} EVAP`,
        });
        if (result.hash) {
          get().trackTx(result.hash, "batch_refresh", `Batch refresh ${objects.length} objects (${totalEnergy} EVAP)`);
        }
        get().refreshBalance();
        get().refreshObjects();
      } else {
        set({ loading: false, error: result.message });
      }
    } catch (e: any) {
      set({ loading: false, error: e.message });
    }
  },

  socialLogin: async (provider: "google" | "apple") => {
    set({ loading: true, error: null });
    try {
      // In production, this would open an OAuth popup and get a real token.
      // For now, we simulate the OAuth flow by passing a placeholder token.
      const token = `${provider}_oauth_token_${Date.now()}`;
      const result = await api.socialAuth(provider, token);

      if (result.success) {
        // Auto-generate a local keystore entry from the social auth result
        let ks = get().keystore;
        if (!ks) {
          ks = new BrowserKeyStore();
        }
        const name = `${provider}-${result.address.slice(0, 8)}`;
        const autoPassword = result.encrypted_key.slice(0, 32);
        await ks.generateKey(name, autoPassword);
        const accounts = ks.listAccounts();
        const active = ks.getActiveAccount();

        const tutorialDone = get().tutorialComplete;
        set({
          keystore: ks,
          accounts,
          activeAccount: active,
          isUnlocked: true,
          password: autoPassword,
          loading: false,
          view: tutorialDone ? "home" : "tutorial",
        });

        // Fetch balance in background
        get().refreshBalance();
        get().refreshChainStatus();
      } else {
        set({ loading: false, error: result.message ?? "Social login failed" });
      }
    } catch (e: any) {
      set({ loading: false, error: e.message });
    }
  },

  completeTutorial: () => {
    try {
      localStorage.setItem("evaporchain_tutorial_complete", "true");
    } catch {
      // localStorage may not be available
    }
    set({ tutorialComplete: true, view: "home" });
  },

  setWcSessions: (sessions: WcSession[]) => set({ wcSessions: sessions }),
  setWcPendingProposal: (proposal: WcSessionProposal | null) => set({ wcPendingProposal: proposal }),

  connectLedger: async () => {
    set({ loading: true, error: null });
    try {
      const connected = await ledgerManager.connect();
      if (connected) {
        const accounts = await ledgerManager.getAccounts(5);
        set({ ledgerConnected: true, ledgerAccounts: accounts, loading: false });
      } else {
        set({ loading: false, error: "No Ledger device found" });
      }
    } catch (e: any) {
      set({ loading: false, error: e.message });
    }
  },

  disconnectLedger: async () => {
    await ledgerManager.disconnect();
    set({ ledgerConnected: false, ledgerAccounts: [] });
  },

  importLedgerAccounts: (ledgerAccts: LedgerAccount[]) => {
    // Store ledger accounts alongside software accounts.
    // In production, these would be persisted with a "hardware" flag in the keystore.
    set({ ledgerConnected: true, ledgerAccounts: ledgerAccts });
  },

  addBridgeTransfer: (transfer: BridgeTransfer) => {
    set({ bridgeTransfers: [transfer, ...get().bridgeTransfers] });
  },

  setView: (view: View) => set({ view, error: null }),
  setError: (error: string | null) => set({ error }),
  setNotification: (msg: string | null) => set({ notification: msg }),
  setNodeUrl: (url: string) => {
    api.setNode(url);
    set((state) => ({
      nodeUrl: url,
      preferences: { ...state.preferences, nodeUrl: url },
    }));
    savePreferences({ nodeUrl: url });
  },

  updatePreferences: async (prefs: Partial<UserPreferences>) => {
    await savePreferences(prefs);
    set((state) => ({
      preferences: { ...state.preferences, ...prefs },
      ...(prefs.nodeUrl != null ? { nodeUrl: prefs.nodeUrl } : {}),
    }));
    if (prefs.nodeUrl != null) {
      api.setNode(prefs.nodeUrl);
    }
  },

  // ── Tx tracking ────────────────────────────────────────────────

  trackTx: (hash, kind, summary) => {
    const existing = get().pendingTxs.find(t => t.hash === hash);
    if (existing) return;
    set({
      pendingTxs: [
        ...get().pendingTxs,
        { hash, kind, summary, submittedAt: Date.now(), status: "pending" },
      ],
    });
  },

  clearTx: (hash) => {
    set({ pendingTxs: get().pendingTxs.filter(t => t.hash !== hash) });
  },

  pollTxStatuses: async () => {
    const txs = get().pendingTxs;
    if (txs.length === 0) return;

    const now = Date.now();
    // First sweep: drop anything that finalised more than 10s ago.
    const live = txs.filter(t => !t.finalisedAt || now - t.finalisedAt < 10_000);

    // Query each non-terminal tx.
    const updated = await Promise.all(live.map(async (tx) => {
      // Skip API call if already finalised/rejected — just preserve.
      if (tx.status === "finalised" || tx.status === "rejected") return tx;
      try {
        const status = await api.getTxStatus(tx.hash);
        if (status.state === tx.status) return tx; // no change
        const next: PendingTx = {
          ...tx,
          status: status.state,
          blockHeight: status.block_height ?? tx.blockHeight,
          error: status.error,
        };
        if (status.state === "finalised" || status.state === "rejected") {
          next.finalisedAt = Date.now();
        }
        return next;
      } catch {
        return tx;
      }
    }));

    set({ pendingTxs: updated });
  },

  // ── Patronage ─────────────────────────────────────────────────

  refreshPatronage: async () => {
    try {
      const status = await api.getPatronageStatus();
      set({ patronageStatus: status });
    } catch {
      // Endpoint unavailable; leave previous value in place.
    }
  },

  refreshPatronageImmunity: async (objectIdHex, epoch) => {
    try {
      const info = await api.getPatronageImmunity(objectIdHex, epoch);
      set({ patronageImmunities: { ...get().patronageImmunities, [objectIdHex]: info } });
    } catch {
      // Ignore — immunity unknown stays absent from map.
    }
  },

  pledgePatronage: async (req) => {
    set({ loading: true, error: null });
    try {
      const resp = await api.pledgePatronage(req);
      if (resp.status === "pledged") {
        set({ loading: false, notification: `Patronage pledged: ${resp.pre_funded} EVAP until epoch ${resp.expires_epoch}` });
        await get().refreshPatronage();
        await get().refreshPatronageImmunity(req.object_id_hex, req.current_epoch);
      } else {
        set({ loading: false, error: resp.detail || "Pledge failed" });
      }
      return resp;
    } catch (e: any) {
      set({ loading: false, error: e.message });
      throw e;
    }
  },

  honourPatronage: async (req) => {
    set({ loading: true, error: null });
    try {
      const resp = await api.honourPatronage(req);
      if (resp.status === "honoured") {
        set({ loading: false, notification: `Honoured ${resp.donated} EVAP; score now ${resp.patronage_score}` });
        await get().refreshPatronage();
        await get().refreshPatronageImmunity(req.object_id_hex, req.epoch);
      } else {
        set({ loading: false, error: resp.detail || "Honour failed" });
      }
      return resp;
    } catch (e: any) {
      set({ loading: false, error: e.message });
      throw e;
    }
  },

  revokePatronage: async (req) => {
    set({ loading: true, error: null });
    try {
      const resp = await api.revokePatronage(req);
      if (resp.status === "revoked") {
        set({ loading: false, notification: `Covenant revoked; ${resp.refunded ?? 0} EVAP refunded` });
        await get().refreshPatronage();
        await get().refreshPatronageImmunity(req.object_id_hex, req.epoch);
      } else {
        set({ loading: false, error: resp.detail || "Revoke failed" });
      }
      return resp;
    } catch (e: any) {
      set({ loading: false, error: e.message });
      throw e;
    }
  },

  // ── Substrate visibility surfaces ─────────────────────────────

  refreshRefreshPool: async () => {
    try {
      const pool = await api.getRefreshPool();
      set({ refreshPool: pool });
    } catch {
      // Endpoint unavailable; keep previous value.
    }
  },

  refreshFeeStatus: async () => {
    try {
      const status = await api.getFeeControllerStatus();
      // Append a synthetic drift sample derived from the energy delta so
      // the widget sparkline shows movement even when the chain isn't
      // calling /step. Real drift comes from /step responses; this is a
      // best-effort proxy until the controller is stepped explicitly.
      const prev = get().feeStatus;
      const driftProxy = prev ? status.energy - prev.energy : 0;
      const history = [...get().feeDriftHistory, driftProxy].slice(-24);
      set({ feeStatus: status, feeDriftHistory: history });
    } catch {
      // Endpoint unavailable.
    }
  },

  refreshDemurrage: async () => {
    // The /api/address/:addr endpoint does NOT expose last_touched_epoch
    // (see crates/evaporchain-node/src/api.rs §AddressDetailResponse,
    // L7389-7397). Without it we cannot meaningfully compute demurrage
    // owed on the active account. The badge component therefore gates
    // itself behind import.meta.env.DEV and uses last_touched_epoch=0
    // for demonstration. This action keeps the surface live so once
    // last_touched_epoch is added to the address response it becomes a
    // single edit here. TODO: wire real last_touched_epoch.
    const { activeAccount, balance, chainStatus } = get();
    if (!activeAccount || !chainStatus) {
      set({ demurrageOwed: null });
      return;
    }
    try {
      const resp = await api.getDemurrageOwed({
        balance,
        last_touched_epoch: 0,
        current_epoch: chainStatus.epoch,
        // Default genesis params: lambda_base 1 ppm/epoch above 1024 threshold.
        lambda_base_ppm: 1,
        threshold: 1024,
      });
      set({ demurrageOwed: resp.is_disabled ? 0 : resp.owed });
    } catch {
      set({ demurrageOwed: null });
    }
  },

  refreshForkChoiceMode: async () => {
    try {
      const mode = await api.getForkChoiceMode();
      set({ forkChoiceMode: mode });
    } catch {
      // Endpoint unavailable.
    }
  },
}));

// Test-only escape hatch: Playwright e2e specs read the store via
// `globalThis.__zustandStore`. Fenced behind Vite's MODE so it cannot
// leak into production builds (`npm run build` runs as MODE=production).
if (import.meta.env.MODE === "test") { (globalThis as any).__zustandStore = useWallet; }

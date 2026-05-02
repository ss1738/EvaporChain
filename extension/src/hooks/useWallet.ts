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
  type DsnStatus,
  type BellBeaconReq,
  type SettleDemurrageResp,
  type ShardInfo,
  type ShardsHealth,
} from "@/utils/api";
import type { WcSession, WcSessionProposal } from "@/utils/walletconnect";
import { ledgerManager, type LedgerAccount } from "@/utils/ledger";
import { type BridgeTransfer } from "@/utils/bridge";
import {
  loadPreferences,
  savePreferences,
  resolveNodeUrl,
  MAINNET_URL,
  TESTNET_URL,
  type UserPreferences,
  type NetworkKind,
} from "@/utils/preferences";
import {
  loadContacts,
  saveContact as persistContact,
  removeContact as persistRemoveContact,
  type Contact,
} from "@/utils/contacts";

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
  | "governance"
  | "dsn-details"
  | "shards"
  | "contacts"
  | "da-verify";

export type PendingTxKind = "transfer" | "swap" | "resurrect" | "batch_refresh" | "settle_demurrage";

/** Transient toast surface for tx-finalisation notifications. */
export interface Toast {
  id: string;
  kind: "finalised" | "rejected";
  /** Short summary copied from the finalising PendingTx. */
  summary: string;
  /** Tx hash (for the click-to-copy chip). */
  hash: string;
  /** Wall-clock ms when the toast was pushed. */
  createdAt: number;
}

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
  /** Confirmations on top of the inclusion block (head − inclusion). */
  confirmations?: number;
  /** FIFO position in the mempool while `status === "mempool"`. */
  mempoolPosition?: number;
  /** Total mempool depth at the most recent poll. */
  mempoolSize?: number;
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

  // Toast queue — transient bottom-center surface for finalised/rejected
  // txs. Each toast auto-dismisses after 4s (handled in the renderer).
  toasts: Toast[];

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
  /** Cached `last_touched_epoch` for the active account, from the most
   *  recent `/api/address/:addr` response. `null` until the first
   *  refreshBalance / refreshDemurrage roundtrip. */
  accountLastTouched: number | null;
  forkChoiceMode: ForkChoiceModeStatus | null;
  /** DSN privacy-window status: total folded count + aggregate root. */
  dsnStatus: DsnStatus | null;
  /** Last Bell-Beacon CHSH S-value (milli-units), from a single read. */
  bellSValue: number | null;
  /** Bell threshold from the same read; cached so the badge can render
   *  without re-fetching. */
  bellThreshold: number | null;
  /** Whether the last Bell read certified S > threshold. */
  bellCertified: boolean | null;

  // Sharding visibility — per-shard health rows (or null if not yet
  // refreshed / endpoint unavailable). The full snapshot lives in
  // `shardsHealth`; `shards` is a convenience alias for the rows.
  shards: ShardInfo[] | null;
  shardsHealth: ShardsHealth | null;
  /** Computed shard for the active account's address; null when
   *  sharding is disabled or not yet refreshed. */
  addressShard: number | null;

  // UI
  view: View;
  loading: boolean;
  error: string | null;
  notification: string | null;

  // Network
  nodeUrl: string;

  // Preferences
  preferences: UserPreferences;

  // Address book / contacts (persisted via chrome.storage.local)
  contacts: Contact[];

  // Multi-account batch view — refreshed by `refreshAllBalances`. Maps
  // each known account address to its most recent balance/nonce snapshot
  // plus a wall-clock `lastFetched` ms timestamp so the UI can render a
  // "stale Xs ago" indicator on non-active accounts.
  accountBalances: Record<string, {
    balance: number;
    nonce: number;
    last_touched_epoch: number;
    lastFetched: number;
  }>;

  // Actions
  init: () => Promise<void>;
  unlock: (password: string) => Promise<void>;
  lock: () => void;
  createAccount: (name: string, password: string) => Promise<string>;
  switchAccount: (name: string) => void;
  refreshBalance: () => Promise<void>;
  /** Batch-refresh balances for every known account in parallel. */
  refreshAllBalances: () => Promise<void>;
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
  /** Resolves the node URL for the currently selected network. */
  getActiveNodeUrl: () => string;
  /** Switch network kind (mainnet/testnet/custom) and re-point the API client. */
  setNetwork: (network: NetworkKind, customUrl?: string) => Promise<void>;
  updatePreferences: (prefs: Partial<UserPreferences>) => Promise<void>;

  // Contacts / address book
  refreshContacts: () => Promise<void>;
  addContact: (contact: Contact) => Promise<void>;
  removeContact: (address: string) => Promise<void>;

  // Tx tracking
  trackTx: (hash: string, kind: PendingTxKind, summary: string) => void;
  pollTxStatuses: () => Promise<void>;
  clearTx: (hash: string) => void;

  // Toast queue
  pushToast: (toast: Omit<Toast, "id" | "createdAt">) => void;
  dismissToast: (id: string) => void;

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
  refreshDsnStatus: () => Promise<void>;
  refreshBellBeacon: (req?: BellBeaconReq) => Promise<void>;

  // Sharding — pulls /api/shards + /api/shards/health and recomputes
  // `addressShard` for the active account.
  refreshShards: () => Promise<void>;

  // Demurrage settlement (real on-chain debit + refresh-pool credit)
  settleDemurrage: () => Promise<SettleDemurrageResp | null>;
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
  toasts: [],
  patronageStatus: null,
  patronageImmunities: {},
  refreshPool: null,
  feeStatus: null,
  feeDriftHistory: [],
  demurrageOwed: null,
  accountLastTouched: null,
  forkChoiceMode: null,
  dsnStatus: null,
  bellSValue: null,
  bellThreshold: null,
  bellCertified: null,
  shards: null,
  shardsHealth: null,
  addressShard: null,
  tutorialComplete: (() => { try { return localStorage.getItem("evaporchain_tutorial_complete") === "true"; } catch { return false; } })(),
  view: "locked",
  loading: false,
  error: null,
  notification: null,
  nodeUrl: TESTNET_URL,
  preferences: {
    nodeUrl: TESTNET_URL,
    network: "testnet",
    customNodeUrl: TESTNET_URL,
    currency: "USD",
    autoLockMinutes: 15,
    defaultSlippage: 0.5,
    hideSmallBalances: false,
    notificationsEnabled: true,
    lockOnBlur: false,
    lockOnTabClose: true,
  },
  contacts: [],
  accountBalances: {},

  init: async () => {
    const [ks, prefs, contacts] = await Promise.all([
      BrowserKeyStore.load(),
      loadPreferences(),
      loadContacts(),
    ]);
    const accounts = ks.listAccounts();
    const active = ks.getActiveAccount();
    const activeUrl = resolveNodeUrl(prefs);
    api.setNode(activeUrl);
    // Fresh installs only land on the simulated social-login flow in dev.
    // In prod the OAuth backend isn't wired up yet, so we route to the
    // real `create` flow instead. TODO real OAuth.
    const freshInstallView: View = import.meta.env.DEV ? "social-login" : "create";
    set({
      keystore: ks,
      accounts,
      activeAccount: active,
      view: accounts.length === 0 ? freshInstallView : "locked",
      nodeUrl: activeUrl,
      preferences: prefs,
      contacts,
    });

    // Batch-refresh balances for every account so the accounts dropdown
    // can render fresh balances without waiting for the user to switch
    // accounts. Best-effort — node may be unreachable.
    get().refreshAllBalances().catch(() => { /* swallow */ });

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

    // DSN privacy-set status — fetched once on init alongside chain
    // status. Subsequent refreshes happen after each tx finalises, in
    // pollTxStatuses() below.
    get().refreshDsnStatus().catch(() => { /* swallow */ });
    // Sharding snapshot — same lifecycle: refreshed once on init plus
    // after each finalised tx so per-shard health/object counts move
    // when objects evaporate.
    get().refreshShards().catch(() => { /* swallow */ });
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
      const lastTouched = detail.last_touched_epoch ?? 0;
      set((state) => ({
        balance: detail.balance,
        nonce: detail.nonce,
        // Cache last_touched_epoch from the same roundtrip — used by
        // refreshDemurrage and DemurrageBadge gating.
        accountLastTouched: lastTouched,
        // Mirror into the batch view so the accounts dropdown renders
        // fresh balances without waiting for the next sweep.
        accountBalances: {
          ...state.accountBalances,
          [activeAccount.address]: {
            balance: detail.balance,
            nonce: detail.nonce,
            last_touched_epoch: lastTouched,
            lastFetched: Date.now(),
          },
        },
      }));
    } catch {
      // Node might be unreachable — keep last known balance
    }
  },

  refreshAllBalances: async () => {
    const { accounts } = get();
    if (accounts.length === 0) return;
    // Fan out address-detail fetches in parallel, tolerate partials.
    const results = await Promise.all(
      accounts.map(async (acc) => {
        try {
          const detail = await api.getAddressDetail(acc.address);
          return {
            address: acc.address,
            entry: {
              balance: detail.balance,
              nonce: detail.nonce,
              last_touched_epoch: detail.last_touched_epoch ?? 0,
              lastFetched: Date.now(),
            },
          };
        } catch {
          return null;
        }
      }),
    );
    const next: Record<string, {
      balance: number;
      nonce: number;
      last_touched_epoch: number;
      lastFetched: number;
    }> = { ...get().accountBalances };
    for (const r of results) {
      if (r) next[r.address] = r.entry;
    }
    // If the active account was in the batch, mirror its values into the
    // top-level balance/nonce slice so home renders fresh data too.
    const active = get().activeAccount;
    const activeEntry = active ? next[active.address] : undefined;
    set({
      accountBalances: next,
      ...(activeEntry
        ? {
            balance: activeEntry.balance,
            nonce: activeEntry.nonce,
            accountLastTouched: activeEntry.last_touched_epoch,
          }
        : {}),
    });
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
    // Treat raw setNodeUrl calls as switching to a custom URL — the
    // network kind moves to "custom" and the URL is persisted as
    // customNodeUrl. Mainnet/testnet selection goes through setNetwork.
    api.setNode(url);
    set((state) => ({
      nodeUrl: url,
      preferences: {
        ...state.preferences,
        nodeUrl: url,
        network: "custom",
        customNodeUrl: url,
      },
    }));
    savePreferences({ nodeUrl: url, network: "custom", customNodeUrl: url });
  },

  getActiveNodeUrl: () => {
    return resolveNodeUrl(get().preferences);
  },

  setNetwork: async (network, customUrl) => {
    const prev = get().preferences;
    const nextCustom = customUrl ?? prev.customNodeUrl ?? prev.nodeUrl;
    const nextPrefs: UserPreferences = {
      ...prev,
      network,
      customNodeUrl: nextCustom,
    };
    const url = resolveNodeUrl(nextPrefs);
    nextPrefs.nodeUrl = url;
    api.setNode(url);
    await savePreferences({
      network,
      customNodeUrl: nextCustom,
      nodeUrl: url,
    });
    set({ preferences: nextPrefs, nodeUrl: url });
    // Re-fetch balances against the new endpoint.
    get().refreshAllBalances().catch(() => { /* swallow */ });
  },

  updatePreferences: async (prefs: Partial<UserPreferences>) => {
    await savePreferences(prefs);
    set((state) => {
      const merged = { ...state.preferences, ...prefs };
      // If anything network-shaped changed, recompute the active URL
      // from the merged preferences.
      const networkChanged =
        prefs.network !== undefined ||
        prefs.customNodeUrl !== undefined ||
        prefs.nodeUrl !== undefined;
      const url = networkChanged ? resolveNodeUrl(merged) : state.nodeUrl;
      if (networkChanged) merged.nodeUrl = url;
      return {
        preferences: merged,
        nodeUrl: url,
      };
    });
    const networkChanged =
      prefs.network !== undefined ||
      prefs.customNodeUrl !== undefined ||
      prefs.nodeUrl !== undefined;
    if (networkChanged) {
      api.setNode(get().nodeUrl);
    }
  },

  // ── Contacts / address book ───────────────────────────────────

  refreshContacts: async () => {
    try {
      const contacts = await loadContacts();
      set({ contacts });
    } catch {
      // Storage unavailable — keep previous list.
    }
  },

  addContact: async (contact) => {
    const list = await persistContact(contact);
    set({ contacts: list });
  },

  removeContact: async (address) => {
    const list = await persistRemoveContact(address);
    set({ contacts: list });
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
    let anyNewlyFinalised = false;
    // Track newly-terminal txs in this tick so we can fire a toast for
    // each. We compare against the previous status on the in-memory
    // record before we overwrite it.
    const newlyTerminal: { tx: PendingTx; kind: "finalised" | "rejected" }[] = [];
    const updated = await Promise.all(live.map(async (tx) => {
      // Skip API call if already finalised/rejected — just preserve.
      if (tx.status === "finalised" || tx.status === "rejected") return tx;
      try {
        const status = await api.getTxStatus(tx.hash);
        // Always refresh the live progress fields (confirmations bump as new
        // blocks land, mempool position drops as txs ahead of us drain) even
        // when the high-level state hasn't changed yet.
        const liveUpdates = {
          confirmations: status.confirmations,
          mempoolPosition: status.mempool_position,
          mempoolSize: status.mempool_size,
        };
        if (status.state === tx.status) {
          return { ...tx, ...liveUpdates };
        }
        const next: PendingTx = {
          ...tx,
          ...liveUpdates,
          status: status.state,
          blockHeight: status.block_height ?? tx.blockHeight,
          error: status.error,
        };
        if (status.state === "finalised" || status.state === "rejected") {
          next.finalisedAt = Date.now();
          newlyTerminal.push({ tx: next, kind: status.state });
          if (status.state === "finalised") anyNewlyFinalised = true;
        }
        return next;
      } catch {
        return tx;
      }
    }));

    set({ pendingTxs: updated });

    // Push a transient toast for each tx that newly transitioned to a
    // terminal state. Toasts auto-dismiss in the renderer after 4s.
    for (const { tx, kind } of newlyTerminal) {
      get().pushToast({ kind, summary: tx.summary, hash: tx.hash });
    }

    // Spec: refresh DSN status after each tx finalises so the privacy
    // badge reflects the new accumulator if the tx was a shielded
    // transfer. Same trigger drives refreshShards() so per-shard
    // health/object counts reflect the post-tx state.
    if (anyNewlyFinalised) {
      get().refreshDsnStatus().catch(() => { /* swallow */ });
      get().refreshShards().catch(() => { /* swallow */ });
      // Refresh every account's balance so the accounts dropdown
      // reflects post-tx state without waiting for an account switch.
      get().refreshAllBalances().catch(() => { /* swallow */ });
    }
  },

  // ── Toast queue ────────────────────────────────────────────────

  pushToast: (toast) => {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    set({
      toasts: [
        ...get().toasts,
        { id, createdAt: Date.now(), ...toast },
      ],
    });
  },

  dismissToast: (id) => {
    set({ toasts: get().toasts.filter(t => t.id !== id) });
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
    // /api/address/:addr now exposes `last_touched_epoch` (api.rs
    // §AddressDetailResponse). The node still serves `0` until the
    // Account struct carries a real per-account epoch (see TODO at
    // AddressDetailResponse.last_touched_epoch in api.rs), but once
    // that lands the value below becomes accurate without further TS
    // changes. We pull it via getAddressDetail to ensure a fresh
    // value is cached even if refreshBalance hasn't fired this tick.
    const { activeAccount, chainStatus } = get();
    if (!activeAccount || !chainStatus) {
      set({ demurrageOwed: null });
      return;
    }
    try {
      const detail = await api.getAddressDetail(activeAccount.address);
      const lastTouched = detail.last_touched_epoch ?? 0;
      const resp = await api.getDemurrageOwed({
        balance: detail.balance,
        last_touched_epoch: lastTouched,
        current_epoch: chainStatus.epoch,
        // Default genesis params: lambda_base 1 ppm/epoch above 1024 threshold.
        lambda_base_ppm: 1,
        threshold: 1024,
      });
      set({
        balance: detail.balance,
        nonce: detail.nonce,
        accountLastTouched: lastTouched,
        demurrageOwed: resp.is_disabled ? 0 : resp.owed,
      });
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

  refreshDsnStatus: async () => {
    try {
      const status = await api.getDsnStatus();
      set({ dsnStatus: status });
    } catch {
      // Endpoint unavailable; keep previous value.
    }
  },

  settleDemurrage: async () => {
    // POST /api/tx/settle_demurrage — debits the active account's owed
    // demurrage to the protocol-owned refresh pool. Signs the canonical
    // payload `{type:"settle_demurrage",from,current_epoch}` (matches
    // how the Rust handler reconstructs the message — see
    // api.rs::post_settle_demurrage).
    const { activeAccount, chainStatus } = get();
    if (!activeAccount || !chainStatus) return null;
    set({ loading: true, error: null });
    try {
      const txPayload = JSON.stringify({
        type: "settle_demurrage",
        from: activeAccount.address,
        current_epoch: chainStatus.epoch,
      });
      const { signature, publicKey } = await get().signTransaction(txPayload);
      const resp = await api.settleDemurrage(activeAccount.address, signature, publicKey);
      if (resp.status === "settled") {
        set({
          loading: false,
          notification: `Demurrage settled: ${resp.settled} EVAP → refresh pool`,
          balance: resp.new_balance,
          accountLastTouched: resp.new_last_touched_epoch,
          demurrageOwed: 0,
        });
        // Refresh the refresh-pool view so the DEMU credit shows up.
        get().refreshRefreshPool().catch(() => { /* swallow */ });
        get().refreshBalance().catch(() => { /* swallow */ });
      } else if (resp.status === "nothing_owed") {
        set({
          loading: false,
          notification: "No demurrage owed",
          demurrageOwed: 0,
        });
      } else {
        set({ loading: false, error: resp.detail || "Settle failed" });
      }
      return resp;
    } catch (e: any) {
      set({ loading: false, error: e.message });
      return null;
    }
  },

  refreshBellBeacon: async (req) => {
    // Prefer the new read-only GET /api/bell/latest (api.rs
    // §get_bell_latest). When the chain hasn't persisted a measurement
    // yet (status === "no_data") we fall back to a single
    // worked-example POST /api/bell_beacon roundtrip so the card still
    // renders the design-target S — the BellBeaconCard then shows a
    // "no live measurement yet" badge to make the distinction visible.
    try {
      const latest = await api.getBellBeaconLatest();
      if (latest.status === "ok") {
        set({
          bellSValue: latest.s_value_milli,
          bellThreshold: latest.threshold_milli,
          bellCertified: latest.bell_certified,
        });
        return;
      }
      // no_data | error → fall through to worked-example POST.
    } catch {
      // GET unreachable; fall through.
    }
    const body: BellBeaconReq = req ?? {
      e_ab: 500,
      e_ab_prime: -500,
      e_a_prime_b: 500,
      e_a_prime_b_prime: 500,
    };
    try {
      const resp = await api.getBellBeacon(body);
      set({
        bellSValue: resp.s_value_milli,
        bellThreshold: resp.threshold_milli,
        bellCertified: resp.bell_certified,
      });
    } catch {
      // Endpoint unavailable; keep previous values.
    }
  },

  // ── Sharding ─────────────────────────────────────────────────
  //
  // Pulls /api/shards + /api/shards/health (combined into a
  // ShardsHealth by the api client) and recomputes the active
  // account's shard locally. There is no node endpoint for
  // address→shard, so we mirror `shard_for_object` from
  // crates/evaporchain-sharding/src/shard_assignment.rs L42-L49.
  refreshShards: async () => {
    try {
      const snap = await api.getShardsHealth();
      const { activeAccount } = get();
      let addressShard: number | null = null;
      if (snap.active && activeAccount) {
        const assignment = api.computeShardForAddress(
          activeAccount.address,
          snap.total_shards,
        );
        addressShard = assignment?.shard_id ?? null;
      }
      set({
        shardsHealth: snap,
        shards: snap.shards,
        addressShard,
      });
    } catch {
      // Endpoints unavailable; keep previous values.
    }
  },
}));

// Test-only escape hatch: Playwright e2e specs read the store via
// `globalThis.__zustandStore`. Fenced behind Vite's MODE so it cannot
// leak into production builds (`npm run build` runs as MODE=production).
if (import.meta.env.MODE === "test") { (globalThis as any).__zustandStore = useWallet; }

/**
 * Zustand store for wallet state management.
 * Single source of truth for the extension popup.
 */

import { create } from "zustand";
import { BrowserKeyStore, type KeyEntry, signMessage } from "@/crypto/keystore";
import { api, type AccountDetail, type StateObject, type ChainStatus, type TokenInfo, type SwapResult, type NftItem, type GhostObject, type GhostDetail, type RefreshCostEstimate, type SocialAuthResult } from "@/utils/api";
import type { WcSession, WcSessionProposal } from "@/utils/walletconnect";
import { ledgerManager, type LedgerAccount } from "@/utils/ledger";
import { type BridgeTransfer } from "@/utils/bridge";
import { loadPreferences, savePreferences, type UserPreferences } from "@/utils/preferences";

export type View = "locked" | "create" | "import" | "home" | "send" | "receive" | "objects" | "activity" | "settings" | "backup" | "swap" | "nfts" | "nft-detail" | "buy" | "batch-refresh" | "ghost-recovery" | "energy-dashboard" | "social-login" | "tutorial" | "decay-forecast" | "walletconnect" | "ledger" | "bridge" | "plugins" | "ai-assistant";

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
}

interface TxSendResult {
  success: boolean;
  message: string;
}

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
    set({
      keystore: ks,
      accounts,
      activeAccount: active,
      view: accounts.length === 0 ? "social-login" : "locked",
      nodeUrl: prefs.nodeUrl,
      preferences: prefs,
    });
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
}));

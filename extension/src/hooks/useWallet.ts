/**
 * Zustand store for wallet state management.
 * Single source of truth for the extension popup.
 */

import { create } from "zustand";
import { BrowserKeyStore, type KeyEntry } from "@/crypto/keystore";
import { api, type AccountDetail, type StateObject, type ChainStatus, type TokenInfo, type SwapResult, type NftItem } from "@/utils/api";

export type View = "locked" | "create" | "import" | "home" | "send" | "receive" | "objects" | "activity" | "settings" | "swap" | "nfts" | "nft-detail" | "buy";

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

  // UI
  view: View;
  loading: boolean;
  error: string | null;
  notification: string | null;

  // Network
  nodeUrl: string;

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
  claimFaucet: () => Promise<void>;
  refreshTokens: () => Promise<void>;
  swapTokens: (fromToken: string, toToken: string, amount: number, slippage: number) => Promise<SwapResult>;
  refreshNfts: () => Promise<void>;
  selectNft: (nft: NftItem | null) => void;
  setView: (view: View) => void;
  setError: (error: string | null) => void;
  setNotification: (msg: string | null) => void;
  setNodeUrl: (url: string) => void;
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
  view: "locked",
  loading: false,
  error: null,
  notification: null,
  nodeUrl: "https://testnet.evaporchain.com",

  init: async () => {
    const ks = await BrowserKeyStore.load();
    const accounts = ks.listAccounts();
    const active = ks.getActiveAccount();
    set({
      keystore: ks,
      accounts,
      activeAccount: active,
      view: accounts.length === 0 ? "create" : "locked",
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
    const { activeAccount, nonce } = get();
    if (!activeAccount) throw new Error("No active account");

    set({ loading: true, error: null });
    try {
      const result = await api.transfer(activeAccount.address, to, amount, nonce);
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

  refreshTokens: async () => {
    try {
      const tokens = await api.getTokens();
      set({ tokens });
    } catch {
      // Ignore — tokens endpoint may not be available
    }
  },

  swapTokens: async (fromToken: string, toToken: string, amount: number, slippage: number) => {
    set({ loading: true, error: null });
    try {
      const result = await api.executeSwap(fromToken, toToken, amount, slippage);
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

  setView: (view: View) => set({ view, error: null }),
  setError: (error: string | null) => set({ error }),
  setNotification: (msg: string | null) => set({ notification: msg }),
  setNodeUrl: (url: string) => {
    api.setNode(url);
    set({ nodeUrl: url });
  },
}));

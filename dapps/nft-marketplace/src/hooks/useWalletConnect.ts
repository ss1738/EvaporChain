/**
 * Wallet connection hook — powered by @evaporchain/wallet-sdk.
 *
 * Thin wrapper that maintains the same interface for backward compatibility.
 * All wallet logic now lives in the SDK instead of being reimplemented per-dApp.
 */

import { useEvaporChain } from "@evaporchain/wallet-sdk/react";

interface WalletState {
  connected: boolean;
  address: string | null;
  connecting: boolean;
  error: string | null;
  connect: () => Promise<void>;
  disconnect: () => void;
}

export function useWalletConnect(): WalletState {
  const { address, connected, connecting, error, connect, disconnect } = useEvaporChain();

  return {
    connected,
    address,
    connecting,
    error,
    connect: async () => { await connect(); },
    disconnect: () => { disconnect(); },
  };
}

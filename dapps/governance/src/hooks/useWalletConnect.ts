/**
 * Wallet connection hook for EvaporChain governance dApp.
 * Powered by @evaporchain/wallet-sdk.
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

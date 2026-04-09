/**
 * Wallet connection hook — powered by @evaporchain/wallet-sdk.
 *
 * Thin wrapper that maintains the same interface for backward compatibility.
 * All wallet logic now lives in the SDK instead of being reimplemented per-dApp.
 */

import { useEvaporChain } from "@evaporchain/wallet-sdk/react";

export function useWalletConnect() {
  const { address, connected, connecting, connect, disconnect } = useEvaporChain();

  return {
    address,
    connected,
    loading: connecting,
    connect: async () => { await connect(); },
    disconnect: async () => { await disconnect(); },
  };
}

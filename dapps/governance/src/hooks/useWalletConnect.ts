/**
 * Wallet connection hook for EvaporChain governance dApp.
 *
 * Uses window.evaporchain provider injected by the browser extension.
 * When @evaporchain/wallet-sdk is published to npm, migrate to:
 *   import { useEvaporChain } from "@evaporchain/wallet-sdk/react";
 */

import { useState, useEffect, useCallback } from "react";

interface WalletState {
  connected: boolean;
  address: string | null;
  connecting: boolean;
  error: string | null;
  connect: () => Promise<void>;
  disconnect: () => void;
}

export function useWalletConnect(): WalletState {
  const [connected, setConnected] = useState(false);
  const [address, setAddress] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const check = async () => {
      const provider = (window as any).evaporchain;
      if (provider?.isConnected) {
        try {
          const accounts = await provider.getAccounts();
          if (accounts.length > 0) {
            setAddress(accounts[0]);
            setConnected(true);
          }
        } catch {
          // not connected
        }
      }
    };

    if ((window as any).evaporchain) {
      check();
    } else {
      const handler = () => check();
      window.addEventListener("evaporchain#initialized", handler);
      return () => window.removeEventListener("evaporchain#initialized", handler);
    }
  }, []);

  const connect = useCallback(async () => {
    setConnecting(true);
    setError(null);

    const provider = (window as any).evaporchain;
    if (!provider) {
      setError("EvaporChain Wallet extension not found. Please install it first.");
      setConnecting(false);
      return;
    }

    try {
      const accounts = await provider.connect();
      if (accounts.length > 0) {
        setAddress(accounts[0]);
        setConnected(true);
      }
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : "Connection failed";
      setError(message);
    } finally {
      setConnecting(false);
    }
  }, []);

  const disconnect = useCallback(() => {
    (window as any).evaporchain?.disconnect();
    setConnected(false);
    setAddress(null);
  }, []);

  return { connected, address, connecting, error, connect, disconnect };
}

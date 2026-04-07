/**
 * Hook to connect to the EvaporChain browser extension wallet.
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

declare global {
  interface Window {
    evaporchain?: {
      isEvaporChain: true;
      isConnected: boolean;
      connect(): Promise<string[]>;
      disconnect(): Promise<void>;
      getAccounts(): Promise<string[]>;
      sendTransaction(params: { to: string; amount: number }): Promise<{ success: boolean; message: string }>;
      signMessage(message: string): Promise<string>;
      on(event: string, cb: (...args: unknown[]) => void): void;
      off(event: string, cb: (...args: unknown[]) => void): void;
    };
  }
}

export function useWalletConnect(): WalletState {
  const [connected, setConnected] = useState(false);
  const [address, setAddress] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Check for existing connection on mount
  useEffect(() => {
    const check = async () => {
      if (window.evaporchain?.isConnected) {
        try {
          const accounts = await window.evaporchain.getAccounts();
          if (accounts.length > 0) {
            setAddress(accounts[0]);
            setConnected(true);
          }
        } catch {
          // Not connected
        }
      }
    };

    if (window.evaporchain) {
      check();
    } else {
      // Wait for provider injection
      const handler = () => check();
      window.addEventListener("evaporchain#initialized", handler);
      return () => window.removeEventListener("evaporchain#initialized", handler);
    }
  }, []);

  const connect = useCallback(async () => {
    setConnecting(true);
    setError(null);

    if (!window.evaporchain) {
      setError("EvaporChain Wallet extension not found. Please install it first.");
      setConnecting(false);
      return;
    }

    try {
      const accounts = await window.evaporchain.connect();
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
    window.evaporchain?.disconnect();
    setConnected(false);
    setAddress(null);
  }, []);

  return { connected, address, connecting, error, connect, disconnect };
}

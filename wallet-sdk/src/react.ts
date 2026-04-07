/**
 * React hooks for the EvaporChain wallet SDK.
 *
 * Optional import — only use in React projects:
 *   import { useEvaporChain, useObjects, useNfts } from "@evaporchain/wallet-sdk/react";
 */

import { useState, useEffect, useCallback, useRef } from "react";
import { EvaporChainProvider } from "./provider";
import type {
  Balance,
  EvaporObject,
  Nft,
  ChainStatus,
  ConnectResult,
} from "./types";

// Singleton provider — shared across all hook instances
let _sharedProvider: EvaporChainProvider | null = null;

function getProvider(): EvaporChainProvider {
  if (!_sharedProvider) {
    _sharedProvider = new EvaporChainProvider();
  }
  return _sharedProvider;
}

// ── useEvaporChain ──

export interface UseEvaporChainResult {
  provider: EvaporChainProvider;
  address: string | null;
  balance: number | null;
  connected: boolean;
  connecting: boolean;
  error: string | null;
  connect: () => Promise<ConnectResult | undefined>;
  disconnect: () => Promise<void>;
}

/**
 * Core hook for connecting to the EvaporChain wallet.
 *
 * @example
 * ```tsx
 * function App() {
 *   const { address, balance, connected, connect, disconnect } = useEvaporChain();
 *
 *   if (!connected) {
 *     return <button onClick={connect}>Connect Wallet</button>;
 *   }
 *
 *   return (
 *     <div>
 *       <p>Address: {address}</p>
 *       <p>Balance: {balance} EVAP</p>
 *       <button onClick={disconnect}>Disconnect</button>
 *     </div>
 *   );
 * }
 * ```
 */
export function useEvaporChain(): UseEvaporChainResult {
  const provider = getProvider();
  const [address, setAddress] = useState<string | null>(provider.address);
  const [balance, setBalance] = useState<number | null>(null);
  const [connected, setConnected] = useState(provider.connected);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Sync state when provider fires events
  useEffect(() => {
    const onConnect = (result: unknown) => {
      const r = result as ConnectResult;
      setAddress(r.address);
      setConnected(true);
      setError(null);
    };

    const onDisconnect = () => {
      setAddress(null);
      setBalance(null);
      setConnected(false);
    };

    const onAccountsChanged = (accounts: unknown) => {
      const accs = accounts as string[];
      if (accs.length > 0) {
        setAddress(accs[0]);
      } else {
        setAddress(null);
        setConnected(false);
      }
    };

    provider.on("connect", onConnect);
    provider.on("disconnect", onDisconnect);
    provider.on("accountsChanged", onAccountsChanged);

    return () => {
      provider.off("connect", onConnect);
      provider.off("disconnect", onDisconnect);
      provider.off("accountsChanged", onAccountsChanged);
    };
  }, [provider]);

  // Fetch balance when address changes
  useEffect(() => {
    if (!address || !connected) {
      setBalance(null);
      return;
    }

    let cancelled = false;
    provider.getBalance(address).then((b: Balance) => {
      if (!cancelled) setBalance(b.balance);
    }).catch(() => {
      // Ignore — balance will be null
    });

    return () => { cancelled = true; };
  }, [address, connected, provider]);

  const connect = useCallback(async () => {
    setConnecting(true);
    setError(null);
    try {
      const result = await provider.connect();
      return result;
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      return undefined;
    } finally {
      setConnecting(false);
    }
  }, [provider]);

  const disconnect = useCallback(async () => {
    await provider.disconnect();
  }, [provider]);

  return {
    provider,
    address,
    balance,
    connected,
    connecting,
    error,
    connect,
    disconnect,
  };
}

// ── useObjects ──

export interface UseObjectsResult {
  objects: EvaporObject[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Hook for fetching decaying state objects owned by an address.
 *
 * @param address - Address to query. Defaults to the connected account.
 *
 * @example
 * ```tsx
 * function ObjectList() {
 *   const { objects, loading, refresh } = useObjects();
 *
 *   return (
 *     <ul>
 *       {objects.map(obj => (
 *         <li key={obj.id}>{obj.name}: {obj.currentEnergy}/{obj.maxEnergy}</li>
 *       ))}
 *     </ul>
 *   );
 * }
 * ```
 */
export function useObjects(address?: string): UseObjectsResult {
  const provider = getProvider();
  const [objects, setObjects] = useState<EvaporObject[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await provider.getObjects(address);
      if (mountedRef.current) setObjects(result);
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [provider, address]);

  useEffect(() => {
    mountedRef.current = true;
    if (provider.connected) {
      refresh();
    }
    return () => { mountedRef.current = false; };
  }, [provider.connected, refresh]);

  return { objects, loading, error, refresh };
}

// ── useNfts ──

export interface UseNftsResult {
  nfts: Nft[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Hook for fetching NFTs owned by an address.
 *
 * @param address - Address to query. Defaults to the connected account.
 *
 * @example
 * ```tsx
 * function NftGallery() {
 *   const { nfts, loading } = useNfts();
 *   if (loading) return <p>Loading...</p>;
 *   return nfts.map(nft => <NftCard key={nft.id} nft={nft} />);
 * }
 * ```
 */
export function useNfts(address?: string): UseNftsResult {
  const provider = getProvider();
  const [nfts, setNfts] = useState<Nft[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await provider.getNfts(address);
      if (mountedRef.current) setNfts(result);
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [provider, address]);

  useEffect(() => {
    mountedRef.current = true;
    if (provider.connected) {
      refresh();
    }
    return () => { mountedRef.current = false; };
  }, [provider.connected, refresh]);

  return { nfts, loading, error, refresh };
}

// ── useChainStatus ──

export interface UseChainStatusResult {
  blockHeight: number | null;
  epoch: number | null;
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Hook for fetching the current chain status.
 * Auto-polls every 10 seconds while mounted.
 *
 * @example
 * ```tsx
 * function StatusBar() {
 *   const { blockHeight, epoch } = useChainStatus();
 *   return <p>Block {blockHeight} | Epoch {epoch}</p>;
 * }
 * ```
 */
export function useChainStatus(): UseChainStatusResult {
  const provider = getProvider();
  const [status, setStatus] = useState<ChainStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await provider.getChainStatus();
      if (mountedRef.current) setStatus(result);
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [provider]);

  useEffect(() => {
    mountedRef.current = true;
    refresh();

    const interval = setInterval(refresh, 10_000);
    return () => {
      mountedRef.current = false;
      clearInterval(interval);
    };
  }, [refresh]);

  return {
    blockHeight: status?.blockHeight ?? null,
    epoch: status?.epoch ?? null,
    loading,
    error,
    refresh,
  };
}

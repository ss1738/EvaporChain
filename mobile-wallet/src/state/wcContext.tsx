/**
 * WalletConnectContext — single shared `WalletConnectManager` for the
 * whole mobile wallet, plus an `initError` so the screen can render a
 * clear "Install WC v2 deps" message when the SDK isn't resolvable yet.
 *
 * Mounted in `App.tsx` so the manager's lifetime equals the app's,
 * matching the extension's singleton-at-module-scope pattern. Consumers
 * (currently just `WalletConnectScreen`) read it via `useWalletConnect()`.
 */

import React, {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { WalletConnectManager } from '../utils/walletconnect';

interface WcContextValue {
  manager: WalletConnectManager;
  initialized: boolean;
  initError: string | null;
}

const WcContext = createContext<WcContextValue | null>(null);

export const WalletConnectProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const managerRef = useRef<WalletConnectManager | null>(null);
  if (managerRef.current === null) {
    managerRef.current = new WalletConnectManager();
  }
  const manager = managerRef.current;

  const [initialized, setInitialized] = useState(false);
  const [initError, setInitError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    manager
      .init()
      .then(() => {
        if (!cancelled) {
          setInitialized(true);
          setInitError(null);
        }
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        const msg = e instanceof Error ? e.message : String(e);
        setInitError(msg);
        // eslint-disable-next-line no-console
        console.warn('[walletconnect] init failed:', msg);
      });
    return () => {
      cancelled = true;
    };
  }, [manager]);

  const value = useMemo<WcContextValue>(
    () => ({ manager, initialized, initError }),
    [manager, initialized, initError],
  );

  return <WcContext.Provider value={value}>{children}</WcContext.Provider>;
};

export function useWalletConnect(): WcContextValue {
  const ctx = useContext(WcContext);
  if (!ctx) {
    throw new Error('useWalletConnect must be used inside <WalletConnectProvider>');
  }
  return ctx;
}

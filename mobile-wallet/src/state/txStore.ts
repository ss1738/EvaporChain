/**
 * Tx-tracking + toast queue for the mobile wallet.
 *
 * Lifted from the browser extension (extension/src/hooks/useWallet.ts
 * `pendingTxs` / `toasts` slices). Mirrors the same state machine —
 * pending → mempool → included → finalised | rejected — with a 10s
 * grace window after terminal state before the row is auto-cleared.
 *
 * Implementation: React Context + useReducer (no zustand on mobile).
 * The extension uses zustand; for mobile we kept the dependency
 * footprint small. If the user runs `npm install zustand` later, this
 * file can be replaced with a `create()` store and the consumer
 * surface (the `use*` hooks below) stays identical so callers don't
 * have to change.
 *
 * The polling lives in `utils/txTracker.ts` and pokes this store via
 * the imperative handle returned from `useTxStoreImperative`.
 */

import React, { createContext, useCallback, useContext, useMemo, useReducer, useRef } from 'react';
import type { TxStatus } from '../utils/api';

export type PendingTxKind =
  | 'transfer'
  | 'swap'
  | 'resurrect'
  | 'batch_refresh'
  | 'settle_demurrage';

export interface PendingTx {
  hash: string;
  kind: PendingTxKind;
  summary: string;
  submittedAt: number;
  status: TxStatus['state'];
  blockHeight?: number;
  error?: string;
  /** Filled when the tx finalises so we can clear it after a 10s grace. */
  finalisedAt?: number;
}

export interface Toast {
  id: string;
  kind: 'finalised' | 'rejected';
  /** Short summary copied from the finalising PendingTx. */
  summary: string;
  /** Tx hash for the click-to-copy chip. */
  hash: string;
  createdAt: number;
  /**
   * Executor error string for rejected toasts — mirrors the extension
   * Toast.error and ultimately TxRecord.error / TxStatus.error from
   * the node. Rendered in red beneath the summary on the rejected
   * variant.
   */
  error?: string;
}

interface State {
  pendingTxs: PendingTx[];
  toasts: Toast[];
}

type Action =
  | { type: 'TRACK'; hash: string; kind: PendingTxKind; summary: string }
  | { type: 'CLEAR'; hash: string }
  | { type: 'UPDATE_STATUSES'; updates: PendingTx[] }
  | { type: 'PUSH_TOAST'; toast: Omit<Toast, 'id' | 'createdAt'> }
  | { type: 'DISMISS_TOAST'; id: string };

const initialState: State = { pendingTxs: [], toasts: [] };

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case 'TRACK': {
      if (state.pendingTxs.find((t) => t.hash === action.hash)) return state;
      return {
        ...state,
        pendingTxs: [
          ...state.pendingTxs,
          {
            hash: action.hash,
            kind: action.kind,
            summary: action.summary,
            submittedAt: Date.now(),
            status: 'pending',
          },
        ],
      };
    }
    case 'CLEAR':
      return {
        ...state,
        pendingTxs: state.pendingTxs.filter((t) => t.hash !== action.hash),
      };
    case 'UPDATE_STATUSES':
      return { ...state, pendingTxs: action.updates };
    case 'PUSH_TOAST': {
      const id = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      return {
        ...state,
        toasts: [
          ...state.toasts,
          { id, createdAt: Date.now(), ...action.toast },
        ],
      };
    }
    case 'DISMISS_TOAST':
      return { ...state, toasts: state.toasts.filter((t) => t.id !== action.id) };
  }
}

interface TxStoreContextValue {
  state: State;
  trackTx: (hash: string, kind: PendingTxKind, summary: string) => void;
  clearTx: (hash: string) => void;
  applyStatuses: (updates: PendingTx[]) => void;
  pushToast: (toast: Omit<Toast, 'id' | 'createdAt'>) => void;
  dismissToast: (id: string) => void;
}

const TxStoreContext = createContext<TxStoreContextValue | null>(null);

/**
 * Imperative handle so `txTracker.ts` (a non-React singleton) can read
 * and write the store from its setInterval callback. Populated on mount
 * by `TxStoreProvider`.
 */
export interface TxStoreHandle {
  getState: () => State;
  applyStatuses: (updates: PendingTx[]) => void;
  pushToast: (toast: Omit<Toast, 'id' | 'createdAt'>) => void;
}

const handleRef: { current: TxStoreHandle | null } = { current: null };

export function getTxStoreHandle(): TxStoreHandle | null {
  return handleRef.current;
}

export const TxStoreProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [state, dispatch] = useReducer(reducer, initialState);
  // Keep a ref to current state so the imperative handle's getState()
  // doesn't capture a stale snapshot when the tracker queries it
  // mid-poll. React reducers update synchronously enough for our 3s
  // tick, but the ref is the safe path.
  const stateRef = useRef(state);
  stateRef.current = state;

  const trackTx = useCallback(
    (hash: string, kind: PendingTxKind, summary: string) => {
      dispatch({ type: 'TRACK', hash, kind, summary });
    },
    [],
  );
  const clearTx = useCallback((hash: string) => {
    dispatch({ type: 'CLEAR', hash });
  }, []);
  const applyStatuses = useCallback((updates: PendingTx[]) => {
    dispatch({ type: 'UPDATE_STATUSES', updates });
  }, []);
  const pushToast = useCallback((toast: Omit<Toast, 'id' | 'createdAt'>) => {
    dispatch({ type: 'PUSH_TOAST', toast });
  }, []);
  const dismissToast = useCallback((id: string) => {
    dispatch({ type: 'DISMISS_TOAST', id });
  }, []);

  // Wire the imperative handle once. The closures read from stateRef so
  // they always see the latest state.
  handleRef.current = {
    getState: () => stateRef.current,
    applyStatuses: (updates) => dispatch({ type: 'UPDATE_STATUSES', updates }),
    pushToast: (toast) => dispatch({ type: 'PUSH_TOAST', toast }),
  };

  const value = useMemo<TxStoreContextValue>(
    () => ({ state, trackTx, clearTx, applyStatuses, pushToast, dismissToast }),
    [state, trackTx, clearTx, applyStatuses, pushToast, dismissToast],
  );

  return React.createElement(TxStoreContext.Provider, { value }, children);
};

/** Hook for components that need both pendingTxs and the action API. */
export function useTxStore(): TxStoreContextValue {
  const ctx = useContext(TxStoreContext);
  if (!ctx) {
    throw new Error('useTxStore must be used inside TxStoreProvider');
  }
  return ctx;
}

/** Convenience hook returning just the pendingTxs slice. */
export function usePendingTxs(): PendingTx[] {
  return useTxStore().state.pendingTxs;
}

/** Convenience hook returning just the toast queue. */
export function useToasts(): Toast[] {
  return useTxStore().state.toasts;
}

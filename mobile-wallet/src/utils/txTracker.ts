/**
 * TxTracker — 3-second polling loop for pending transactions.
 *
 * Mirrors the extension's `pollTxStatuses` (extension/src/hooks/
 * useWallet.ts). The loop is active only while at least one tx is
 * pending; the tick body short-circuits on an empty pending list so
 * the interval is effectively idle until `trackTx` registers a hash.
 *
 * Lifecycle:
 *   - Started once at app mount via `txTracker.start()` in App.tsx.
 *   - Foreground/background: AppState listener pauses the interval
 *     when the app backgrounds and resumes on foreground. We don't
 *     want background ticks burning data, but we also don't want to
 *     drop pending txs — they survive in the store.
 *
 * Toast firing:
 *   - On state transition into `finalised` or `rejected`, push a toast
 *     to the store. Mirrors extension TxToast lifecycle (4s auto-dismiss
 *     handled by the renderer).
 *
 * Implementation note:
 *   We can't import the React Context inside a singleton, so we read /
 *   write through `getTxStoreHandle()` — populated by TxStoreProvider
 *   when the React tree mounts. If the handle isn't ready (boot race),
 *   the tick is a no-op.
 */

import { AppState, type AppStateStatus } from 'react-native';
import { api } from './api';
import { getTxStoreHandle, type PendingTx } from '../state/txStore';

const POLL_INTERVAL_MS = 3000;
/** Grace period between a tx going terminal and being auto-cleared. */
const TERMINAL_GRACE_MS = 10_000;

class TxTracker {
  private timer: ReturnType<typeof setInterval> | null = null;
  private appStateListener: ReturnType<typeof AppState.addEventListener> | null = null;
  private foregrounded = true;

  start(): void {
    if (this.timer != null) return;
    this.foregrounded = AppState.currentState === 'active';
    this.appStateListener = AppState.addEventListener('change', this.handleAppStateChange);
    this.timer = setInterval(() => {
      if (!this.foregrounded) return;
      const handle = getTxStoreHandle();
      if (!handle) return;
      if (handle.getState().pendingTxs.length === 0) return;
      void this.tick();
    }, POLL_INTERVAL_MS);
  }

  stop(): void {
    if (this.timer != null) {
      clearInterval(this.timer);
      this.timer = null;
    }
    this.appStateListener?.remove();
    this.appStateListener = null;
  }

  private handleAppStateChange = (nextState: AppStateStatus): void => {
    this.foregrounded = nextState === 'active';
  };

  private async tick(): Promise<void> {
    const handle = getTxStoreHandle();
    if (!handle) return;

    const txs = handle.getState().pendingTxs;
    if (txs.length === 0) return;

    const now = Date.now();
    // Drop anything that finalised more than TERMINAL_GRACE_MS ago.
    const live = txs.filter(
      (t) => !t.finalisedAt || now - t.finalisedAt < TERMINAL_GRACE_MS,
    );

    const newlyTerminal: { tx: PendingTx; kind: 'finalised' | 'rejected' }[] = [];
    const updated = await Promise.all(
      live.map(async (tx) => {
        if (tx.status === 'finalised' || tx.status === 'rejected') return tx;
        try {
          const status = await api.getTxStatus(tx.hash);
          if (status.state === tx.status) return tx;
          const next: PendingTx = {
            ...tx,
            status: status.state,
            blockHeight: status.block_height ?? tx.blockHeight,
            error: status.error,
          };
          if (status.state === 'finalised' || status.state === 'rejected') {
            next.finalisedAt = Date.now();
            newlyTerminal.push({ tx: next, kind: status.state });
          }
          return next;
        } catch {
          // Tolerate transient API errors — keep the row.
          return tx;
        }
      }),
    );

    handle.applyStatuses(updated);

    for (const { tx, kind } of newlyTerminal) {
      handle.pushToast({
        kind,
        summary: tx.summary,
        hash: tx.hash,
        // Forward executor error string so rejected toasts can render
        // the failure reason (e.g. "InsufficientBalance") beneath the
        // summary. Always undefined for finalised toasts.
        error: kind === 'rejected' ? tx.error : undefined,
      });
    }
  }
}

export const txTracker = new TxTracker();

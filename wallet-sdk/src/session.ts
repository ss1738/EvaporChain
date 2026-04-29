/**
 * Session management for EvaporChain dApps.
 *
 * A session records pre-approved constraints for a connected dApp:
 *   - TTL (default 30 min)
 *   - energy spend cap
 *   - allowed transaction types
 *
 * Session data is stored in `sessionStorage` (cleared when the tab closes).
 * The session does NOT hold private keys — signing is always routed through
 * the injected wallet extension. The session controls *which* transactions
 * can be auto-approved without an explicit popup.
 *
 * Usage:
 *   const mgr = new SessionManager();
 *   await mgr.create({ expiryMs: 10 * 60 * 1000, energyLimit: 500 });
 *   // ... later, before submitting a tx:
 *   if (mgr.canApprove("transfer", 100)) { ... skip popup ... }
 */

import type { SessionOptions, SessionInfo, OfflineTxType } from "./types";

const STORAGE_KEY = "evaporchain_session_v1";
const DEFAULT_EXPIRY_MS = 30 * 60 * 1000; // 30 minutes

function generateSessionId(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

export class SessionManager {
  private _storage: Storage;

  constructor(storage?: Storage) {
    this._storage =
      storage ??
      (typeof sessionStorage !== "undefined"
        ? sessionStorage
        : // Node/test fallback: in-memory map
          (() => {
            const m = new Map<string, string>();
            return {
              getItem: (k: string) => m.get(k) ?? null,
              setItem: (k: string, v: string) => m.set(k, v),
              removeItem: (k: string) => m.delete(k),
              clear: () => m.clear(),
              key: () => null,
              get length() {
                return m.size;
              },
            } as Storage;
          })());
  }

  /** Create a new session with the given options. Overwrites any prior session. */
  create(options?: SessionOptions): SessionInfo {
    const now = Date.now();
    const session: SessionInfo = {
      sessionId: generateSessionId(),
      createdAt: now,
      expiresAt: now + (options?.expiryMs ?? DEFAULT_EXPIRY_MS),
      energyLimit: options?.energyLimit,
      energyUsed: 0,
      allowedTxTypes: options?.allowedTxTypes,
    };
    this._storage.setItem(STORAGE_KEY, JSON.stringify(session));
    return session;
  }

  /** Return the active session, or null if expired or not found. */
  get(): SessionInfo | null {
    const raw = this._storage.getItem(STORAGE_KEY);
    if (!raw) return null;
    try {
      const session = JSON.parse(raw) as SessionInfo;
      if (Date.now() > session.expiresAt) {
        this._storage.removeItem(STORAGE_KEY);
        return null;
      }
      return session;
    } catch {
      return null;
    }
  }

  /** Clear the active session immediately. */
  clear(): void {
    this._storage.removeItem(STORAGE_KEY);
  }

  /** Renew the session expiry without changing other settings. */
  renew(expiryMs?: number): SessionInfo | null {
    const session = this.get();
    if (!session) return null;
    session.expiresAt = Date.now() + (expiryMs ?? DEFAULT_EXPIRY_MS);
    this._storage.setItem(STORAGE_KEY, JSON.stringify(session));
    return session;
  }

  /**
   * Check whether a transaction of `txType` spending `energy` units can be
   * auto-approved by this session (no wallet popup required).
   *
   * Returns false if:
   *   - no active session
   *   - the tx type is not in `allowedTxTypes` (if that list is set)
   *   - the energy spend would exceed `energyLimit`
   */
  canApprove(txType: OfflineTxType, energy = 0): boolean {
    const session = this.get();
    if (!session) return false;

    if (
      session.allowedTxTypes !== undefined &&
      session.allowedTxTypes.length > 0 &&
      !session.allowedTxTypes.includes(txType)
    ) {
      return false;
    }

    if (session.energyLimit !== undefined) {
      if (session.energyUsed + energy > session.energyLimit) return false;
    }

    return true;
  }

  /**
   * Record energy spent against the session budget.
   * Call this after a session-approved transaction is confirmed.
   * Returns false if the spend would exceed the budget (and does not record it).
   */
  recordEnergySpend(energy: number): boolean {
    const session = this.get();
    if (!session) return false;

    if (
      session.energyLimit !== undefined &&
      session.energyUsed + energy > session.energyLimit
    ) {
      return false;
    }

    session.energyUsed += energy;
    this._storage.setItem(STORAGE_KEY, JSON.stringify(session));
    return true;
  }

  /** Remaining energy budget for this session (undefined if no limit set). */
  remainingEnergy(): number | undefined {
    const session = this.get();
    if (!session) return 0;
    if (session.energyLimit === undefined) return undefined;
    return Math.max(0, session.energyLimit - session.energyUsed);
  }

  /** Milliseconds until the session expires (0 if already expired). */
  timeToExpiry(): number {
    const session = this.get();
    if (!session) return 0;
    return Math.max(0, session.expiresAt - Date.now());
  }
}

// Shared auth layer for the EvaporChain dApp set.
//
// The wallet dApp (`dapps/wallet/`) handles the /api/auth/{register,
// verify-email, login} flow and writes the session token to
// localStorage under these keys. Every other reference-contract dApp
// reads from those keys + injects the Authorization header on
// /api/tx/* writes.
//
// Why localStorage and not a shared in-memory module:
//   1. Each dApp is a separate page (separate JS module graph). They
//      can't share runtime state.
//   2. The browser's localStorage is the natural cross-page state
//      store within the same origin.
//   3. When the wallet logs out, every other tab on the same origin
//      sees the token cleared on its next read — no propagation
//      protocol needed.
//
// Storage shape (mirrors `dapps/wallet/index.html`):
//   evaporchain_wallet_node          → string  (e.g. "http://89.167.52.40:8099")
//   evaporchain_wallet_token         → string  (24h TTL per auth.rs)
//   evaporchain_wallet_email         → string
//   evaporchain_wallet_user_id       → string  (numeric id, stringified)
//   evaporchain_wallet_display_name  → string
//
// `localStorage` is a browser global. For node:test runs (no DOM),
// we polyfill with a tiny in-memory shim so unit tests can drive
// the helpers without a JSDOM dep.

declare const localStorage:
  | { getItem(k: string): string | null; setItem(k: string, v: string): void; removeItem(k: string): void }
  | undefined;

const MEMORY_STORE: Record<string, string> = {};

function store() {
  if (typeof localStorage !== "undefined") return localStorage;
  return {
    getItem: (k: string) => (k in MEMORY_STORE ? MEMORY_STORE[k] : null),
    setItem: (k: string, v: string) => { MEMORY_STORE[k] = v; },
    removeItem: (k: string) => { delete MEMORY_STORE[k]; },
  };
}

export const LS_KEYS = {
  node:    "evaporchain_wallet_node",
  token:   "evaporchain_wallet_token",
  email:   "evaporchain_wallet_email",
  userId:  "evaporchain_wallet_user_id",
  display: "evaporchain_wallet_display_name",
} as const;

export const DEFAULT_NODE = "http://89.167.52.40:8099";

/** Read the saved session token. Returns null if absent or empty. */
export function getToken(): string | null {
  const t = store().getItem(LS_KEYS.token);
  return t && t.length > 0 ? t : null;
}

/** Read the saved node URL, falling back to the public devnet. */
export function getNode(): string {
  const n = store().getItem(LS_KEYS.node);
  return (n && n.length > 0 ? n : DEFAULT_NODE).replace(/\/$/, "");
}

/** True iff a token is present in storage. Use to gate UI on
 *  "is the user logged in" without committing to a network probe. */
export function isAuthed(): boolean {
  return getToken() !== null;
}

/** Best-effort read of the saved profile metadata. Fields are null
 *  when absent. Use for "logged in as X" UI without making a
 *  network call. */
export function getProfile(): {
  email: string | null;
  userId: number | null;
  displayName: string | null;
} {
  const s = store();
  const email = s.getItem(LS_KEYS.email);
  const userIdRaw = s.getItem(LS_KEYS.userId);
  const displayName = s.getItem(LS_KEYS.display);
  const userId = userIdRaw ? Number.parseInt(userIdRaw, 10) : NaN;
  return {
    email: email && email.length > 0 ? email : null,
    userId: Number.isFinite(userId) ? userId : null,
    displayName: displayName && displayName.length > 0 ? displayName : null,
  };
}

/** Set the auth state from a successful /api/auth/login response.
 *  Called by the wallet; other dApps wouldn't normally use this. */
export function setSession(opts: {
  node: string;
  token: string;
  email: string;
  userId: number;
  displayName: string;
}): void {
  const s = store();
  s.setItem(LS_KEYS.node, opts.node);
  s.setItem(LS_KEYS.token, opts.token);
  s.setItem(LS_KEYS.email, opts.email);
  s.setItem(LS_KEYS.userId, opts.userId.toString());
  s.setItem(LS_KEYS.display, opts.displayName);
}

/** Clear the session (token + profile). Called by the wallet's
 *  logout button. */
export function clearSession(): void {
  const s = store();
  [LS_KEYS.token, LS_KEYS.email, LS_KEYS.userId, LS_KEYS.display].forEach((k) =>
    s.removeItem(k),
  );
}

/** Generic tx-shaped response. The 12 reference-contract clients
 *  used to define this locally — they now import it from here. */
export interface TxResponse {
  success: boolean;
  tx_hash?: string;
  message: string;
}

/** POST `baseUrl + path` with the auth token from localStorage (if
 *  present) and the given JSON body. The baseUrl arg matches the
 *  shape the existing client.ts files expect — most callers pass
 *  `getNode()` here, but it's left explicit so an integration test
 *  can override.
 *
 *  Throws if the response isn't JSON-parseable; returns the parsed
 *  object otherwise (whether the chain accepted the tx or not — the
 *  caller inspects `success`/`message`). */
export async function authedPost(baseUrl: string, path: string, body: unknown): Promise<TxResponse> {
  const headers: Record<string, string> = { "content-type": "application/json" };
  const token = getToken();
  if (token) headers["authorization"] = `Bearer ${token}`;
  const res = await fetch(`${baseUrl}${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  return (await res.json()) as TxResponse;
}

/** GET `baseUrl + path` with the auth token (if present). Used by
 *  the wallet's /api/auth/me probe + any future read endpoint that
 *  the node requires auth for. */
export async function authedGet<T = unknown>(baseUrl: string, path: string): Promise<T> {
  const headers: Record<string, string> = {};
  const token = getToken();
  if (token) headers["authorization"] = `Bearer ${token}`;
  const res = await fetch(`${baseUrl}${path}`, { headers });
  return (await res.json()) as T;
}

/** Reset the memory shim — exported for tests only. In a real
 *  browser there's a real localStorage; this is a no-op. */
export function __resetForTests(): void {
  if (typeof localStorage === "undefined") {
    for (const k of Object.keys(MEMORY_STORE)) delete MEMORY_STORE[k];
  }
}

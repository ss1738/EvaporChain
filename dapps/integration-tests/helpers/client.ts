/**
 * Shared HTTP client for EvaporChain integration tests.
 * Targets the live testnet node.
 */

const BASE_URL = process.env.EVAPORCHAIN_URL ?? "https://testnet.evaporchain.com";

export interface ApiResponse<T = unknown> {
  status: number;
  ok: boolean;
  data: T;
  headers: Headers;
}

export async function get<T = unknown>(path: string): Promise<ApiResponse<T>> {
  const res = await fetch(`${BASE_URL}${path}`, {
    method: "GET",
    headers: { Accept: "application/json" },
  });
  const data = await res.json().catch(() => null);
  return { status: res.status, ok: res.ok, data: data as T, headers: res.headers };
}

export async function post<T = unknown>(path: string, body?: unknown): Promise<ApiResponse<T>> {
  const res = await fetch(`${BASE_URL}${path}`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Accept: "application/json",
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => null);
  return { status: res.status, ok: res.ok, data: data as T, headers: res.headers };
}

/** Generate a random hex address for testing */
export function randomAddress(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return "0x" + Array.from(bytes).map(b => b.toString(16).padStart(2, "0")).join("");
}

/** Generate a unique name with prefix */
export function uniqueName(prefix: string): string {
  return `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

/** Wait for a given number of milliseconds */
export function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

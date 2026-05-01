/**
 * Explorer-Light API client.
 *
 * Wraps the EvaporChain wallet-sdk for the calls it exposes (chain status,
 * balance, transactions) and uses direct fetch for the *light-client* endpoints
 * that aren't surfaced through the SDK yet:
 *
 *   GET /api/light/headers              — compact header range
 *   GET /api/light/tx-proof/:b/:i       — Merkle inclusion proof
 *   GET /api/light/state-proof/account  — Verkle state proof for accounts
 *   GET /api/light/state-proof/object   — Verkle state proof for objects
 *   GET /api/light/client/status        — trusted-header watermark
 *   GET /api/blocks/:height             — full block (heavy)
 *   GET /api/tx/:hash                   — tx-status enriched response
 *   GET /api/transactions               — recent tx feed
 *   GET /api/address/:addr              — address detail (balance, nonce, objects)
 *   GET /api/shards                     — shard bridge status
 *
 * Anything we route through wallet-sdk arrives camelCased; raw fetches stay
 * snake_case (matching the wire format) and are typed against the chain.
 */

import { EvaporChainAPI } from "@evaporchain/wallet-sdk";

let _client: EvaporChainAPI | null = null;

export function getClient(): EvaporChainAPI {
  if (!_client) {
    _client = new EvaporChainAPI({
      rpcUrl:
        typeof window !== "undefined" ? window.location.origin : "https://testnet.evaporchain.com",
    });
  }
  return _client;
}

// ── Wire-format types (snake_case as returned by /api/light/*) ──

export interface CompactHeader {
  number: number;
  epoch: number;
  parent_hash: string;
  state_root: string;
  tx_count: number;
  tx_merkle_root: string;
  timestamp: number;
  has_nova_proof: boolean;
}

export interface HeadersResponse {
  count: number;
  from: number;
  to: number;
  headers: CompactHeader[];
}

export interface TxMerkleSibling {
  hash: string;
  position: "left" | "right";
}

export interface TxInclusionProof {
  tx_hash: string;
  tx_index: number;
  block_number: number;
  merkle_root: string;
  siblings: TxMerkleSibling[];
}

export interface VerkleProof {
  key: string;
  value: string | null;
  depth: number;
  commitments: string[];
  path_indices: number[];
  siblings: Array<Array<{ index: number; hash: string }>>;
  hit_compressed: boolean;
}

export interface StateProofResponse {
  type: "account" | "object";
  address?: string;
  object_id?: string;
  state_root: string;
  exists: boolean;
  account?: { balance: number; nonce: number };
  object?: {
    energy: number;
    half_life: number;
    state: string;
    created_at: number;
    last_refreshed: number;
  };
  proof: VerkleProof;
}

export interface TxStatus {
  hash: string;
  state: "pending" | "mempool" | "included" | "finalised" | "rejected";
  block_height?: number;
  epoch?: number;
  error?: string;
  confirmations?: number;
  tx_index?: number;
  gas_used?: number;
  mempool_position?: number;
  mempool_size?: number;
}

export interface TxRecord {
  hash: string;
  type: string;
  from: string;
  to: string;
  amount?: number;
  object_id?: string;
  energy?: number;
  half_life?: number;
  method?: string;
  gas: number;
  block_number: number;
  epoch: number;
  status: string;
}

export interface BlockDetail {
  number: number;
  epoch: number;
  parent_hash: string;
  state_root: string;
  tx_count: number;
  evaporations: number;
  entered_grace: number;
  timestamp: number;
  active_objects: number;
  ghost_count: number;
  gas_used: number;
  base_fee: number;
  total_fees: number;
  transactions: TxRecord[];
  has_nova_proof?: boolean;
  nova_proof_size?: number;
  data_root?: string | null;
}

export interface TransactionsFeed {
  transactions: TxRecord[];
  total: number;
  limit: number;
  offset: number;
}

export interface AddressDetail {
  address: string;
  balance: number;
  nonce: number;
  last_touched_epoch: number;
  objects: Array<{
    id: string;
    name: string;
    state: string;
    energy: number;
    max_energy: number;
    half_life: number;
    decay_percentage: number;
    current_energy: number;
  }>;
  nfts: unknown[];
  tokens: unknown[];
}

export interface ShardStatus {
  active: boolean;
  num_shards?: number;
  pending_cross_shard_messages?: number;
}

export interface LightClientStatus {
  latest_trusted_height: number;
  trusted_headers_stored: number;
}

// ── Fetch helpers ────────────────────────────────────────────────────────

async function rawJson<T>(path: string): Promise<T> {
  const res = await fetch(path, { headers: { Accept: "application/json" } });
  if (!res.ok) {
    throw new Error(`API ${res.status}: ${res.statusText} (${path})`);
  }
  return (await res.json()) as T;
}

// ── Public surface ──────────────────────────────────────────────────────

export const explorerApi = {
  /** Latest N compact headers, newest first. */
  async latestHeaders(limit = 20): Promise<CompactHeader[]> {
    const status = await getClient().getChainStatus();
    const tip = status.blockHeight ?? 0;
    const from = Math.max(0, tip - limit + 1);
    const r = await rawJson<HeadersResponse>(
      `/api/light/headers?from=${from}&to=${tip}&limit=${limit}`,
    );
    return r.headers.slice().reverse();
  },

  /** Compact header for a single height (returns null on miss). */
  async headerAt(height: number): Promise<CompactHeader | null> {
    const r = await rawJson<HeadersResponse>(
      `/api/light/headers?from=${height}&to=${height}&limit=1`,
    );
    return r.headers[0] ?? null;
  },

  /** Heavy fetch: full block with transactions. */
  async fullBlock(height: number): Promise<BlockDetail> {
    return rawJson<BlockDetail>(`/api/block/${height}`);
  },

  /** Tx-status enriched response (state machine: pending → mempool → included → finalised). */
  async tx(hash: string): Promise<TxStatus> {
    return rawJson<TxStatus>(`/api/tx/${hash}`);
  },

  /** Recent transactions feed (paginated). */
  async transactions(limit = 25, offset = 0): Promise<TransactionsFeed> {
    return rawJson<TransactionsFeed>(`/api/transactions?limit=${limit}&offset=${offset}`);
  },

  /** Address overview (balance, nonce, owned objects). */
  async address(addr: string): Promise<AddressDetail> {
    return rawJson<AddressDetail>(`/api/address/${addr}`);
  },

  /** Tx inclusion proof (Merkle path within block.tx_merkle_root). */
  async txProof(blockNumber: number, txIndex: number): Promise<{ proof: TxInclusionProof; valid: boolean }> {
    return rawJson(`/api/light/tx-proof/${blockNumber}/${txIndex}`);
  },

  /** Verkle state proof for an account address. */
  async accountProof(addrHex: string): Promise<StateProofResponse> {
    return rawJson<StateProofResponse>(`/api/light/state-proof/account/${addrHex}`);
  },

  /** Verkle state proof for a state object. */
  async objectProof(objectIdHex: string): Promise<StateProofResponse> {
    return rawJson<StateProofResponse>(`/api/light/state-proof/object/${objectIdHex}`);
  },

  /** Trusted-header watermark from the on-node light-client verifier. */
  async lightClientStatus(): Promise<LightClientStatus> {
    return rawJson<LightClientStatus>(`/api/light/client/status`);
  },

  /** Shard bridge status (used to render shard chips on the address page). */
  async shardStatus(): Promise<ShardStatus> {
    return rawJson<ShardStatus>(`/api/shards`);
  },

  /** Chain tip / status (via wallet-sdk; arrives camelCased). */
  async chainStatus() {
    return getClient().getChainStatus();
  },
};

/** Strip an optional 0x prefix and lowercase a hex string. */
export function normaliseHex(raw: string): string {
  return raw.trim().replace(/^0x/i, "").toLowerCase();
}

/** Truncate a 64-char hex string for display: 0xabcd…1234. */
export function shortHex(hex: string, head = 6, tail = 4): string {
  const h = hex.startsWith("0x") ? hex : `0x${hex}`;
  if (h.length <= head + tail + 2) return h;
  return `${h.slice(0, head + 2)}…${h.slice(-tail)}`;
}

/** Compute a deterministic shard index from a 32-byte hex address. */
export function shardOf(addrHex: string, numShards: number): number {
  const norm = normaliseHex(addrHex);
  if (numShards <= 0 || norm.length < 2) return 0;
  // First byte mod num_shards — matches the simplest assignment scheme used by
  // the chain's ShardBridge until a richer locator is exposed via the API.
  const firstByte = parseInt(norm.slice(0, 2), 16);
  return Number.isFinite(firstByte) ? firstByte % numShards : 0;
}

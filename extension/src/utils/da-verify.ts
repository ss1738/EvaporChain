// Wallet-side DA light-client verification.
//
// Mirrors `crates/evaporchain-da/src/commitments.rs::verify_cell_proof` and
// `crates/evaporchain-cli/src/main.rs::da_verify_inner`. Lets the wallet
// independently verify that a node serving block data is actually serving
// real data — same chain-attestation cross-check + cell-sampling pipeline
// the CLI exposes.
//
// The verification math is byte-identical to the Rust path (blake3 leaf,
// blake3 hash-pair Merkle, even-index = (current ‖ sibling), odd-index =
// (sibling ‖ current)). Bug-for-bug parity is what makes "the wallet
// rejects a block iff a Rust node's verifier would" hold.

import { blake3 } from "@noble/hashes/blake3";

// ── Types matching the chain's API surface ─────────────────────────

export interface DaHeaderResponse {
  data_root: string; // hex
  row_roots: string[]; // hex
  col_roots: string[]; // hex
  extended_dim: number;
  original_dim: number;
  cell_size: number;
  original_len: number;
  data_hash: string; // hex
}

export interface DaCellResponse {
  block: number;
  row: number;
  col: number;
  cell_data: string; // hex
  cell_hash: string; // hex
  row_root: string; // hex
  col_root: string; // hex
  data_root: string; // hex
  extended_dim: number;
  row_proof_siblings: string[]; // hex array
  col_proof_siblings: string[]; // hex array
}

export interface BlockRecordResponse {
  number: number;
  data_root?: string | null; // hex or null
}

// ── Verifier outcomes ──────────────────────────────────────────────

export type ChainAttestation =
  | { kind: "verified" }
  | { kind: "skipped" }
  | { kind: "no-data-root" }
  | { kind: "block-not-in-ring" }
  | { kind: "mismatch"; on_chain: string; served: string };

export interface CellSampleResult {
  row: number;
  col: number;
  valid: boolean;
  peer: string;
  reason?: string;
}

export interface DaVerifyOutcome {
  attestation: ChainAttestation;
  samples_requested: number;
  samples_valid: number;
  unique_rows_hit: number;
  unique_cols_hit: number;
  /** Celestia DAS confidence: 1 - 2^(-samples_valid). */
  confidence: number;
  faulty_peers: { peer: string; reason: string }[];
  all_valid: boolean;
  passes: boolean;
}

export interface DaVerifyOptions {
  /** Number of cells to sample. Cap at 256 to mirror the CLI. */
  samples?: number;
  /** Confidence threshold the run must clear for `passes = true`. */
  threshold?: number;
  /** Optional 32-byte hex seed. Defaults to blake3(block_le_bytes). */
  seedHex?: string;
  /** Skip the on-chain data_root cross-check (operator opt-out). */
  skipChainAttestation?: boolean;
}

// ── Hex helpers ────────────────────────────────────────────────────

function hexToBytes(hex: string): Uint8Array {
  const stripped = hex.replace(/^0x/, "");
  if (stripped.length % 2 !== 0) {
    throw new Error(`hex string has odd length: ${stripped.length}`);
  }
  const out = new Uint8Array(stripped.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(stripped.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

function u64LeBytes(n: number | bigint): Uint8Array {
  const v = typeof n === "bigint" ? n : BigInt(n);
  const out = new Uint8Array(8);
  let x = v;
  for (let i = 0; i < 8; i++) {
    out[i] = Number(x & 0xffn);
    x >>= 8n;
  }
  return out;
}

function u64FromLeBytes(bytes: Uint8Array, offset: number): bigint {
  let v = 0n;
  for (let i = 0; i < 8; i++) {
    v |= BigInt(bytes[offset + i]) << BigInt(i * 8);
  }
  return v;
}

// ── Merkle path verification (byte-identical to Rust) ──────────────

/** `crates/evaporchain-da/src/commitments.rs::verify_merkle_path` */
function verifyMerklePath(
  leaf: Uint8Array,
  index: number,
  siblings: Uint8Array[],
): Uint8Array {
  let current = leaf;
  let idx = index;
  for (const sibling of siblings) {
    const combined = new Uint8Array(64);
    if (idx % 2 === 0) {
      combined.set(current, 0);
      combined.set(sibling, 32);
    } else {
      combined.set(sibling, 0);
      combined.set(current, 32);
    }
    current = blake3(combined);
    idx = Math.floor(idx / 2);
  }
  return current;
}

/** `crates/evaporchain-da/src/commitments.rs::generate_2d_queries` */
function generate2dQueries(
  block: number,
  extendedDim: number,
  numSamples: number,
  seed: Uint8Array,
): { row: number; col: number }[] {
  const queries: { row: number; col: number }[] = [];
  const blockLe = u64LeBytes(block);
  const dim = BigInt(extendedDim);
  for (let i = 0; i < numSamples; i++) {
    const idxLe = u64LeBytes(i);
    const buf = new Uint8Array(seed.length + 16);
    buf.set(seed, 0);
    buf.set(blockLe, seed.length);
    buf.set(idxLe, seed.length + 8);
    const hash = blake3(buf);
    const row = Number(u64FromLeBytes(hash, 0) % dim);
    const col = Number(u64FromLeBytes(hash, 8) % dim);
    queries.push({ row, col });
  }
  return queries;
}

/** Verify a single CellProof against a header. Returns null on success
 *  or a human-readable reason on failure. */
function verifyCellProof(
  header: DaHeaderResponse,
  proof: DaCellResponse,
): string | null {
  const cellData = hexToBytes(proof.cell_data);
  const claimedHash = hexToBytes(proof.cell_hash);
  const recomputedHash = blake3(cellData);
  if (!bytesEqual(claimedHash, recomputedHash)) {
    return "cell_hash does not match blake3(cell_data)";
  }

  const claimedRowRoot = hexToBytes(proof.row_root);
  const rowSiblings = proof.row_proof_siblings.map(hexToBytes);
  const recomputedRowRoot = verifyMerklePath(
    recomputedHash,
    proof.col,
    rowSiblings,
  );
  if (!bytesEqual(claimedRowRoot, recomputedRowRoot)) {
    return "row Merkle path does not reconstruct to row_root";
  }

  const claimedColRoot = hexToBytes(proof.col_root);
  const colSiblings = proof.col_proof_siblings.map(hexToBytes);
  const recomputedColRoot = verifyMerklePath(
    recomputedHash,
    proof.row,
    colSiblings,
  );
  if (!bytesEqual(claimedColRoot, recomputedColRoot)) {
    return "col Merkle path does not reconstruct to col_root";
  }

  // Header consistency: the served data_root must match what the header
  // committed to, and the row/col root indexed by this cell's coords
  // must match what the cell carries.
  if (proof.data_root.replace(/^0x/, "").toLowerCase() !== header.data_root.replace(/^0x/, "").toLowerCase()) {
    return "cell data_root differs from header data_root";
  }
  if (header.row_roots[proof.row]?.replace(/^0x/, "").toLowerCase() !== proof.row_root.replace(/^0x/, "").toLowerCase()) {
    return "header.row_roots[row] differs from cell-served row_root";
  }
  if (header.col_roots[proof.col]?.replace(/^0x/, "").toLowerCase() !== proof.col_root.replace(/^0x/, "").toLowerCase()) {
    return "header.col_roots[col] differs from cell-served col_root";
  }
  return null;
}

// ── HTTP plumbing ──────────────────────────────────────────────────

async function fetchJson<T>(url: string): Promise<{ status: number; body: T | null }> {
  const res = await fetch(url);
  const status = res.status;
  if (status === 404) return { status, body: null };
  if (!res.ok) {
    throw new Error(`${url} → ${status}: ${await res.text().catch(() => "")}`);
  }
  return { status, body: (await res.json()) as T };
}

// ── Public API ─────────────────────────────────────────────────────

/** Run a DA verification round against `nodes` for `block`. Round-robins
 *  per cell across multiple URLs so faulty-peer reports name the right
 *  node. Mirrors `evaporchain da verify`. */
export async function daVerify(
  nodes: string[],
  block: number,
  opts: DaVerifyOptions = {},
): Promise<DaVerifyOutcome> {
  if (nodes.length === 0) {
    throw new Error("daVerify requires at least one node URL");
  }
  const samples = Math.min(Math.max(opts.samples ?? 16, 1), 256);
  const threshold = opts.threshold ?? 0.999;
  if (!(threshold > 0 && threshold < 1)) {
    throw new Error(`threshold must be in (0, 1); got ${threshold}`);
  }
  const bases = nodes.map((n) => n.replace(/\/$/, ""));
  const primary = bases[0];

  // 1. Fetch header from the primary node.
  const headerResp = await fetchJson<DaHeaderResponse>(
    `${primary}/api/da/header/${block}`,
  );
  if (!headerResp.body) {
    throw new Error(`no DA header for block ${block} on ${primary}`);
  }
  const header = headerResp.body;

  // 2. Chain attestation cross-check.
  let attestation: ChainAttestation;
  if (opts.skipChainAttestation) {
    attestation = { kind: "skipped" };
  } else {
    const blockResp = await fetchJson<BlockRecordResponse>(
      `${primary}/api/block/${block}`,
    );
    if (!blockResp.body) {
      attestation = { kind: "block-not-in-ring" };
    } else if (blockResp.body.data_root == null) {
      attestation = { kind: "no-data-root" };
    } else {
      const onChain = blockResp.body.data_root
        .replace(/^0x/, "")
        .toLowerCase();
      const served = header.data_root.replace(/^0x/, "").toLowerCase();
      if (onChain === served) {
        attestation = { kind: "verified" };
      } else {
        // Hard fail — match the CLI's behaviour of refusing to sample
        // a block whose data_root the producer is lying about.
        return {
          attestation: { kind: "mismatch", on_chain: onChain, served },
          samples_requested: 0,
          samples_valid: 0,
          unique_rows_hit: 0,
          unique_cols_hit: 0,
          confidence: 0,
          faulty_peers: [],
          all_valid: false,
          passes: false,
        };
      }
    }
  }

  // 3. Generate cell queries (deterministic seed = blake3(block_le)).
  const seed = opts.seedHex
    ? hexToBytes(opts.seedHex)
    : blake3(u64LeBytes(block));
  const queries = generate2dQueries(block, header.extended_dim, samples, seed);

  // 4. Round-robin per cell across `bases`. Verify each proof locally.
  const results: CellSampleResult[] = [];
  const faultyPeers = new Map<string, string>();
  let validCount = 0;
  const rows = new Set<number>();
  const cols = new Set<number>();

  for (let i = 0; i < queries.length; i++) {
    const q = queries[i];
    const peer = bases[i % bases.length];
    if (q.row >= header.extended_dim || q.col >= header.extended_dim) {
      results.push({ row: q.row, col: q.col, valid: false, peer, reason: "out-of-range" });
      continue;
    }
    try {
      const resp = await fetchJson<DaCellResponse>(
        `${peer}/api/da/cell/${block}/${q.row}/${q.col}`,
      );
      if (!resp.body) {
        results.push({ row: q.row, col: q.col, valid: false, peer, reason: "not-found" });
        if (!faultyPeers.has(peer)) faultyPeers.set(peer, "NotFound");
        continue;
      }
      const reason = verifyCellProof(header, resp.body);
      if (reason !== null) {
        results.push({ row: q.row, col: q.col, valid: false, peer, reason });
        if (!faultyPeers.has(peer)) faultyPeers.set(peer, "BadProof");
        continue;
      }
      results.push({ row: q.row, col: q.col, valid: true, peer });
      validCount += 1;
      rows.add(q.row);
      cols.add(q.col);
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      results.push({ row: q.row, col: q.col, valid: false, peer, reason });
      if (!faultyPeers.has(peer)) faultyPeers.set(peer, `Transport: ${reason}`);
    }
  }

  const confidence = validCount === 0 ? 0 : 1 - Math.pow(2, -validCount);
  const allValid = results.every((r) => r.valid);
  // Mismatch attestation early-returns at line 297; by the time we
  // get here `attestation.kind` is provably one of verified / skipped
  // / no-data-root / block-not-in-ring, so no mismatch check needed.
  const passes =
    allValid &&
    faultyPeers.size === 0 &&
    confidence >= threshold;

  return {
    attestation,
    samples_requested: results.length,
    samples_valid: validCount,
    unique_rows_hit: rows.size,
    unique_cols_hit: cols.size,
    confidence,
    faulty_peers: Array.from(faultyPeers.entries()).map(([peer, reason]) => ({
      peer,
      reason,
    })),
    all_valid: allValid,
    passes,
  };
}

// Re-export internal helpers for tests + adjacent tooling. Exposing them
// lets a future test in `extension/src/utils/da-verify.test.ts` drive
// `verifyCellProof` against synthetic fixtures without going through HTTP.
export const __internal = {
  verifyMerklePath,
  verifyCellProof,
  generate2dQueries,
  hexToBytes,
  bytesToHex,
  u64LeBytes,
  u64FromLeBytes,
};

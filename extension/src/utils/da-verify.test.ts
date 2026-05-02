// Vitest unit tests for da-verify.ts.
//
// Covers the verification math directly via the __internal exports so we
// don't need a server. The end-to-end multi-peer + chain-attestation
// path is covered by the Rust integration suite at
// `crates/evaporchain-cli/src/main.rs::tests::da_verify_*` against the
// same endpoint shapes — bug-for-bug parity is the contract.

import { describe, expect, it } from "vitest";
import { blake3 } from "@noble/hashes/blake3";
import { __internal } from "./da-verify";
import type { DaCellResponse, DaHeaderResponse } from "./da-verify";

const {
  verifyMerklePath,
  verifyCellProof,
  generate2dQueries,
  hexToBytes,
  bytesToHex,
  u64LeBytes,
  u64FromLeBytes,
} = __internal;

// ── Hex round-trip ────────────────────────────────────────────────

describe("hex helpers", () => {
  it("bytesToHex round-trips", () => {
    const orig = new Uint8Array([0, 1, 15, 16, 254, 255]);
    const hex = bytesToHex(orig);
    expect(hex).toBe("00010f10feff");
    expect(hexToBytes(hex)).toEqual(orig);
  });

  it("hexToBytes accepts 0x prefix", () => {
    expect(hexToBytes("0xdeadbeef")).toEqual(
      new Uint8Array([0xde, 0xad, 0xbe, 0xef]),
    );
  });

  it("hexToBytes rejects odd-length input", () => {
    expect(() => hexToBytes("abc")).toThrow();
  });
});

// ── u64 LE conversion ─────────────────────────────────────────────

describe("u64 LE conversion", () => {
  it("u64LeBytes round-trips through u64FromLeBytes", () => {
    const cases: bigint[] = [
      0n,
      1n,
      255n,
      256n,
      4_294_967_296n, // 2^32
      18_446_744_073_709_551_615n, // u64::MAX
    ];
    for (const v of cases) {
      const bytes = u64LeBytes(v);
      expect(bytes.length).toBe(8);
      const decoded = u64FromLeBytes(bytes, 0);
      expect(decoded).toBe(v);
    }
  });

  it("u64LeBytes matches expected layout for 0x0123456789abcdef", () => {
    // 0x0123456789abcdef → little-endian bytes:
    // ef cd ab 89 67 45 23 01
    const v = 0x0123_4567_89ab_cdefn;
    const bytes = u64LeBytes(v);
    expect(Array.from(bytes)).toEqual([
      0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01,
    ]);
  });

  it("u64FromLeBytes reads at offset", () => {
    const buf = new Uint8Array(16);
    buf.set([0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01], 8);
    expect(u64FromLeBytes(buf, 8)).toBe(0x0123_4567_89ab_cdefn);
  });
});

// ── Merkle path verification ──────────────────────────────────────

describe("verifyMerklePath", () => {
  function hashPair(left: Uint8Array, right: Uint8Array): Uint8Array {
    const buf = new Uint8Array(64);
    buf.set(left, 0);
    buf.set(right, 32);
    return blake3(buf);
  }

  it("reconstructs root for a 4-leaf tree at index 0", () => {
    const leaves = [
      blake3(new Uint8Array([0])),
      blake3(new Uint8Array([1])),
      blake3(new Uint8Array([2])),
      blake3(new Uint8Array([3])),
    ];
    // Build the tree bottom-up.
    const layer1 = [hashPair(leaves[0], leaves[1]), hashPair(leaves[2], leaves[3])];
    const root = hashPair(layer1[0], layer1[1]);

    // For leaf 0: siblings are leaves[1] (at depth 0), layer1[1] (at depth 1).
    const recomputed = verifyMerklePath(leaves[0], 0, [leaves[1], layer1[1]]);
    expect(recomputed).toEqual(root);
  });

  it("reconstructs root for a 4-leaf tree at index 3 (right-side path)", () => {
    const leaves = [
      blake3(new Uint8Array([0])),
      blake3(new Uint8Array([1])),
      blake3(new Uint8Array([2])),
      blake3(new Uint8Array([3])),
    ];
    const layer1 = [hashPair(leaves[0], leaves[1]), hashPair(leaves[2], leaves[3])];
    const root = hashPair(layer1[0], layer1[1]);

    // For leaf 3: index=3 (binary 11). At depth 0 sibling is leaves[2]
    // (index even? 3%2==1 → odd → sibling on left). At depth 1
    // sibling is layer1[0] (index now 1, odd → sibling on left).
    const recomputed = verifyMerklePath(leaves[3], 3, [leaves[2], layer1[0]]);
    expect(recomputed).toEqual(root);
  });

  it("returns leaf for empty siblings (single-leaf tree)", () => {
    const leaf = blake3(new Uint8Array([42]));
    expect(verifyMerklePath(leaf, 0, [])).toEqual(leaf);
  });

  it("wrong sibling produces a different root", () => {
    const leaves = [
      blake3(new Uint8Array([0])),
      blake3(new Uint8Array([1])),
    ];
    const real = hashPair(leaves[0], leaves[1]);
    const tampered = blake3(new Uint8Array([99]));
    const recomputed = verifyMerklePath(leaves[0], 0, [tampered]);
    expect(recomputed).not.toEqual(real);
  });
});

// ── 2D query generation ──────────────────────────────────────────

describe("generate2dQueries", () => {
  it("is deterministic for the same (block, dim, samples, seed)", () => {
    const seed = blake3(u64LeBytes(7));
    const a = generate2dQueries(7, 8, 16, seed);
    const b = generate2dQueries(7, 8, 16, seed);
    expect(a).toEqual(b);
  });

  it("returns the requested number of queries", () => {
    const seed = blake3(u64LeBytes(1));
    const queries = generate2dQueries(1, 16, 32, seed);
    expect(queries.length).toBe(32);
  });

  it("constrains row/col within [0, extended_dim)", () => {
    const seed = blake3(u64LeBytes(3));
    const dim = 8;
    const queries = generate2dQueries(3, dim, 100, seed);
    for (const q of queries) {
      expect(q.row).toBeGreaterThanOrEqual(0);
      expect(q.row).toBeLessThan(dim);
      expect(q.col).toBeGreaterThanOrEqual(0);
      expect(q.col).toBeLessThan(dim);
    }
  });

  it("different seeds produce different query sets", () => {
    const seedA = blake3(new Uint8Array([1]));
    const seedB = blake3(new Uint8Array([2]));
    const a = generate2dQueries(0, 16, 8, seedA);
    const b = generate2dQueries(0, 16, 8, seedB);
    // At least one query differs — not a strict guarantee in theory,
    // but for blake3 with different seeds the probability of full
    // collision is astronomical.
    expect(JSON.stringify(a)).not.toBe(JSON.stringify(b));
  });
});

// ── verifyCellProof — synthetic 2×2 fixture ──────────────────────

/** Build a synthetic 2×2 BlockDA2D-style fixture. Picks 4 cells, computes
 *  cell hashes, then builds row/col roots via the same Merkle scheme the
 *  Rust verifier uses. Returns a header + per-cell proof so we can drive
 *  verifyCellProof against known-good and tampered versions. */
function buildFixture() {
  function hashPair(left: Uint8Array, right: Uint8Array): Uint8Array {
    const buf = new Uint8Array(64);
    buf.set(left, 0);
    buf.set(right, 32);
    return blake3(buf);
  }

  // 2×2 grid of 4-byte cells. cells[row][col] is the raw payload.
  const cells: Uint8Array[][] = [
    [new Uint8Array([0xa0, 0x01]), new Uint8Array([0xa0, 0x02])],
    [new Uint8Array([0xa1, 0x01]), new Uint8Array([0xa1, 0x02])],
  ];
  const cellHashes: Uint8Array[][] = cells.map((r) => r.map((c) => blake3(c)));

  // Row roots: row i's root = hash_pair(cellHash[i][0], cellHash[i][1]).
  // Sibling for col=0: cellHash[i][1]. For col=1: cellHash[i][0].
  const rowRoots = cellHashes.map((r) => hashPair(r[0], r[1]));
  // Col roots: col j's root = hash_pair(cellHash[0][j], cellHash[1][j]).
  const colRoots = [
    hashPair(cellHashes[0][0], cellHashes[1][0]),
    hashPair(cellHashes[0][1], cellHashes[1][1]),
  ];

  // data_root: blake3 over (row_roots ‖ col_roots) — for the fixture we
  // just hash a deterministic concatenation. The Rust impl uses a Merkle
  // tree over the combined leaves; for a 2×2 fixture the test matters
  // only for the cross-check assertion that the served data_root agrees
  // with the header data_root, so we use ANY consistent value.
  const concat = new Uint8Array(rowRoots.length * 32 + colRoots.length * 32);
  rowRoots.forEach((r, i) => concat.set(r, i * 32));
  colRoots.forEach((c, i) => concat.set(c, (rowRoots.length + i) * 32));
  const dataRoot = blake3(concat);

  const header: DaHeaderResponse = {
    data_root: bytesToHex(dataRoot),
    row_roots: rowRoots.map(bytesToHex),
    col_roots: colRoots.map(bytesToHex),
    extended_dim: 2,
    original_dim: 1,
    cell_size: 2,
    original_len: 4,
    data_hash: bytesToHex(blake3(new Uint8Array(0))),
  };

  // Build a CellProof for cell (row=0, col=1).
  const r = 0;
  const c = 1;
  const proof: DaCellResponse = {
    block: 1,
    row: r,
    col: c,
    cell_data: bytesToHex(cells[r][c]),
    cell_hash: bytesToHex(cellHashes[r][c]),
    row_root: bytesToHex(rowRoots[r]),
    col_root: bytesToHex(colRoots[c]),
    data_root: bytesToHex(dataRoot),
    extended_dim: 2,
    // Row Merkle path for col=1 in a 2-leaf row: sibling is cellHash[r][0].
    row_proof_siblings: [bytesToHex(cellHashes[r][0])],
    // Col Merkle path for row=0 in a 2-leaf col: sibling is cellHash[1][c].
    col_proof_siblings: [bytesToHex(cellHashes[1][c])],
  };
  return { header, proof, cells, cellHashes };
}

describe("verifyCellProof", () => {
  it("accepts a valid proof", () => {
    const { header, proof } = buildFixture();
    expect(verifyCellProof(header, proof)).toBeNull();
  });

  it("rejects when cell_data has been tampered with", () => {
    const { header, proof } = buildFixture();
    // Flip a byte in the served cell_data so its blake3 no longer
    // matches the claimed cell_hash.
    const tampered = { ...proof, cell_data: "99" + proof.cell_data.slice(2) };
    const reason = verifyCellProof(header, tampered);
    expect(reason).not.toBeNull();
    expect(reason).toContain("cell_hash");
  });

  it("rejects when a row sibling is wrong", () => {
    const { header, proof } = buildFixture();
    const tampered = {
      ...proof,
      row_proof_siblings: [bytesToHex(blake3(new Uint8Array([0xff])))],
    };
    const reason = verifyCellProof(header, tampered);
    expect(reason).toContain("row Merkle");
  });

  it("rejects when col sibling is wrong", () => {
    const { header, proof } = buildFixture();
    const tampered = {
      ...proof,
      col_proof_siblings: [bytesToHex(blake3(new Uint8Array([0xff])))],
    };
    const reason = verifyCellProof(header, tampered);
    expect(reason).toContain("col Merkle");
  });

  it("rejects when served data_root disagrees with header", () => {
    const { header, proof } = buildFixture();
    const tampered = { ...proof, data_root: bytesToHex(new Uint8Array(32)) };
    const reason = verifyCellProof(header, tampered);
    expect(reason).toContain("data_root");
  });

  it("rejects when header.row_roots[row] disagrees with cell-served row_root", () => {
    const { header, proof } = buildFixture();
    // Swap header's row_roots so the indexed entry no longer matches
    // the cell-served root.
    const swapped: DaHeaderResponse = {
      ...header,
      row_roots: [bytesToHex(new Uint8Array(32)), header.row_roots[1]],
    };
    const reason = verifyCellProof(swapped, proof);
    expect(reason).toContain("row_roots");
  });
});

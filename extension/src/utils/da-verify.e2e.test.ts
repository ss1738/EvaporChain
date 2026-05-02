// End-to-end tests for `daVerify` — the orchestration on top of the
// math primitives covered by da-verify.test.ts.
//
// Stubs `globalThis.fetch` to mock the three endpoints daVerify
// consumes (/api/block/:N, /api/da/header/:N, /api/da/cell/:N/r/c).
// Asserts each ChainAttestation outcome plus multi-peer round-robin —
// mirrors the Rust integration suite at
// `crates/evaporchain-cli/src/main.rs::tests::da_verify_*` so any
// future change to the orchestration produces the same behaviour on
// both sides.

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { blake3 } from "@noble/hashes/blake3";
import { daVerify, __internal } from "./da-verify";

const { bytesToHex } = __internal;

// 16 samples gives 1 - 2^-16 ≈ 0.99998 confidence, comfortably above
// daVerify's default 0.999 threshold. Lower counts (e.g. 4 → 0.9375)
// fail the threshold check and `passes` would be false even when
// every cell verifies.
const SAMPLES = 16;

// ── Synthetic 2×2 fixture (mirrors da-verify.test.ts's buildFixture) ──

function hashPair(left: Uint8Array, right: Uint8Array): Uint8Array {
  const buf = new Uint8Array(64);
  buf.set(left, 0);
  buf.set(right, 32);
  return blake3(buf);
}

interface Fixture {
  block: number;
  headerJson: Record<string, unknown>;
  /** Map of `{row}/{col}` → cell-proof JSON. */
  cells: Record<string, Record<string, unknown>>;
  /** On-chain data_root the BlockRecord endpoint returns. */
  onChainDataRootHex: string;
}

function buildFixture(block: number, tamperCells: boolean = false): Fixture {
  const cells: Uint8Array[][] = [
    [new Uint8Array([0xa0, 0x01]), new Uint8Array([0xa0, 0x02])],
    [new Uint8Array([0xa1, 0x01]), new Uint8Array([0xa1, 0x02])],
  ];
  const cellHashes = cells.map((row) => row.map((c) => blake3(c)));
  const rowRoots = cellHashes.map((row) => hashPair(row[0], row[1]));
  const colRoots = [
    hashPair(cellHashes[0][0], cellHashes[1][0]),
    hashPair(cellHashes[0][1], cellHashes[1][1]),
  ];
  const concat = new Uint8Array(rowRoots.length * 32 + colRoots.length * 32);
  rowRoots.forEach((r, i) => concat.set(r, i * 32));
  colRoots.forEach((c, i) => concat.set(c, (rowRoots.length + i) * 32));
  const dataRoot = blake3(concat);

  const headerJson = {
    data_root: bytesToHex(dataRoot),
    row_roots: rowRoots.map(bytesToHex),
    col_roots: colRoots.map(bytesToHex),
    extended_dim: 2,
    original_dim: 1,
    cell_size: 2,
    original_len: 4,
    data_hash: bytesToHex(blake3(new Uint8Array(0))),
  };

  const cellMap: Record<string, Record<string, unknown>> = {};
  for (let r = 0; r < 2; r++) {
    for (let c = 0; c < 2; c++) {
      const cellData = cells[r][c];
      const wireCellData = tamperCells
        ? new Uint8Array([cellData[0] ^ 0xff, cellData[1]])
        : cellData;
      cellMap[`${r}/${c}`] = {
        block,
        row: r,
        col: c,
        cell_data: bytesToHex(wireCellData),
        cell_hash: bytesToHex(cellHashes[r][c]),
        row_root: bytesToHex(rowRoots[r]),
        col_root: bytesToHex(colRoots[c]),
        data_root: bytesToHex(dataRoot),
        extended_dim: 2,
        row_proof_siblings: [bytesToHex(cellHashes[r][1 - c])],
        col_proof_siblings: [bytesToHex(cellHashes[1 - r][c])],
      };
    }
  }

  return {
    block,
    headerJson,
    cells: cellMap,
    onChainDataRootHex: bytesToHex(dataRoot),
  };
}

// ── Fetch mock ───────────────────────────────────────────────────

type Resp = { status: number; body: unknown };
type Handler = (url: URL) => Resp | "passthrough";

interface MockNode {
  base: string;
  handle: Handler;
}

function defaultHandler(
  fixture: Fixture,
  blockOverride?: "absent" | "no-root" | "wrong-root",
): Handler {
  return (url) => {
    const path = url.pathname;
    if (path === `/api/block/${fixture.block}`) {
      if (blockOverride === "absent") {
        return { status: 404, body: { error: "block not in ring" } };
      }
      if (blockOverride === "no-root") {
        return { status: 200, body: { number: fixture.block, data_root: null } };
      }
      if (blockOverride === "wrong-root") {
        return {
          status: 200,
          body: {
            number: fixture.block,
            data_root: "0x" + "ff".repeat(32),
          },
        };
      }
      return {
        status: 200,
        body: {
          number: fixture.block,
          data_root: "0x" + fixture.onChainDataRootHex,
        },
      };
    }
    if (path === `/api/da/header/${fixture.block}`) {
      return { status: 200, body: fixture.headerJson };
    }
    const cellMatch = path.match(/^\/api\/da\/cell\/(\d+)\/(\d+)\/(\d+)$/);
    if (cellMatch && Number(cellMatch[1]) === fixture.block) {
      const key = `${cellMatch[2]}/${cellMatch[3]}`;
      const cell = fixture.cells[key];
      if (!cell) return { status: 404, body: { error: "no cell" } };
      return { status: 200, body: cell };
    }
    return { status: 404, body: { error: "unhandled" } };
  };
}

function installFetchMock(nodes: MockNode[]) {
  const original = globalThis.fetch;
  globalThis.fetch = async (input: RequestInfo | URL): Promise<Response> => {
    const urlStr = typeof input === "string" ? input : input.toString();
    const url = new URL(urlStr);
    const node = nodes.find((n) => urlStr.startsWith(n.base));
    if (!node) {
      return new Response(JSON.stringify({ error: "no mock for " + urlStr }), {
        status: 500,
      });
    }
    const resp = node.handle(url);
    if (resp === "passthrough") {
      return original(input);
    }
    return new Response(JSON.stringify(resp.body), {
      status: resp.status,
      headers: { "Content-Type": "application/json" },
    });
  };
  return () => {
    globalThis.fetch = original;
  };
}

// ── Tests ────────────────────────────────────────────────────────

describe("daVerify end-to-end (mocked fetch)", () => {
  let restore: (() => void) | null = null;
  beforeEach(() => {
    restore = null;
  });
  afterEach(() => {
    if (restore) restore();
  });

  it("verified path: chain root matches header → all cells valid → passes", async () => {
    const fixture = buildFixture(7);
    restore = installFetchMock([
      { base: "http://n1:8080", handle: defaultHandler(fixture) },
    ]);
    const out = await daVerify(["http://n1:8080"], 7, { samples: SAMPLES });
    expect(out.attestation.kind).toBe("verified");
    expect(out.all_valid).toBe(true);
    expect(out.passes).toBe(true);
    expect(out.faulty_peers).toEqual([]);
  });

  it("mismatch path: chain root != header root → hard-fail outcome, no sampling", async () => {
    const fixture = buildFixture(7);
    restore = installFetchMock([
      { base: "http://n1:8080", handle: defaultHandler(fixture, "wrong-root") },
    ]);
    const out = await daVerify(["http://n1:8080"], 7, { samples: SAMPLES });
    expect(out.attestation.kind).toBe("mismatch");
    expect(out.passes).toBe(false);
    expect(out.samples_requested).toBe(0);
  });

  it("skipped path: --skip-chain-attestation runs sampling regardless", async () => {
    const fixture = buildFixture(7);
    restore = installFetchMock([
      { base: "http://n1:8080", handle: defaultHandler(fixture, "wrong-root") },
    ]);
    const out = await daVerify(["http://n1:8080"], 7, {
      samples: SAMPLES,
      skipChainAttestation: true,
    });
    expect(out.attestation.kind).toBe("skipped");
    expect(out.passes).toBe(true);
  });

  it("block-not-in-ring path: chain returns 404 but sampling proceeds", async () => {
    const fixture = buildFixture(7);
    restore = installFetchMock([
      { base: "http://n1:8080", handle: defaultHandler(fixture, "absent") },
    ]);
    const out = await daVerify(["http://n1:8080"], 7, { samples: SAMPLES });
    expect(out.attestation.kind).toBe("block-not-in-ring");
    expect(out.all_valid).toBe(true);
    expect(out.passes).toBe(true);
  });

  it("no-data-root path: chain serves block with null data_root", async () => {
    const fixture = buildFixture(7);
    restore = installFetchMock([
      { base: "http://n1:8080", handle: defaultHandler(fixture, "no-root") },
    ]);
    const out = await daVerify(["http://n1:8080"], 7, { samples: SAMPLES });
    expect(out.attestation.kind).toBe("no-data-root");
    expect(out.passes).toBe(true);
  });

  it("fabricated cells: peer serves tampered cell_data → faulty_peers names it", async () => {
    const honest = buildFixture(7);
    const tampered = buildFixture(7, /* tamperCells= */ true);
    restore = installFetchMock([
      {
        base: "http://lying:8080",
        handle: (url) => {
          if (url.pathname === `/api/block/${honest.block}`) {
            return defaultHandler(honest)(url);
          }
          if (url.pathname === `/api/da/header/${honest.block}`) {
            return defaultHandler(honest)(url);
          }
          return defaultHandler(tampered)(url);
        },
      },
    ]);
    const out = await daVerify(["http://lying:8080"], 7, { samples: SAMPLES });
    expect(out.attestation.kind).toBe("verified");
    expect(out.all_valid).toBe(false);
    expect(out.faulty_peers.length).toBeGreaterThanOrEqual(1);
    expect(out.faulty_peers.some((p) => p.peer === "http://lying:8080")).toBe(
      true,
    );
    expect(out.passes).toBe(false);
  });

  it("multi-peer round-robin: bad node named, honest nodes spared", async () => {
    const honest = buildFixture(7);
    const tampered = buildFixture(7, true);
    const honestHandler = defaultHandler(honest);
    const tamperedHandler = (url: URL) => {
      if (url.pathname === `/api/block/${honest.block}`) return honestHandler(url);
      if (url.pathname === `/api/da/header/${honest.block}`) return honestHandler(url);
      return defaultHandler(tampered)(url);
    };
    restore = installFetchMock([
      { base: "http://n1:8080", handle: honestHandler },
      { base: "http://n2:8080", handle: honestHandler },
      { base: "http://bad:8080", handle: tamperedHandler },
    ]);
    const out = await daVerify(
      ["http://n1:8080", "http://n2:8080", "http://bad:8080"],
      7,
      { samples: SAMPLES },
    );
    expect(out.attestation.kind).toBe("verified");
    expect(out.faulty_peers.some((p) => p.peer === "http://bad:8080")).toBe(
      true,
    );
    expect(out.faulty_peers.every((p) => p.peer !== "http://n1:8080")).toBe(
      true,
    );
    expect(out.faulty_peers.every((p) => p.peer !== "http://n2:8080")).toBe(
      true,
    );
    expect(out.passes).toBe(false);
  });

  it("rejects empty nodes list", async () => {
    await expect(daVerify([], 7)).rejects.toThrow(/at least one node URL/);
  });

  it("rejects threshold outside (0, 1)", async () => {
    const fixture = buildFixture(7);
    restore = installFetchMock([
      { base: "http://n1:8080", handle: defaultHandler(fixture) },
    ]);
    await expect(
      daVerify(["http://n1:8080"], 7, { threshold: 1.5 }),
    ).rejects.toThrow(/threshold/);
    await expect(
      daVerify(["http://n1:8080"], 7, { threshold: 0 }),
    ).rejects.toThrow(/threshold/);
  });
});

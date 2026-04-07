/**
 * Block explorer integration tests.
 * Verifies blocks, transactions, accounts, and address lookups.
 */
import { describe, it, expect } from "vitest";
import { get } from "../helpers/client";

describe("Block Explorer", () => {
  it("GET /api/blocks returns a list of blocks", async () => {
    const res = await get<unknown[]>("/api/blocks");
    expect(res.ok).toBe(true);
    expect(Array.isArray(res.data)).toBe(true);
    expect(res.data.length).toBeGreaterThan(0);
  });

  it("GET /api/blocks/latest returns the latest block", async () => {
    const res = await get<Record<string, unknown>>("/api/blocks/latest");
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("number");
  });

  it("GET /api/block/latest is an alias for latest block", async () => {
    const res = await get<Record<string, unknown>>("/api/block/latest");
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("number");
  });

  it("GET /api/block/1 returns first block", async () => {
    const blocks = await get<Array<{ number: number }>>("/api/blocks");
    if (blocks.data.length === 0) return;
    const first = blocks.data[blocks.data.length - 1];
    const res = await get<Record<string, unknown>>(`/api/block/${first.number}`);
    expect(res.ok).toBe(true);
    expect(res.data).toBeDefined();
  });

  it("GET /api/block/:number returns specific block", async () => {
    const latest = await get<{ number: number }>("/api/blocks/latest");
    const blockNum = latest.data.number;
    const res = await get<Record<string, unknown>>(`/api/block/${blockNum}`);
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("number");
  });

  it("GET /api/transactions returns transaction list", async () => {
    const res = await get<{ transactions: unknown[] }>("/api/transactions");
    expect(res.ok).toBe(true);
    expect(Array.isArray(res.data.transactions)).toBe(true);
  });

  it("GET /api/accounts returns account list", async () => {
    const res = await get<unknown[]>("/api/accounts");
    expect(res.ok).toBe(true);
    expect(Array.isArray(res.data)).toBe(true);
  });

  it("GET /api/ghosts returns ghost objects", async () => {
    const res = await get("/api/ghosts");
    expect(res.ok).toBe(true);
  });

  it("latest block number matches status block_height", async () => {
    const status = await get<{ block_height: number }>("/api/status");
    const latest = await get<{ number: number } | null>("/api/blocks/latest");
    if (!latest.data || !("number" in latest.data)) return;
    // They should be very close (within 2 blocks due to timing)
    expect(Math.abs(status.data.block_height - latest.data.number)).toBeLessThanOrEqual(2);
  });
});

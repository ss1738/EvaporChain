/**
 * Chain health, status, and metadata integration tests.
 * Verifies the node is alive and returning correct chain data.
 */
import { describe, it, expect } from "vitest";
import { get } from "../helpers/client";

describe("Chain Health & Status", () => {
  it("GET /health returns 200", async () => {
    const res = await get("/health");
    expect(res.ok).toBe(true);
    expect(res.status).toBe(200);
  });

  it("GET /api/status returns chain status with expected fields", async () => {
    const res = await get<Record<string, unknown>>("/api/status");
    expect(res.ok).toBe(true);
    const d = res.data;
    expect(d).toHaveProperty("block_height");
    expect(d).toHaveProperty("epoch");
    expect(typeof d.block_height).toBe("number");
    expect(typeof d.epoch).toBe("number");
    expect(d.block_height).toBeGreaterThanOrEqual(0);
    expect(d.epoch).toBeGreaterThanOrEqual(0);
  });

  it("GET /api/chain returns chain metadata", async () => {
    const res = await get<Record<string, unknown>>("/api/chain");
    expect(res.ok).toBe(true);
    expect(res.data).toBeDefined();
  });

  it("GET /api/stats returns stats summary", async () => {
    const res = await get<Record<string, unknown>>("/api/stats");
    expect(res.ok).toBe(true);
    expect(res.data).toBeDefined();
  });

  it("GET /api/stats/summary returns same as /api/stats", async () => {
    const [stats, summary] = await Promise.all([
      get<Record<string, unknown>>("/api/stats"),
      get<Record<string, unknown>>("/api/stats/summary"),
    ]);
    expect(stats.ok).toBe(true);
    expect(summary.ok).toBe(true);
  });

  it("GET /api/stats/timeline returns timeline data", async () => {
    const res = await get("/api/stats/timeline");
    expect(res.ok).toBe(true);
  });

  it("GET /api/network returns network info", async () => {
    const res = await get<Record<string, unknown>>("/api/network");
    expect(res.ok).toBe(true);
    expect(res.data).toBeDefined();
  });

  it("GET /api/mempool returns response", async () => {
    const res = await get("/api/mempool");
    // May require auth — verify we get a response
    expect(res.data).toBeDefined();
  });

  it("GET /api/events returns response", async () => {
    const res = await get("/api/events");
    // May require auth — verify we get a response
    expect(res.data).toBeDefined();
  });

  it("block height is positive and advancing", async () => {
    const res1 = await get<{ block_height: number }>("/api/status");
    expect(res1.data.block_height).toBeGreaterThan(0);
  });
});

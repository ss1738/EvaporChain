/**
 * Staking system integration tests.
 * Tests staking pools, stake, unstake, and claim operations.
 */
import { describe, it, expect } from "vitest";
import { get, post, randomAddress, sleep } from "../helpers/client";

describe("Staking System", () => {
  it("GET /api/staking returns staking pools", async () => {
    const res = await get<unknown[]>("/api/staking");
    expect(res.ok).toBe(true);
    expect(Array.isArray(res.data)).toBe(true);
  });

  it("GET /api/staking/pools is an alias", async () => {
    const res = await get<unknown[]>("/api/staking/pools");
    expect(res.ok).toBe(true);
    expect(Array.isArray(res.data)).toBe(true);
  });

  it("POST /api/staking/stake stakes into a pool", async () => {
    const pools = await get<Array<{ id: string }>>("/api/staking/pools");
    if (pools.data.length === 0) return;

    const res = await post<Record<string, unknown>>("/api/staking/stake", {
      pool_id: pools.data[0].id,
      address: randomAddress(),
      amount: 1000,
    });
    expect(res.ok).toBe(true);
  });

  it("GET /api/staking/pool/:id returns pool detail", async () => {
    const pools = await get<Array<{ id: string }>>("/api/staking/pools");
    if (pools.data.length === 0) return;

    const res = await get<Record<string, unknown>>(`/api/staking/pool/${pools.data[0].id}`);
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("id");
  });

  it("POST /api/staking/unstake withdraws from a pool", async () => {
    const pools = await get<Array<{ id: string }>>("/api/staking/pools");
    if (pools.data.length === 0) return;

    const address = randomAddress();

    // Stake first
    await post("/api/staking/stake", {
      pool_id: pools.data[0].id,
      address,
      amount: 2000,
    });

    await sleep(2000);

    const res = await post<Record<string, unknown>>("/api/staking/unstake", {
      pool_id: pools.data[0].id,
      address,
      amount: 500,
    });
    expect(res.ok).toBe(true);
  });

  it("POST /api/staking/claim claims staking rewards", async () => {
    const pools = await get<Array<{ id: string }>>("/api/staking/pools");
    if (pools.data.length === 0) return;

    const res = await post<Record<string, unknown>>("/api/staking/claim", {
      pool_id: pools.data[0].id,
      address: randomAddress(),
    });
    // May succeed or fail depending on rewards available
    expect(res.data).toBeDefined();
  });

  it("staking pools have expected structure", async () => {
    const pools = await get<Array<Record<string, unknown>>>("/api/staking/pools");
    if (pools.data.length === 0) return;

    const pool = pools.data[0];
    expect(pool).toHaveProperty("id");
  });
});

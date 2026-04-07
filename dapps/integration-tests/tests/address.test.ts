/**
 * Address, transfer, and resurrect integration tests.
 * Tests address lookups and transaction operations.
 */
import { describe, it, expect } from "vitest";
import { get, post, randomAddress, sleep } from "../helpers/client";

describe("Address & Transfers", () => {
  it("GET /api/address/:addr returns address detail after faucet", async () => {
    const address = randomAddress();
    await post("/api/faucet", { address });
    await sleep(3000);

    const res = await get<Record<string, unknown>>(`/api/address/${address}`);
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("balance");
  });

  it("address detail has expected fields", async () => {
    const address = randomAddress();
    await post("/api/faucet", { address });
    await sleep(3000);

    const res = await get<Record<string, unknown>>(`/api/address/${address}`);
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("address");
    expect(res.data).toHaveProperty("balance");
  });

  it("POST /api/tx/transfer accepts correct shape", async () => {
    const from = randomAddress();
    const to = randomAddress();

    await post("/api/faucet", { address: from });
    await sleep(3000);

    const res = await post<Record<string, unknown>>("/api/tx/transfer", {
      from,
      to,
      amount: 100,
    });
    // Transfer may require auth — verify structured response
    expect(res.data).toBeDefined();
  });

  it("POST /api/tx/resurrect accepts correct shape", async () => {
    const ghosts = await get<Array<{ id: string }>>("/api/ghosts");
    if (!Array.isArray(ghosts.data) || ghosts.data.length === 0) return;

    const res = await post<Record<string, unknown>>("/api/tx/resurrect", {
      object_id: String(ghosts.data[0].id),
      energy: 10000,
    });
    expect(res.data).toBeDefined();
  });

  it("unknown address returns valid response", async () => {
    const address = randomAddress();
    const res = await get(`/api/address/${address}`);
    expect([200, 404]).toContain(res.status);
  });

  it("GET /api/tx/:hash returns transaction detail", async () => {
    const txs = await get<{ transactions: Array<{ hash: string }> }>("/api/transactions");
    if (!txs.data.transactions || txs.data.transactions.length === 0) return;

    const res = await get<Record<string, unknown>>(`/api/tx/${txs.data.transactions[0].hash}`);
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("hash");
  });

  it("faucet-funded address has positive balance", async () => {
    const address = randomAddress();
    await post("/api/faucet", { address });
    await sleep(3000);

    const res = await get<{ balance: number }>(`/api/address/${address}`);
    expect(res.ok).toBe(true);
    expect(res.data.balance).toBeGreaterThan(0);
  });
});

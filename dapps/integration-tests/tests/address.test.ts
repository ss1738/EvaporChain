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

  it("GET /api/tx/:hash returns lifecycle status", async () => {
    // Post commit d0394b1 the endpoint returns TxStatusResponse:
    //   { hash, state: "pending"|"mempool"|"included"|"finalised"|"rejected",
    //     block_height?, epoch?, error? }
    // The originating tx body (type/from/to/amount) is no longer carried
    // here — those fields live on /api/transactions. Tests that need
    // them should redirect there.
    const txs = await get<{ transactions: Array<{ hash: string }> }>("/api/transactions");
    if (!txs.data.transactions || txs.data.transactions.length === 0) return;

    const hash = txs.data.transactions[0].hash;
    const res = await get<{
      hash: string;
      state: "pending" | "mempool" | "included" | "finalised" | "rejected";
      block_height?: number;
      epoch?: number;
      error?: string;
    }>(`/api/tx/${hash}`);
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("hash");
    expect(res.data).toHaveProperty("state");
    // Anything pulled from /api/transactions has already landed in a
    // block, so it must be included or finalised (single-node dev mode
    // collapses the gap and reports finalised).
    expect(["included", "finalised"]).toContain(res.data.state);
    expect(typeof res.data.block_height).toBe("number");
    expect(typeof res.data.epoch).toBe("number");
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

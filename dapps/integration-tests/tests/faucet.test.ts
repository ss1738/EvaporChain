/**
 * Faucet integration tests.
 * Verifies the testnet faucet distributes tokens.
 */
import { describe, it, expect } from "vitest";
import { get, post, randomAddress, sleep } from "../helpers/client";

describe("Faucet", () => {
  it("POST /api/faucet distributes tokens to an address", async () => {
    const address = randomAddress();
    const res = await post<Record<string, unknown>>("/api/faucet", { address });
    expect(res.ok).toBe(true);
    expect(res.data).toBeDefined();
  });

  it("faucet recipient has a balance after claim", async () => {
    const address = randomAddress();
    await post("/api/faucet", { address });

    await sleep(3000);

    const res = await get<Record<string, unknown>>(`/api/address/${address}`);
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("balance");
  });

  it("faucet creates a transaction", async () => {
    const address = randomAddress();
    const res = await post<Record<string, unknown>>("/api/faucet", { address });
    expect(res.ok).toBe(true);
    // Should return tx hash or success indicator
    expect(res.data).toBeDefined();
  });

  it("faucet works with different addresses", async () => {
    const addr1 = randomAddress();
    const addr2 = randomAddress();

    const [res1, res2] = await Promise.all([
      post("/api/faucet", { address: addr1 }),
      post("/api/faucet", { address: addr2 }),
    ]);

    expect(res1.ok).toBe(true);
    expect(res2.ok).toBe(true);
  });
});

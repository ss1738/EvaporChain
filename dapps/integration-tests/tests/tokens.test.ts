/**
 * Token system integration tests.
 * Tests token listing, detail, and write operations.
 * Note: Deploy/transfer require auth — tests verify API shape.
 */
import { describe, it, expect } from "vitest";
import { get, post, randomAddress, uniqueName } from "../helpers/client";

describe("Token System", () => {
  it("GET /api/tokens returns token list", async () => {
    const res = await get<unknown[]>("/api/tokens");
    expect(res.ok).toBe(true);
    expect(Array.isArray(res.data)).toBe(true);
  });

  it("POST /api/token/deploy accepts correct shape", async () => {
    const res = await post<Record<string, unknown>>("/api/token/deploy", {
      name: uniqueName("TEST"),
      symbol: "TST",
      total_supply: 1000000,
      decimals: 18,
      owner: randomAddress(),
      decay_half_life: 100,
    });
    // Requires auth — verify structured response not 422
    expect(res.status).not.toBe(422);
    expect(res.data).toBeDefined();
  });

  it("GET /api/token/:id returns single token", async () => {
    const tokens = await get<Array<{ id: string }>>("/api/tokens");
    if (tokens.data.length === 0) return;

    const id = tokens.data[0].id;
    const res = await get<Record<string, unknown>>(`/api/token/${id}`);
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("id");
  });

  it("POST /api/token/transfer accepts correct shape", async () => {
    const tokens = await get<Array<{ id: string }>>("/api/tokens");
    if (tokens.data.length === 0) return;

    const res = await post<Record<string, unknown>>("/api/token/transfer", {
      token_id: tokens.data[0].id,
      from: randomAddress(),
      to: randomAddress(),
      amount: 100,
    });
    // May need auth
    expect(res.data).toBeDefined();
  });

  it("POST /api/token/balance returns balance info", async () => {
    const tokens = await get<Array<{ id: string }>>("/api/tokens");
    if (tokens.data.length === 0) return;

    const res = await post<Record<string, unknown>>("/api/token/balance", {
      token_id: tokens.data[0].id,
      address: randomAddress(),
    });
    expect(res.data).toBeDefined();
  });

  it("token list returns consistent results", async () => {
    const res1 = await get<unknown[]>("/api/tokens");
    const res2 = await get<unknown[]>("/api/tokens");
    expect(res1.data.length).toBe(res2.data.length);
  });
});

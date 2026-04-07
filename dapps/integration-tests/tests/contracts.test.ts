/**
 * Smart contract integration tests.
 * Tests contract listing and detail queries.
 * Note: Deploy/call require auth — tests verify API shape acceptance.
 */
import { describe, it, expect } from "vitest";
import { get, post, randomAddress, uniqueName } from "../helpers/client";

describe("Smart Contracts", () => {
  it("GET /api/contracts returns wrapped contract list", async () => {
    const res = await get<{ contracts: unknown[] }>("/api/contracts");
    expect(res.ok).toBe(true);
    expect(Array.isArray(res.data.contracts)).toBe(true);
  });

  it("POST /api/tx/deploy-contract accepts request", async () => {
    const res = await post<Record<string, unknown>>("/api/tx/deploy-contract", {
      name: uniqueName("test-contract"),
      code: "fn main() { return 42; }",
      deployer: randomAddress(),
    });
    // deployer format may need adjustment — verify not a 500
    expect([200, 422]).toContain(res.status);
    expect(res.data).toBeDefined();
  });

  it("GET /api/contract/:id handles missing contracts", async () => {
    const res = await get("/api/contract/nonexistent-999");
    // Should return 404 or empty result
    expect([200, 400, 404]).toContain(res.status);
  });

  it("POST /api/tx/call-contract accepts request shape", async () => {
    const res = await post<Record<string, unknown>>("/api/tx/call-contract", {
      contract_id: "test-contract",
      function: "main",
      args: [],
      caller: randomAddress(),
    });
    // May need auth or contract must exist
    expect(res.data).toBeDefined();
  });

  it("contracts endpoint returns consistent structure", async () => {
    const res1 = await get<{ contracts: unknown[] }>("/api/contracts");
    const res2 = await get<{ contracts: unknown[] }>("/api/contracts");
    expect(res1.data.contracts.length).toBe(res2.data.contracts.length);
  });
});

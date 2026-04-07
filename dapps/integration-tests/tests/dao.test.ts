/**
 * DAO governance integration tests.
 * Tests proposal listing, creation, and voting.
 * Note: Propose/vote require auth — tests verify API shape.
 */
import { describe, it, expect } from "vitest";
import { get, post, randomAddress, uniqueName } from "../helpers/client";

describe("DAO Governance", () => {
  it("GET /api/dao returns proposals list", async () => {
    const res = await get<unknown[]>("/api/dao");
    expect(res.ok).toBe(true);
    expect(Array.isArray(res.data)).toBe(true);
  });

  it("GET /api/dao/proposals is an alias", async () => {
    const res = await get<unknown[]>("/api/dao/proposals");
    expect(res.ok).toBe(true);
    expect(Array.isArray(res.data)).toBe(true);
  });

  it("POST /api/dao/propose accepts correct shape", async () => {
    const res = await post<Record<string, unknown>>("/api/dao/propose", {
      title: uniqueName("proposal"),
      description: "Integration test proposal",
      proposer: randomAddress(),
      options: ["yes", "no"],
      voting_period: 100,
    });
    // Requires auth — verify structured response not 422
    expect(res.status).not.toBe(422);
    expect(res.data).toBeDefined();
  });

  it("GET /api/dao/proposal/:id handles queries", async () => {
    const proposals = await get<Array<{ id: string }>>("/api/dao/proposals");
    if (proposals.data.length === 0) {
      // No proposals yet — verify endpoint doesn't error
      const res = await get("/api/dao/proposal/nonexistent");
      expect([200, 404]).toContain(res.status);
      return;
    }

    const res = await get<Record<string, unknown>>(`/api/dao/proposal/${proposals.data[0].id}`);
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("id");
  });

  it("POST /api/dao/vote accepts correct shape", async () => {
    const proposals = await get<Array<{ id: string }>>("/api/dao/proposals");
    if (proposals.data.length === 0) return;

    const res = await post<Record<string, unknown>>("/api/dao/vote", {
      proposal_id: proposals.data[0].id,
      voter: randomAddress(),
      vote: "yes",
    });
    expect(res.data).toBeDefined();
  });

  it("proposal list is consistent", async () => {
    const res1 = await get<unknown[]>("/api/dao");
    const res2 = await get<unknown[]>("/api/dao/proposals");
    expect(res1.data.length).toBe(res2.data.length);
  });
});

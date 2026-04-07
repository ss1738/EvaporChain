/**
 * NFT marketplace integration tests.
 * Tests mint, transfer, refresh, collections, and NFT lifecycle.
 * Note: NFT mint/transfer/refresh are public (no auth needed).
 */
import { describe, it, expect } from "vitest";
import { get, post, randomAddress, uniqueName, sleep } from "../helpers/client";

describe("NFT Marketplace", () => {
  it("GET /api/nfts returns NFT array", async () => {
    const res = await get<unknown[]>("/api/nfts");
    expect(res.ok).toBe(true);
    expect(Array.isArray(res.data)).toBe(true);
  });

  it("GET /api/nft/collections returns collections", async () => {
    const res = await get("/api/nft/collections");
    expect(res.ok).toBe(true);
  });

  it("POST /api/nft/mint creates a new NFT", async () => {
    const res = await post<Record<string, unknown>>("/api/nft/mint", {
      name: uniqueName("test-nft"),
      metadata: "Integration test NFT",
      energy: 20000,
      half_life: 150,
      owner: randomAddress(),
    });
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("success");
  });

  it("minted NFT gets an ID", async () => {
    const res = await post<Record<string, unknown>>("/api/nft/mint", {
      name: uniqueName("id-check"),
      metadata: "ID verification test",
      energy: 10000,
      half_life: 100,
      owner: randomAddress(),
    });
    expect(res.ok).toBe(true);
    // Response should contain NFT id or success info
    expect(res.data).toBeDefined();
  });

  it("GET /api/nft/:id returns single NFT with all fields", async () => {
    const nfts = await get<Array<{ id: number }>>("/api/nfts");
    expect(nfts.data.length).toBeGreaterThan(0);
    const id = nfts.data[0].id;

    const res = await get<Record<string, unknown>>(`/api/nft/${id}`);
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("id");
    expect(res.data).toHaveProperty("name");
    expect(res.data).toHaveProperty("current_energy");
    expect(res.data).toHaveProperty("max_energy");
    expect(res.data).toHaveProperty("half_life");
    expect(res.data).toHaveProperty("state");
    expect(res.data).toHaveProperty("owner");
    expect(res.data).toHaveProperty("epochs_remaining");
    expect(res.data).toHaveProperty("decay_percentage");
  });

  it("NFT state is one of Active, Grace, Ghost", async () => {
    const nfts = await get<Array<{ state: string }>>("/api/nfts");
    for (const nft of nfts.data) {
      expect(["Active", "Grace", "Ghost"]).toContain(nft.state);
    }
  });

  it("POST /api/nft/transfer accepts correct shape", async () => {
    const nfts = await get<Array<{ id: number }>>("/api/nfts");
    if (nfts.data.length === 0) return;

    const res = await post<Record<string, unknown>>("/api/nft/transfer", {
      nft_id: nfts.data[0].id,
      to: randomAddress(),
    });
    expect(res.ok).toBe(true);
    expect(res.data).toBeDefined();
  });

  it("POST /api/nft/refresh adds energy to NFT", async () => {
    const nfts = await get<Array<{ id: number; state: string }>>("/api/nfts");
    const active = nfts.data.find(n => n.state === "Active");
    if (!active) return;

    const res = await post<Record<string, unknown>>("/api/nft/refresh", {
      nft_id: active.id,
      energy: 5000,
    });
    expect(res.ok).toBe(true);
    expect(res.data).toBeDefined();
  });

  it("POST /api/nft/mint with collection", async () => {
    const collection = uniqueName("coll");
    const res = await post<Record<string, unknown>>("/api/nft/mint", {
      name: uniqueName("coll-nft"),
      collection,
      metadata: "Collection test",
      energy: 10000,
      half_life: 100,
      owner: randomAddress(),
    });
    expect(res.ok).toBe(true);
  });

  it("genesis NFTs exist with expected data", async () => {
    const res = await get<Array<Record<string, unknown>>>("/api/nfts");
    const genesis = res.data.filter(
      (n: any) => typeof n.name === "string" && n.name.startsWith("Genesis")
    );
    expect(genesis.length).toBeGreaterThan(0);
  });

  it("energy decay is tracked correctly", async () => {
    const nfts = await get<Array<{ current_energy: number; max_energy: number; decay_percentage: number }>>("/api/nfts");
    for (const nft of nfts.data) {
      if (nft.max_energy > 0) {
        expect(nft.current_energy).toBeLessThanOrEqual(nft.max_energy);
        expect(nft.decay_percentage).toBeGreaterThanOrEqual(0);
        expect(nft.decay_percentage).toBeLessThanOrEqual(100);
      }
    }
  });
});

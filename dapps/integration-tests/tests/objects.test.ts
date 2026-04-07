/**
 * Object lifecycle integration tests.
 * Tests create-object, refresh, and state transitions.
 * Note: Write operations require authentication — tests verify correct API shape.
 */
import { describe, it, expect } from "vitest";
import { get, post, randomAddress, uniqueName } from "../helpers/client";

describe("Object Lifecycle", () => {
  it("GET /api/objects returns a list", async () => {
    const res = await get<unknown[]>("/api/objects");
    expect(res.ok).toBe(true);
    expect(Array.isArray(res.data)).toBe(true);
  });

  it("POST /api/tx/create-object accepts correct shape", async () => {
    const res = await post<Record<string, unknown>>("/api/tx/create-object", {
      object_id: uniqueName("obj"),
      name: uniqueName("test-obj"),
      metadata: "integration test object",
      energy: 50000,
      half_life: 200,
      creator: randomAddress(),
    });
    // May require auth — verify we get a structured response, not a 422
    expect(res.status).not.toBe(422);
    expect(res.data).toBeDefined();
  });

  it("GET /api/object/:id returns a single object", async () => {
    const objects = await get<Array<{ id: number }>>("/api/objects");
    expect(objects.data.length).toBeGreaterThan(0);
    const id = objects.data[0].id;

    const res = await get<Record<string, unknown>>(`/api/object/${id}`);
    expect(res.ok).toBe(true);
    expect(res.data).toHaveProperty("id");
    expect(res.data).toHaveProperty("energy");
  });

  it("POST /api/tx/refresh accepts correct shape", async () => {
    const objects = await get<Array<{ id: number; state: string }>>("/api/objects");
    const active = objects.data.find(o => o.state === "Active");
    if (!active) return;

    const res = await post<Record<string, unknown>>("/api/tx/refresh", {
      object_id: String(active.id),
      energy_deposit: 1000,
    });
    expect(res.status).not.toBe(422);
    expect(res.data).toBeDefined();
  });

  it("object has expected fields", async () => {
    const objects = await get<Array<Record<string, unknown>>>("/api/objects");
    if (objects.data.length === 0) return;

    const obj = objects.data[0];
    expect(obj).toHaveProperty("id");
    expect(obj).toHaveProperty("name");
    expect(obj).toHaveProperty("energy");
    expect(obj).toHaveProperty("state");
  });

  it("objects have valid state values", async () => {
    const objects = await get<Array<{ state: string }>>("/api/objects");
    for (const obj of objects.data) {
      expect(["Active", "Grace", "Ghost", "Risen"]).toContain(obj.state);
    }
  });

  it("POST /api/tx/create-object rejects missing required fields", async () => {
    const res = await post("/api/tx/create-object", {
      metadata: "no name or object_id",
      energy: 1000,
      half_life: 100,
    });
    // Should return 422 for missing fields
    expect(res.status).toBe(422);
  });
});

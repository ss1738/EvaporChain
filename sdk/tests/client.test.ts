import { describe, it, beforeEach, mock } from "node:test";
import assert from "node:assert/strict";
import { EvaporChain, EvaporChainError } from "../src/client";
import type {
  ChainStatus,
  Account,
  StateObject,
  Ghost,
  Block,
  TxResult,
  FaucetResult,
  Contract,
} from "../src/types";

// ── Mock fetch ──

const mockResponses = new Map<string, { status: number; body: unknown }>();

function setMock(pathPattern: string, body: unknown, status = 200) {
  mockResponses.set(pathPattern, { status, body });
}

function findMock(url: string): { status: number; body: unknown } | undefined {
  for (const [pattern, val] of mockResponses) {
    if (url.includes(pattern)) return val;
  }
  return undefined;
}

// Patch global fetch
const originalFetch = globalThis.fetch;
globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
  const url = typeof input === "string" ? input : input.toString();
  const matched = findMock(url);
  if (!matched) {
    return { ok: false, status: 404, statusText: "Not Found", text: async () => "Not found", json: async () => ({}) } as Response;
  }
  return {
    ok: matched.status >= 200 && matched.status < 300,
    status: matched.status,
    statusText: matched.status === 200 ? "OK" : "Error",
    text: async () => JSON.stringify(matched.body),
    json: async () => matched.body,
    headers: new Headers(),
  } as unknown as Response;
}) as typeof fetch;

// ── Test data ──

const MOCK_STATUS: ChainStatus = {
  chain_name: "EvaporChain",
  version: "0.2.0",
  block_height: 42,
  epoch: 42,
  active_objects: 5,
  ghost_count: 3,
  total_evaporated: 3,
  peer_count: 0,
  state_root: "abc123",
  proving_enabled: false,
  uptime_seconds: 3600,
};

const MOCK_ACCOUNTS: Account[] = [
  { address: "7f" + "00".repeat(31), name: "0x7f0000…0000", balance: 500000, nonce: 0 },
  { address: "2b" + "00".repeat(31), name: "0x2b0000…0000", balance: 250000, nonce: 0 },
];

const MOCK_OBJECTS: StateObject[] = [
  {
    id: "10" + "00".repeat(31),
    name: "token:evap-governance",
    owner: "7f" + "00".repeat(31),
    owner_name: "0x7f0000…0000",
    energy: 50000,
    max_energy: 50000,
    half_life: 200,
    state: "Active",
    created_epoch: 0,
    last_refreshed: 0,
    grace_epoch: null,
    current_energy: 45000,
    decay_percentage: 10.0,
  },
  {
    id: "15" + "00".repeat(31),
    name: "session:auth-0x1a",
    owner: "e8" + "00".repeat(31),
    owner_name: "0xe80000…0000",
    energy: 80,
    max_energy: 80,
    half_life: 4,
    state: "Grace",
    created_epoch: 0,
    last_refreshed: 0,
    grace_epoch: 30,
    current_energy: 0,
    decay_percentage: 100.0,
  },
];

const MOCK_GHOSTS: Ghost[] = [
  { id: "17" + "00".repeat(31), original_owner: "e8" + "00".repeat(31), evaporated_epoch: 25, data_hash: "aabb" },
];

const MOCK_BLOCKS: Block[] = [
  {
    number: 42,
    epoch: 42,
    parent_hash: "dead",
    state_root: "beef",
    tx_count: 2,
    evaporations: 1,
    entered_grace: 0,
    timestamp: 1700000000,
    active_objects: 5,
    ghost_count: 3,
    transactions: [{ type: "transfer", detail: "0x7f -> 0x2b amount=100" }],
  },
];

const MOCK_TX_RESULT: TxResult = { success: true, message: "Transfer queued" };

const MOCK_FAUCET: FaucetResult = { success: true, balance: 10000 };

const MOCK_CONTRACTS = {
  contracts: [
    { id: 1, template: "DecayingToken", creator: "0x7f0000…0000", energy: 5000, half_life: 100, created_epoch: 5, evaporated: false },
  ],
};

// ── Tests ──

describe("EvaporChain SDK", () => {
  let chain: EvaporChain;

  beforeEach(() => {
    mockResponses.clear();
    chain = new EvaporChain("http://localhost:3000");
  });

  it("getStatus returns valid ChainStatus", async () => {
    setMock("/api/status", MOCK_STATUS);
    const status = await chain.getStatus();
    assert.equal(status.chain_name, "EvaporChain");
    assert.equal(status.block_height, 42);
    assert.equal(status.active_objects, 5);
    assert.equal(typeof status.uptime_seconds, "number");
  });

  it("getAccounts returns array of accounts", async () => {
    setMock("/api/accounts", MOCK_ACCOUNTS);
    const accounts = await chain.getAccounts();
    assert.ok(Array.isArray(accounts));
    assert.equal(accounts.length, 2);
    assert.equal(accounts[0].balance, 500000);
    assert.ok(accounts[0].address.length > 0);
  });

  it("getObjects returns array with energy fields", async () => {
    setMock("/api/objects", MOCK_OBJECTS);
    const objects = await chain.getObjects();
    assert.ok(Array.isArray(objects));
    assert.equal(objects.length, 2);
    assert.equal(objects[0].name, "token:evap-governance");
    assert.equal(objects[0].energy, 50000);
    assert.equal(objects[0].half_life, 200);
    assert.equal(typeof objects[0].decay_percentage, "number");
    assert.equal(typeof objects[0].current_energy, "number");
  });

  it("getObject returns single object by ID", async () => {
    const objId = "10" + "00".repeat(31);
    setMock(`/api/object/${objId}`, MOCK_OBJECTS[0]);
    const obj = await chain.getObject(objId);
    assert.equal(obj.name, "token:evap-governance");
    assert.equal(obj.state, "Active");
  });

  it("getGhosts returns array of ghosts", async () => {
    setMock("/api/ghosts", MOCK_GHOSTS);
    const ghosts = await chain.getGhosts();
    assert.ok(Array.isArray(ghosts));
    assert.equal(ghosts.length, 1);
    assert.equal(ghosts[0].evaporated_epoch, 25);
    assert.ok(ghosts[0].data_hash.length > 0);
  });

  it("getBlocks returns recent blocks", async () => {
    setMock("/api/blocks", MOCK_BLOCKS);
    const blocks = await chain.getBlocks(10);
    assert.ok(Array.isArray(blocks));
    assert.equal(blocks[0].number, 42);
    assert.equal(blocks[0].evaporations, 1);
  });

  it("getBlock returns specific block by height", async () => {
    setMock("/api/block/42", MOCK_BLOCKS[0]);
    const block = await chain.getBlock(42);
    assert.equal(block.number, 42);
    assert.equal(block.epoch, 42);
  });

  it("transfer sends successfully", async () => {
    setMock("/api/tx/transfer", MOCK_TX_RESULT);
    const result = await chain.transfer(0x7f, 0x2b, 1000);
    assert.equal(result.success, true);
    assert.ok(result.message.length > 0);
  });

  it("createObject creates with energy and half-life", async () => {
    setMock("/api/tx/create-object", { success: true, message: "CreateObject queued" });
    const result = await chain.createObject(0x7f, 0x30, 5000, 10);
    assert.equal(result.success, true);
  });

  it("refreshObject sends refresh tx", async () => {
    setMock("/api/tx/refresh", { success: true, message: "Refresh queued" });
    const result = await chain.refreshObject(0x10, 500);
    assert.equal(result.success, true);
  });

  it("resurrectObject sends resurrect tx", async () => {
    setMock("/api/tx/resurrect", { success: true, message: "Resurrect queued" });
    const result = await chain.resurrectObject(0x17, 1000);
    assert.equal(result.success, true);
  });

  it("claimFaucet returns balance", async () => {
    setMock("/api/faucet", MOCK_FAUCET);
    const result = await chain.claimFaucet(0xFF);
    assert.equal(result.success, true);
    assert.equal(result.balance, 10000);
  });

  it("getContracts returns contract list", async () => {
    setMock("/api/contracts", MOCK_CONTRACTS);
    const contracts = await chain.getContracts();
    assert.ok(Array.isArray(contracts));
    assert.equal(contracts[0].template, "DecayingToken");
  });

  it("deployContract submits deploy tx", async () => {
    setMock("/api/tx/deploy-contract", { success: true, message: "Deploy queued" });
    const result = await chain.deployContract(0x7f, "DecayingToken", { name: "Test" }, 5000, 100);
    assert.equal(result.success, true);
  });

  it("callContract submits call tx", async () => {
    setMock("/api/tx/call-contract", { success: true, message: "Call queued" });
    const result = await chain.callContract(0x7f, 1, "transfer", { to: "0x2b", amount: 100 }, 42);
    assert.equal(result.success, true);
  });

  it("getEvents returns event list", async () => {
    setMock("/api/events", { events: [{ epoch: 40, event_type: "evaporated", message: "Object evaporated", timestamp_ms: 170000 }] });
    const events = await chain.getEvents(10);
    assert.ok(Array.isArray(events));
    assert.equal(events[0].event_type, "evaporated");
  });

  it("getEnergyDecayEstimate calculates correctly", async () => {
    const objId = "10" + "00".repeat(31);
    setMock(`/api/object/${objId}`, MOCK_OBJECTS[0]);
    setMock("/api/status", MOCK_STATUS);

    const estimate = await chain.getEnergyDecayEstimate(objId);
    assert.equal(estimate.max_energy, 50000);
    assert.equal(estimate.half_life, 200);
    assert.equal(typeof estimate.estimated_epochs_remaining, "number");
    assert.ok(estimate.estimated_epochs_remaining > 0);
    assert.ok(estimate.will_enter_grace_at > 0);
    assert.ok(estimate.will_evaporate_at > estimate.will_enter_grace_at);
  });

  it("handles HTTP errors with EvaporChainError", async () => {
    setMock("/api/object/nonexistent", { error: "not found" }, 404);
    try {
      await chain.getObject("nonexistent");
      assert.fail("Should have thrown");
    } catch (err) {
      assert.ok(err instanceof EvaporChainError);
      assert.equal((err as EvaporChainError).status, 404);
    }
  });

  it("constructor accepts options object", () => {
    const client = new EvaporChain({ baseUrl: "http://custom:9999", timeout: 5000 });
    // Just verify it constructs without error
    assert.ok(client);
  });

  it("constructor uses default URL", () => {
    const client = new EvaporChain();
    assert.ok(client);
  });

  it("watchObject returns stop function", async () => {
    const objId = "10" + "00".repeat(31);
    setMock(`/api/object/${objId}`, MOCK_OBJECTS[0]);

    let callCount = 0;
    const stop = chain.watchObject(objId, () => { callCount++; }, 50);

    await new Promise((r) => setTimeout(r, 200));
    stop();
    const countAfterStop = callCount;
    assert.ok(callCount >= 1, `Expected at least 1 callback, got ${callCount}`);

    // Verify it stopped
    await new Promise((r) => setTimeout(r, 150));
    assert.ok(callCount - countAfterStop <= 1, "Watch should have stopped");
  });

  it("getStatsSummary returns stats", async () => {
    setMock("/api/stats/summary", { total_created: 10, total_evaporated: 3, total_resurrected: 1, total_refreshed: 5, avg_lifetime_epochs: 12.5, total_transactions: 50 });
    const stats = await chain.getStatsSummary();
    assert.equal(stats.total_created, 10);
    assert.equal(stats.total_evaporated, 3);
  });

  it("getNetwork returns peer count", async () => {
    setMock("/api/network", { peer_count: 0 });
    const net = await chain.getNetwork();
    assert.equal(net.peer_count, 0);
  });
});

// ── WebSocket Subscription Tests ──

describe("WebSocket subscriptions", () => {
  let chain: EvaporChain;

  beforeEach(() => {
    mockResponses.clear();
    chain = new EvaporChain("http://localhost:9944");
  });

  it("connected property is false before subscribe", () => {
    assert.equal(chain.connected, false);
  });

  it("on/off/once register and remove listeners without error", () => {
    const handler = () => {};
    chain.on("new_block", handler);
    chain.off("new_block", handler);
    chain.once("evaporation", handler);
  });

  it("unsubscribe is safe when not connected", () => {
    chain.unsubscribe();
    assert.equal(chain.connected, false);
  });

  it("constructor respects wsReconnectDelay and wsMaxReconnects", () => {
    const custom = new EvaporChain({
      baseUrl: "http://localhost:9944",
      wsReconnectDelay: 5000,
      wsMaxReconnects: 3,
    });
    assert.equal(custom.connected, false);
    custom.unsubscribe();
  });

  it("returns this from on/off/once for chaining", () => {
    const handler = () => {};
    const result = chain.on("new_block", handler).on("evaporation", handler).off("new_block", handler);
    assert.ok(result instanceof EvaporChain);
  });
});

// Restore fetch (not strictly needed for test, but clean)
process.on("exit", () => {
  globalThis.fetch = originalFetch;
});

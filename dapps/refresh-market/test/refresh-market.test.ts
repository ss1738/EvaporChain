import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  armPayload,
  claimSlotPayload,
  refreshSlotPayload,
  releaseSlotPayload,
  evictPayload,
  currentRatePayload,
  rateAtUsedPayload,
  isHolderPayload,
  isEvictablePayload,
  slotsRemainingPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { REFRESH_MARKET_SOURCE } from "../src/contract.ts";
import { currentRate, rateAfterOneMoreClaim, firstUsedAboveRate } from "../src/rate.ts";

test("deployPayload carries the contract source + params", () => {
  const p = deployPayload({ deployer: 1, energy: 1000, halfLife: 100 });
  assert.equal(p.source_code, REFRESH_MARKET_SOURCE);
  assert.equal(p.energy, 1000);
  assert.equal(p.half_life, 100);
});

test("armPayload carries cap/base/eviction as positional u64 args", () => {
  const p = armPayload({ caller: 1, contractId: 7, capacity: 10, baseRent: 100, evictionWindow: 5, epoch: 0 });
  assert.equal(p.method, "arm");
  assert.deepEqual(p.args, [10, 100, 5]);
});

test("holder methods are no-arg with correct method names", () => {
  assert.equal(claimSlotPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "claim_slot");
  assert.deepEqual(claimSlotPayload({ caller: 1, contractId: 7, epoch: 0 }).args, []);

  assert.equal(refreshSlotPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "refresh_slot");
  assert.equal(releaseSlotPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "release_slot");
});

test("evict carries the target address", () => {
  const p = evictPayload({ caller: 1, contractId: 7, whoHex: "0x21", epoch: 6 });
  assert.equal(p.method, "evict");
  assert.deepEqual(p.args, ["0x21"]);
});

test("view payloads have correct method names + args shape", () => {
  assert.equal(currentRatePayload({ caller: 1, contractId: 7, epoch: 0 }).method, "current_rate");
  assert.deepEqual(currentRatePayload({ caller: 1, contractId: 7, epoch: 0 }).args, []);

  const r = rateAtUsedPayload({ caller: 1, contractId: 7, usedHypothetical: 5, epoch: 0 });
  assert.equal(r.method, "rate_at_used");
  assert.deepEqual(r.args, [5]);

  const ih = isHolderPayload({ caller: 1, contractId: 7, whoHex: "0x21", epoch: 0 });
  assert.deepEqual(ih.args, ["0x21"]);
  assert.equal(ih.method, "is_holder");

  const ie = isEvictablePayload({ caller: 1, contractId: 7, whoHex: "0x21", epoch: 6 });
  assert.equal(ie.method, "is_evictable");

  assert.equal(
    slotsRemainingPayload({ caller: 1, contractId: 7, epoch: 0 }).method,
    "slots_remaining",
  );
});

test("currentRate matches the on-chain formula at boundary points", () => {
  // base=100, capacity=10 — the on-chain pilot test's parameters.
  // rate = 100 * (used + 1)^2 / 100  = (used + 1)^2  for these inputs.
  assert.equal(currentRate(0n, 10n, 100n), 1n);
  assert.equal(currentRate(5n, 10n, 100n), 36n);
  assert.equal(currentRate(9n, 10n, 100n), 100n);
  // pre-arm safety: capacity 0 returns 0 (not crash).
  assert.equal(currentRate(5n, 0n, 100n), 0n);
});

test("rateAfterOneMoreClaim previews the next-claim rate", () => {
  assert.equal(rateAfterOneMoreClaim(0n, 10n, 100n), 4n);   // (1+1)^2
  assert.equal(rateAfterOneMoreClaim(4n, 10n, 100n), 36n);  // (5+1)^2
});

test("firstUsedAboveRate locates the threshold crossing", () => {
  // base=100, capacity=10. threshold=10 → first used where rate > 10.
  // rate(used): 1, 4, 9, 16, 25, ... → first > 10 is used=3 (rate=16).
  assert.equal(firstUsedAboveRate(100n, 10n, 10n), 3);
  // threshold=99 → rate at used=10 (cap) is (10+1)^2=121>99, so used=9 (100>99).
  assert.equal(firstUsedAboveRate(100n, 10n, 99n), 9);
  // threshold above the saturation rate (rate(cap)=121) → null.
  assert.equal(firstUsedAboveRate(100n, 10n, 1000n), null);
  // capacity 0 → null.
  assert.equal(firstUsedAboveRate(100n, 0n, 10n), null);
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("REFRESH_MARKET_SOURCE contains all methods + lifecycle hooks", () => {
  for (const name of [
    "arm",
    "claim_slot",
    "refresh_slot",
    "release_slot",
    "evict",
    "current_rate",
    "rate_at_used",
    "is_evictable",
    "on_grace",
    "on_refresh",
    "on_evaporate",
  ]) {
    assert.ok(REFRESH_MARKET_SOURCE.includes(name), `REFRESH_MARKET_SOURCE missing: ${name}`);
  }
});

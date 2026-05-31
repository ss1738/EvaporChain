import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  armPayload,
  issuePayload,
  redeemPayload,
  currentValuePayload,
  hasActiveAqPayload,
  epochsUntilExpiryPayload,
  issuedInCurrentWindowPayload,
  slotsLeftInWindowPayload,
  isArmedPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { SAP_SOURCE } from "../src/contract.ts";
import { valueAtEpoch, hasActiveAq, epochsUntilExpiry, type AqState } from "../src/value.ts";

test("deployPayload carries the contract source", () => {
  const p = deployPayload({ deployer: 1, energy: 1000000, halfLife: 100 });
  assert.equal(p.source_code, SAP_SOURCE);
});

test("arm carries (initial, hl, max_aq, window) in order", () => {
  const p = armPayload({
    caller: 1,
    contractId: 7,
    initialValue: 1000,
    halfLife: 10,
    maxAqPerWindow: 3,
    windowEpochs: 60,
    epoch: 0,
  });
  assert.equal(p.method, "arm");
  assert.deepEqual(p.args, [1000, 10, 3, 60]);
});

test("issuePayload carries the recipient address", () => {
  const p = issuePayload({ caller: 1, contractId: 7, recipientHex: "0x22", epoch: 0 });
  assert.equal(p.method, "issue");
  assert.deepEqual(p.args, ["0x22"]);
});

test("redeem + no-arg view payloads have correct method names", () => {
  assert.equal(redeemPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "redeem");
  assert.deepEqual(redeemPayload({ caller: 1, contractId: 7, epoch: 0 }).args, []);

  for (const [fn, name] of [
    [issuedInCurrentWindowPayload, "issued_in_current_window"],
    [slotsLeftInWindowPayload, "slots_left_in_window"],
    [isArmedPayload, "is_armed"],
  ] as const) {
    assert.equal(fn({ caller: 1, contractId: 7, epoch: 0 }).method, name);
  }
});

test("address-arg view payloads carry the address", () => {
  for (const [fn, name] of [
    [currentValuePayload, "current_value"],
    [hasActiveAqPayload, "has_active_aq"],
    [epochsUntilExpiryPayload, "epochs_until_expiry"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 7, whoHex: "0x22", epoch: 0 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, ["0x22"]);
  }
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("valueAtEpoch: V2 exact-exponential math (initial=1000, hl=10)", () => {
  const s: AqState = { bornEpochShifted: 1n, redeemed: false, initialValue: 1000n, halfLife: 10n };
  // V2: value halves every `halfLife` epochs via `>>`.
  // age=0: 1000 (no halvings).
  assert.equal(valueAtEpoch(s, 0n), 1000n);
  // age=9: still in first half-life window — 1000.
  assert.equal(valueAtEpoch(s, 9n), 1000n);
  // age=10 (1 halving): 500.
  assert.equal(valueAtEpoch(s, 10n), 500n);
  // age=20 (2 halvings): 250 — V1 LINEAR said 0 here; V2 says 250.
  assert.equal(valueAtEpoch(s, 20n), 250n);
  // age=99 (9 halvings): 1.
  assert.equal(valueAtEpoch(s, 99n), 1n);
  // age=100 (10 halvings): 1000 >> 10 = 0 by integer truncation.
  assert.equal(valueAtEpoch(s, 100n), 0n);
  // age=10000 (way past): still 0 (shift clamped at 64).
  assert.equal(valueAtEpoch(s, 10000n), 0n);
});

test("valueAtEpoch: never-issued + redeemed return 0", () => {
  const never: AqState = { bornEpochShifted: 0n, redeemed: false, initialValue: 1000n, halfLife: 10n };
  assert.equal(valueAtEpoch(never, 5n), 0n);

  const redeemed: AqState = { bornEpochShifted: 1n, redeemed: true, initialValue: 1000n, halfLife: 10n };
  assert.equal(valueAtEpoch(redeemed, 5n), 0n);
});

test("hasActiveAq + epochsUntilExpiry: V2 (value > 0 + 64×hl upper bound)", () => {
  const live: AqState = { bornEpochShifted: 1n, redeemed: false, initialValue: 1000n, halfLife: 10n };
  // V2: hasActiveAq = valueAtEpoch > 0. Lives until age=100 (when
  // 1000 >> 10 floors to 0).
  assert.equal(hasActiveAq(live, 99n), true);   // value=1 → active
  assert.equal(hasActiveAq(live, 100n), false); // value=0 → inactive
  // epochsUntilExpiry: 64 × half_life − age = 640 − age (over-approximate bound).
  assert.equal(epochsUntilExpiry(live, 0n), 640n);
  assert.equal(epochsUntilExpiry(live, 15n), 625n);
  assert.equal(epochsUntilExpiry(live, 640n), 0n);
});

test("SAP_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    "arm",
    "issue",
    "redeem",
    "current_value",
    "has_active_aq",
    "epochs_until_expiry",
    "max_aq_per_window",
    "issued_in_current_window",
    "on_grace",
    "on_refresh",
    "on_evaporate",
  ]) {
    assert.ok(SAP_SOURCE.includes(name), `SAP_SOURCE missing: ${name}`);
  }
});

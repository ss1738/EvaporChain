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

test("valueAtEpoch: matches contract math (initial=1000, hl=10)", () => {
  const s: AqState = { bornEpochShifted: 1n, redeemed: false, initialValue: 1000n, halfLife: 10n };
  // mint at real epoch 0 → bornEpochShifted = 1.
  // age=0: value 1000.
  assert.equal(valueAtEpoch(s, 0n), 1000n);
  // age=10 (half-life): value 500.
  assert.equal(valueAtEpoch(s, 10n), 500n);
  // age=15: value 250.
  assert.equal(valueAtEpoch(s, 15n), 250n);
  // age=20 (expiry): 0.
  assert.equal(valueAtEpoch(s, 20n), 0n);
  // age=100 (way past): 0.
  assert.equal(valueAtEpoch(s, 100n), 0n);
});

test("valueAtEpoch: never-issued + redeemed return 0", () => {
  const never: AqState = { bornEpochShifted: 0n, redeemed: false, initialValue: 1000n, halfLife: 10n };
  assert.equal(valueAtEpoch(never, 5n), 0n);

  const redeemed: AqState = { bornEpochShifted: 1n, redeemed: true, initialValue: 1000n, halfLife: 10n };
  assert.equal(valueAtEpoch(redeemed, 5n), 0n);
});

test("hasActiveAq + epochsUntilExpiry mirror current_value's gates", () => {
  const live: AqState = { bornEpochShifted: 1n, redeemed: false, initialValue: 1000n, halfLife: 10n };
  // At epoch 19 (just before expiry): active, 1 epoch left.
  assert.equal(hasActiveAq(live, 19n), true);
  assert.equal(epochsUntilExpiry(live, 19n), 1n);
  // At epoch 20 (expiry): inactive, 0 epochs left.
  assert.equal(hasActiveAq(live, 20n), false);
  assert.equal(epochsUntilExpiry(live, 20n), 0n);
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

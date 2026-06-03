import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  setTermsPayload,
  claimPayload,
  cancelPayload,
  vestedNowPayload,
  vestedAmountPayload,
  pendingAmountPayload,
  beneficiaryOfPayload,
  grantTotalPayload,
  cliffAtPayload,
  fullyVestedAtPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { VESTING_SCHEDULE_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 5, energy: 100_000, halfLife: 365 });
  assert.equal(p.deployer, 5);
  assert.equal(p.source_code, VESTING_SCHEDULE_SOURCE);
  assert.equal(p.energy, 100_000);
  assert.equal(p.half_life, 365);
});

test("set_terms carries (beneficiary, grant, cliff, duration) in canonical order", () => {
  const p = setTermsPayload({
    caller: 5,
    contractId: 42,
    beneficiaryHex: "0xfeed",
    grant: 10_000,
    cliffEpochs: 100,
    durationEpochs: 400,
    epoch: 0,
  });
  assert.equal(p.method, "set_terms");
  assert.deepEqual(p.args, ["0xfeed", 10_000, 100, 400]);
  assert.equal(p.caller, 5);
  assert.equal(p.contract_id, 42);
  assert.equal(p.epoch, 0);
});

test("claim + cancel + no-arg views have correct method names + zero args", () => {
  for (const [fn, name] of [
    [claimPayload, "claim"],
    [cancelPayload, "cancel"],
    [vestedNowPayload, "vested_now"],
    [vestedAmountPayload, "vested_amount"],
    [pendingAmountPayload, "pending_amount"],
    [beneficiaryOfPayload, "beneficiary_of"],
    [grantTotalPayload, "grant_total"],
    [cliffAtPayload, "cliff_at"],
    [fullyVestedAtPayload, "fully_vested_at"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 2, epoch: 3 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("epoch + caller + contract_id thread through every call payload", () => {
  const setT = setTermsPayload({
    caller: 99, contractId: 88, beneficiaryHex: "0x1", grant: 1, cliffEpochs: 1, durationEpochs: 2, epoch: 77,
  });
  assert.equal(setT.caller, 99);
  assert.equal(setT.contract_id, 88);
  assert.equal(setT.epoch, 77);

  const claim = claimPayload({ caller: 99, contractId: 88, epoch: 77 });
  assert.equal(claim.caller, 99);
  assert.equal(claim.contract_id, 88);
  assert.equal(claim.epoch, 77);
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("VESTING_SCHEDULE_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "grantor:",
    "beneficiary:",
    "total_grant:",
    "cliff_epochs:",
    "duration_epochs:",
    "start_epoch:",
    "sealed:",
    "claimed_amount:",
    "cancelled:",
    "vested_at_evaporate:",
    "forfeit_signaled:",
    // mutators
    "fn set_terms(",
    "fn claim()",
    "fn cancel()",
    // views
    "fn vested_now()",
    "fn vested_amount()",
    "fn pending_amount()",
    "fn beneficiary_of()",
    "fn grant_total()",
    "fn cliff_at()",
    "fn fully_vested_at()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only grantor can set terms",
    "only beneficiary can claim",
    "only grantor can cancel",
    "cliff cannot exceed duration",
    "vest immutable after first claim",
    "unclaimed remainder forfeits",
  ]) {
    assert.ok(
      VESTING_SCHEDULE_SOURCE.includes(name),
      `VESTING_SCHEDULE_SOURCE missing: ${name}`,
    );
  }
});

test("VESTING_SCHEDULE_SOURCE: cancel() blocked after first claim", () => {
  // Once the beneficiary has touched the grant, the schedule is
  // immutable — grantor can no longer cancel. Pin the literal guard;
  // a rename or check-typo would let the grantor rug-pull a partially-
  // claimed vest.
  const cancel = VESTING_SCHEDULE_SOURCE.slice(
    VESTING_SCHEDULE_SOURCE.indexOf("fn cancel()"),
    VESTING_SCHEDULE_SOURCE.indexOf("fn vested_amount()"),
  );
  assert.ok(
    cancel.includes("require(self.claimed_amount == 0,"),
    "cancel() must require claimed_amount == 0",
  );
});

test("VESTING_SCHEDULE_SOURCE: set_terms enforces cliff <= duration", () => {
  // A cliff > duration would prevent the schedule from ever vesting
  // (since `elapsed >= cliff` implies `elapsed >= duration`, the full
  // grant unlocks at the cliff in a degenerate way). Pre-flight the
  // constraint at set_terms.
  const set = VESTING_SCHEDULE_SOURCE.slice(
    VESTING_SCHEDULE_SOURCE.indexOf("fn set_terms("),
    VESTING_SCHEDULE_SOURCE.indexOf("fn vested_now()"),
  );
  assert.ok(
    set.includes("require(cliff <= duration,"),
    "set_terms must require cliff <= duration",
  );
});

test("VESTING_SCHEDULE_SOURCE: VEST-1 division-first arithmetic at all 5 sites", () => {
  // VEST-1 (audit 2026-05-17): all vesting-math sites use
  // division-first form to avoid u64 overflow at large grants.
  // Pattern: `vest_whole * elapsed + vest_rem * elapsed / duration_epochs`.
  // A regression to multiply-first (`total_grant * elapsed / duration_epochs`)
  // would silently overflow at grants > ~10^16 / duration. Pin the
  // VEST-1 marker shape across all sites.
  const expectedPattern =
    "vest_whole * elapsed + vest_rem * elapsed / self.duration_epochs";
  const occurrences = (VESTING_SCHEDULE_SOURCE.match(
    new RegExp(expectedPattern.replace(/[*+/]/g, "\\$&"), "g"),
  ) ?? []).length;
  // Sites: vested_now + claim + vested_amount + pending_amount +
  // on_evaporate = 5 occurrences of the division-first formula.
  assert.equal(
    occurrences,
    5,
    `Expected VEST-1 division-first pattern at exactly 5 sites; found ${occurrences}`,
  );
});

test("VESTING_SCHEDULE_SOURCE: on_evaporate stamps vested-at-death + flips forfeit", () => {
  // The forfeit signal lets the coordinator return the unclaimed
  // remainder to the grantor. Pin the two state mutations on the
  // evaporation path so a future refactor can't silently drop them.
  const evap = VESTING_SCHEDULE_SOURCE.slice(
    VESTING_SCHEDULE_SOURCE.indexOf("on_evaporate()"),
    VESTING_SCHEDULE_SOURCE.length,
  );
  assert.ok(
    evap.includes("self.vested_at_evaporate = vested"),
    "on_evaporate must stamp vested_at_evaporate",
  );
  assert.ok(
    evap.includes("self.forfeit_signaled = true"),
    "on_evaporate must flip forfeit_signaled",
  );
});

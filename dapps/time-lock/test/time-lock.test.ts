import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  setTermsPayload,
  claimPayload,
  revokePayload,
  beneficiaryOfPayload,
  lockedPayload,
  unlockAtPayload,
  isUnlockedPayload,
  isClaimedPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { TIME_LOCK_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 5, energy: 100_000, halfLife: 365 });
  assert.equal(p.deployer, 5);
  assert.equal(p.source_code, TIME_LOCK_SOURCE);
  assert.equal(p.energy, 100_000);
  assert.equal(p.half_life, 365);
});

test("set_terms carries (beneficiary, amount, unlock_epoch) in canonical order", () => {
  const p = setTermsPayload({
    caller: 5,
    contractId: 42,
    beneficiaryHex: "0xfeed",
    amount: 1000,
    unlockEpoch: 500,
    epoch: 100,
  });
  assert.equal(p.method, "set_terms");
  assert.deepEqual(p.args, ["0xfeed", 1000, 500]);
  assert.equal(p.caller, 5);
  assert.equal(p.contract_id, 42);
  assert.equal(p.epoch, 100);
});

test("claim + revoke + no-arg views have correct method names + zero args", () => {
  for (const [fn, name] of [
    [claimPayload, "claim"],
    [revokePayload, "revoke"],
    [beneficiaryOfPayload, "beneficiary_of"],
    [lockedPayload, "locked"],
    [unlockAtPayload, "unlock_at"],
    [isUnlockedPayload, "is_unlocked"],
    [isClaimedPayload, "is_claimed"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 2, epoch: 3 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("epoch + caller + contract_id thread through every call payload", () => {
  const setT = setTermsPayload({
    caller: 99, contractId: 88, beneficiaryHex: "0x1", amount: 1, unlockEpoch: 100, epoch: 77,
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

test("TIME_LOCK_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "grantor:",
    "beneficiary:",
    "amount:",
    "unlock_epoch:",
    "sealed:",
    "claimed:",
    "revoked:",
    "forfeit_signaled:",
    "unclaimed_at_evaporate:",
    // mutators
    "fn set_terms(",
    "fn claim()",
    "fn revoke()",
    // views
    "fn beneficiary_of()",
    "fn locked()",
    "fn unlock_at()",
    "fn is_unlocked()",
    "fn is_claimed()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only grantor can set terms",
    "only grantor can revoke",
    "only beneficiary can claim",
    "still locked",
    "cannot revoke after unlock",
    "forfeit_signaled",
  ]) {
    assert.ok(TIME_LOCK_SOURCE.includes(name), `TIME_LOCK_SOURCE missing: ${name}`);
  }
});

test("TIME_LOCK_SOURCE: set_terms rejects unlock <= current epoch", () => {
  // The contract should refuse arming with an unlock already in the
  // past (or equal to the current epoch). A pre-flight via the typed
  // client doesn't enforce this — the contract does, and it's
  // doctrinally important (a "lock" already unlocked is a degenerate
  // primitive). Pin the literal guard.
  const set = TIME_LOCK_SOURCE.slice(
    TIME_LOCK_SOURCE.indexOf("fn set_terms("),
    TIME_LOCK_SOURCE.indexOf("fn claim()"),
  );
  assert.ok(
    set.includes("require(unlock > epoch, \"unlock must be in the future\")"),
    "set_terms must require unlock > epoch (strictly future)",
  );
});

test("TIME_LOCK_SOURCE: revoke() blocked at or after unlock (irrevocable post-unlock)", () => {
  // The grantor's clawback right ends the moment the unlock epoch is
  // reached — even one block after, the beneficiary's claim window is
  // open and revoking would be a rug-pull. Pin the literal guard.
  const rev = TIME_LOCK_SOURCE.slice(
    TIME_LOCK_SOURCE.indexOf("fn revoke()"),
    TIME_LOCK_SOURCE.indexOf("fn beneficiary_of()"),
  );
  assert.ok(
    rev.includes("require(epoch < self.unlock_epoch, \"cannot revoke after unlock\")"),
    "revoke() must require epoch < unlock_epoch",
  );
});

test("TIME_LOCK_SOURCE: on_evaporate flips forfeit ONLY if !claimed && !revoked", () => {
  // The forfeit signal must distinguish three lifecycle ends:
  // 1. Claimed cleanly — beneficiary got the amount, nothing to do.
  // 2. Revoked pre-unlock — grantor reclaimed already, nothing to do.
  // 3. Never claimed, never revoked — promise lapsed; coordinator must
  //    return the amount to the grantor. forfeit_signaled = true +
  //    unclaimed_at_evaporate = self.amount.
  // The nested-if structure encodes this; pin it against rename.
  const evap = TIME_LOCK_SOURCE.slice(
    TIME_LOCK_SOURCE.indexOf("on_evaporate()"),
    TIME_LOCK_SOURCE.length,
  );
  assert.ok(
    evap.includes("if self.claimed == false"),
    "on_evaporate outer guard must check claimed == false",
  );
  assert.ok(
    evap.includes("if self.revoked == false"),
    "on_evaporate inner guard must check revoked == false",
  );
  assert.ok(
    evap.includes("self.forfeit_signaled = true"),
    "on_evaporate must flip forfeit_signaled on the unclaimed-unrevoked path",
  );
  assert.ok(
    evap.includes("self.unclaimed_at_evaporate = self.amount"),
    "on_evaporate must record unclaimed_at_evaporate for coordinator return",
  );
});

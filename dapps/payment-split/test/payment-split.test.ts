import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  addRecipientPayload,
  sealPayload,
  depositPayload,
  claimPayload,
  entitlementOfPayload,
  pendingOfPayload,
  shareOfPayload,
  totalPoolPayload,
  recipientsPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { PAYMENT_SPLIT_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 5, energy: 100_000, halfLife: 365 });
  assert.equal(p.deployer, 5);
  assert.equal(p.source_code, PAYMENT_SPLIT_SOURCE);
  assert.equal(p.energy, 100_000);
  assert.equal(p.half_life, 365);
});

test("add_recipient carries (target, bps) in canonical order", () => {
  const p = addRecipientPayload({
    caller: 5,
    contractId: 42,
    recipientHex: "0xfeed",
    bps: 5000,
    epoch: 10,
  });
  assert.equal(p.method, "add_recipient");
  assert.deepEqual(p.args, ["0xfeed", 5000]);
});

test("deposit carries the amount", () => {
  const p = depositPayload({ caller: 9, contractId: 42, amount: 10_000, epoch: 50 });
  assert.equal(p.method, "deposit");
  assert.deepEqual(p.args, [10_000]);
});

test("address-arg views carry the queried address", () => {
  for (const [fn, name] of [
    [entitlementOfPayload, "entitlement_of"],
    [pendingOfPayload, "pending_of"],
    [shareOfPayload, "share_of"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 42, whoHex: "0xcd", epoch: 0 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, ["0xcd"]);
  }
});

test("seal + claim + no-arg views have correct method names + zero args", () => {
  for (const [fn, name] of [
    [sealPayload, "seal"],
    [claimPayload, "claim"],
    [totalPoolPayload, "total_pool"],
    [recipientsPayload, "recipients"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 2, epoch: 3 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("epoch + caller + contract_id thread through every call payload", () => {
  const cases = [
    addRecipientPayload({ caller: 99, contractId: 88, recipientHex: "0x1", bps: 100, epoch: 77 }),
    sealPayload({ caller: 99, contractId: 88, epoch: 77 }),
    depositPayload({ caller: 99, contractId: 88, amount: 100, epoch: 77 }),
    claimPayload({ caller: 99, contractId: 88, epoch: 77 }),
  ];
  for (const p of cases) {
    assert.equal(p.caller, 99);
    assert.equal(p.contract_id, 88);
    assert.equal(p.epoch, 77);
  }
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("PAYMENT_SPLIT_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "shares:",
    "claimed:",
    "total_bps:",
    "recipient_count:",
    "total_deposited:",
    "sealed:",
    "forfeit_signaled:",
    "unclaimed_at_evaporate:",
    // mutators
    "fn add_recipient(",
    "fn seal()",
    "fn deposit(",
    "fn claim()",
    // views
    "fn entitlement_of(",
    "fn pending_of(",
    "fn share_of(",
    "fn total_pool()",
    "fn recipients()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only owner can add recipients",
    "only owner can seal",
    "not a recipient",
    "total bps would exceed 10000",
    "total bps must equal 10000",
    "nothing to claim",
  ]) {
    assert.ok(
      PAYMENT_SPLIT_SOURCE.includes(name),
      `PAYMENT_SPLIT_SOURCE missing: ${name}`,
    );
  }
});

test("PAYMENT_SPLIT_SOURCE: seal() requires total_bps == 10000 exactly", () => {
  // The 100%-allocation invariant is what makes the split deterministic.
  // Under-allocation (e.g., total_bps = 9000) would leave 10% of every
  // deposit permanently un-claimable. Over-allocation can't happen because
  // add_recipient caps at 10_000. Pin the literal seal-time check.
  const seal = PAYMENT_SPLIT_SOURCE.slice(
    PAYMENT_SPLIT_SOURCE.indexOf("fn seal()"),
    PAYMENT_SPLIT_SOURCE.indexOf("fn deposit("),
  );
  assert.ok(
    seal.includes("require(self.total_bps == 10000,"),
    "seal() must require total_bps == 10000 exactly",
  );
});

test("PAYMENT_SPLIT_SOURCE: SPLIT-1 division-first formula at all 3 sites", () => {
  // SPLIT-1 (audit 2026-05-17): claim() / entitlement_of() / pending_of()
  // all use division-first to avoid u64 overflow at total_deposited >
  // u64::MAX/bps (~1.8e15 at bps=10000). A regression to the multiply-first
  // form `total * bps / 10000` would silently overflow at large pools,
  // bricking all claims. Pin the pattern appears at all 3 sites.
  const pattern = "whole * bps + rem * bps / 10000";
  const occurrences = (PAYMENT_SPLIT_SOURCE.match(
    new RegExp(pattern.replace(/[*+/]/g, "\\$&"), "g"),
  ) ?? []).length;
  assert.equal(
    occurrences,
    3,
    `Expected SPLIT-1 division-first pattern at exactly 3 sites (claim + entitlement_of + pending_of); found ${occurrences}`,
  );
});

test("PAYMENT_SPLIT_SOURCE: on_evaporate stamps unclaimed pool + flips forfeit", () => {
  // The forfeit signal lets the coordinator return the unclaimed pool
  // residue to the deployer at evaporation. Pin both state mutations.
  const evap = PAYMENT_SPLIT_SOURCE.slice(
    PAYMENT_SPLIT_SOURCE.indexOf("on_evaporate()"),
    PAYMENT_SPLIT_SOURCE.length,
  );
  assert.ok(
    evap.includes("self.unclaimed_at_evaporate = self.total_deposited"),
    "on_evaporate must stamp unclaimed_at_evaporate from total_deposited",
  );
  assert.ok(
    evap.includes("self.forfeit_signaled = true"),
    "on_evaporate must flip forfeit_signaled",
  );
});

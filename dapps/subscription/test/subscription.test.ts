import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  setTermsPayload,
  payPayload,
  cancelPayload,
  providerOfPayload,
  subscriberOfPayload,
  amountPerPeriodPayload,
  periodLengthPayload,
  periodsPaidPayload,
  totalPaidPayload,
  lastPaymentPayload,
  isActivePayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { SUBSCRIPTION_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 11, energy: 5_000_000, halfLife: 200 });
  assert.equal(p.deployer, 11);
  assert.equal(p.source_code, SUBSCRIPTION_SOURCE);
  assert.equal(p.energy, 5_000_000);
  assert.equal(p.half_life, 200);
});

test("set_terms carries (provider, amount, period) in canonical order", () => {
  const p = setTermsPayload({
    caller: 11,
    contractId: 88,
    providerHex: "0xfeedface",
    amount: 100,
    period: 30,
    epoch: 50,
  });
  assert.equal(p.method, "set_terms");
  assert.deepEqual(p.args, ["0xfeedface", 100, 30]);
  assert.equal(p.caller, 11);
  assert.equal(p.contract_id, 88);
  assert.equal(p.epoch, 50);
});

test("pay + cancel + no-arg views have correct method names + zero args", () => {
  for (const [fn, name] of [
    [payPayload, "pay"],
    [cancelPayload, "cancel"],
    [providerOfPayload, "provider_of"],
    [subscriberOfPayload, "subscriber_of"],
    [amountPerPeriodPayload, "amount_per_period"],
    [periodLengthPayload, "period_length"],
    [periodsPaidPayload, "periods_paid"],
    [totalPaidPayload, "total_paid"],
    [lastPaymentPayload, "last_payment"],
    [isActivePayload, "is_active"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 2, epoch: 3 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("epoch + caller + contract_id thread through every call payload", () => {
  const setTerms = setTermsPayload({
    caller: 99,
    contractId: 88,
    providerHex: "0x1",
    amount: 10,
    period: 5,
    epoch: 77,
  });
  assert.equal(setTerms.caller, 99);
  assert.equal(setTerms.contract_id, 88);
  assert.equal(setTerms.epoch, 77);

  const pay = payPayload({ caller: 99, contractId: 88, epoch: 77 });
  assert.equal(pay.caller, 99);
  assert.equal(pay.contract_id, 88);
  assert.equal(pay.epoch, 77);

  const cancel = cancelPayload({ caller: 99, contractId: 88, epoch: 77 });
  assert.equal(cancel.caller, 99);
  assert.equal(cancel.contract_id, 88);
  assert.equal(cancel.epoch, 77);
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("SUBSCRIPTION_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "subscriber:",
    "provider:",
    "period_amount:",
    "period_length:",
    "sealed:",
    "paid_periods:",
    "cumulative_paid:",
    "last_payment_epoch:",
    "cancelled:",
    "lapsed:",
    // mutators
    "fn set_terms(",
    "fn pay()",
    "fn cancel()",
    // views
    "fn provider_of()",
    "fn subscriber_of()",
    "fn amount_per_period()",
    "fn period_length()",
    "fn periods_paid()",
    "fn total_paid()",
    "fn last_payment()",
    "fn is_active()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only subscriber can set terms",
    "only subscriber can pay",
    "not authorized to cancel",
    // chain-as-keeper claim
    "self.lapsed = true",
  ]) {
    assert.ok(SUBSCRIPTION_SOURCE.includes(name), `SUBSCRIPTION_SOURCE missing: ${name}`);
  }
});

test("SUBSCRIPTION_SOURCE: on_evaporate flips lapsed ONLY if not cancelled", () => {
  // The clean-cancel vs lapse distinction is doctrinally
  // load-bearing — a cancelled subscription ended cleanly and
  // does NOT relapse on evaporation; a forgotten subscription
  // (no payments, no cancel) DOES lapse. The on_evaporate body
  // must gate on cancelled to preserve this.
  const evap = SUBSCRIPTION_SOURCE.slice(
    SUBSCRIPTION_SOURCE.indexOf("on_evaporate()"),
    SUBSCRIPTION_SOURCE.length,
  );
  assert.ok(
    evap.includes("if self.cancelled == false"),
    "on_evaporate must check cancelled before flipping lapsed",
  );
  assert.ok(evap.includes("self.lapsed = true"), "on_evaporate must flip lapsed when not cancelled");
  assert.ok(evap.includes("self.lapsed_at_epoch = epoch"), "on_evaporate must record lapse epoch");
});

test("SUBSCRIPTION_SOURCE: pay() bumps counters in the right order", () => {
  // SUB-1 (per the .es comment): no rate-limit on pay() — multiple
  // calls in the same epoch all succeed. The off-chain coordinator
  // is responsible for crediting only one payment per period. Pin
  // the counter-bump shape so a future refactor doesn't silently
  // drop one of these mutations.
  const pay = SUBSCRIPTION_SOURCE.slice(
    SUBSCRIPTION_SOURCE.indexOf("fn pay()"),
    SUBSCRIPTION_SOURCE.indexOf("fn cancel()"),
  );
  assert.ok(pay.includes("self.paid_periods += 1"), "pay() must bump paid_periods");
  assert.ok(pay.includes("self.cumulative_paid += self.period_amount"), "pay() must accumulate cumulative_paid");
  assert.ok(pay.includes("self.last_payment_epoch = epoch"), "pay() must record last_payment_epoch");
  assert.ok(pay.includes("return self.period_amount"), "pay() must return amount paid");
});

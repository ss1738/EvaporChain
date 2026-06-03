import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  setFeedPayload,
  updatePayload,
  disputePayload,
  latestPayload,
  agePayload,
  isFreshPayload,
  feedLabelPayload,
  updatesTotalPayload,
  disputesTotalPayload,
  lastUpdatedPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { ORACLE_FEED_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 5, energy: 100_000, halfLife: 365 });
  assert.equal(p.deployer, 5);
  assert.equal(p.source_code, ORACLE_FEED_SOURCE);
  assert.equal(p.energy, 100_000);
  assert.equal(p.half_life, 365);
});

test("set_feed carries (label, freshness_window) in canonical order", () => {
  const p = setFeedPayload({
    caller: 5, contractId: 42, feedLabel: "ETH/USD", freshnessWindow: 60, epoch: 100,
  });
  assert.equal(p.method, "set_feed");
  assert.deepEqual(p.args, ["ETH/USD", 60]);
});

test("update carries the new value", () => {
  const p = updatePayload({ caller: 5, contractId: 42, newValue: 4567, epoch: 150 });
  assert.equal(p.method, "update");
  assert.deepEqual(p.args, [4567]);
});

test("dispute is a no-arg open call", () => {
  const p = disputePayload({ caller: 9, contractId: 42, epoch: 170 });
  assert.equal(p.method, "dispute");
  assert.deepEqual(p.args, []);
});

test("no-arg views have correct method names + zero args", () => {
  for (const [fn, name] of [
    [latestPayload, "latest"],
    [agePayload, "age"],
    [isFreshPayload, "is_fresh"],
    [feedLabelPayload, "feed_label"],
    [updatesTotalPayload, "updates_total"],
    [disputesTotalPayload, "disputes_total"],
    [lastUpdatedPayload, "last_updated"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 2, epoch: 3 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("ORACLE_FEED_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "label:",
    "max_age:",
    "sealed:",
    "value:",
    "value_set:",
    "updated_at_epoch:",
    "update_count:",
    "dispute_count:",
    // mutators
    "fn set_feed(",
    "fn update(",
    "fn dispute()",
    // views
    "fn latest()",
    "fn age()",
    "fn is_fresh()",
    "fn feed_label()",
    "fn updates_total()",
    "fn disputes_total()",
    "fn last_updated()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only operator can configure",
    "feed already configured",
    "feed not configured",
    "only operator can update",
    "no value published yet",
    "oracle feed evaporated — stale data no longer on-chain",
  ]) {
    assert.ok(
      ORACLE_FEED_SOURCE.includes(name),
      `ORACLE_FEED_SOURCE missing: ${name}`,
    );
  }
});

test("ORACLE_FEED_SOURCE: set_feed is one-shot (sealed-flag gate)", () => {
  // The doctrine claim — `max_age` is a hard ceiling — depends on the
  // operator being unable to retroactively raise it after a stale
  // value lands. Pin the sealed-flag gate so set_feed cannot be
  // re-called.
  const setFeed = ORACLE_FEED_SOURCE.slice(
    ORACLE_FEED_SOURCE.indexOf("fn set_feed("),
    ORACLE_FEED_SOURCE.indexOf("fn update("),
  );
  assert.ok(
    setFeed.includes("require(self.sealed == false, \"feed already configured\")"),
    "set_feed must reject re-configuration once sealed",
  );
  assert.ok(
    setFeed.includes("self.sealed = true"),
    "set_feed must flip sealed = true after successful configuration",
  );
});

test("ORACLE_FEED_SOURCE: latest() reverts when no value has been published", () => {
  // The doctrine inversion — "no value" is structurally !fresh — is
  // worthless if `latest()` happily returns the default 0. The whole
  // point of the contract is that consumers cannot silently consume
  // an unset feed. Pin the `value_set == true` gate at read time.
  const latest = ORACLE_FEED_SOURCE.slice(
    ORACLE_FEED_SOURCE.indexOf("fn latest()"),
    ORACLE_FEED_SOURCE.indexOf("fn age()"),
  );
  assert.ok(
    latest.includes("require(self.value_set == true, \"no value published yet\")"),
    "latest() must revert when value_set == false (no sentinel returns)",
  );
});

test("ORACLE_FEED_SOURCE: is_fresh() returns false when value_set == false", () => {
  // Pin the early-return for the unset-value path. Without it,
  // `epoch - updated_at_epoch` would compute against
  // `updated_at_epoch == 0`, potentially returning `true` when
  // `max_age` is huge — a critical hole that would let consumers
  // silently consume an unset feed as fresh.
  const isFresh = ORACLE_FEED_SOURCE.slice(
    ORACLE_FEED_SOURCE.indexOf("fn is_fresh()"),
    ORACLE_FEED_SOURCE.indexOf("on_grace()"),
  );
  assert.ok(
    isFresh.includes("if self.value_set == false"),
    "is_fresh must check value_set BEFORE computing age",
  );
  assert.ok(
    isFresh.includes("return false"),
    "is_fresh must early-return false on the value_set==false path",
  );
});

test("ORACLE_FEED_SOURCE: dispute is open (no caller gate)", () => {
  // The dispute counter is a public signal. If it were operator-only,
  // it would be self-disputable, which defeats the point. If it had
  // some other gate (member-list, stake), arbitration would have to
  // duplicate the gate. Pin the "no caller check" property by asserting
  // dispute() contains the `sealed == true` check but NOT a `caller ==`
  // check.
  const dispute = ORACLE_FEED_SOURCE.slice(
    ORACLE_FEED_SOURCE.indexOf("fn dispute()"),
    ORACLE_FEED_SOURCE.indexOf("fn feed_label()"),
  );
  assert.ok(
    dispute.includes("require(self.sealed == true"),
    "dispute must check the feed is configured",
  );
  assert.ok(
    !dispute.includes("caller =="),
    "dispute must NOT have a caller gate — it's open by design",
  );
});

test("ORACLE_FEED_SOURCE: only owner can update (operator-locked publication)", () => {
  // The whole oracle premise — the operator vouches for the value —
  // collapses if anyone can update. Pin the `caller == owner` gate.
  const update = ORACLE_FEED_SOURCE.slice(
    ORACLE_FEED_SOURCE.indexOf("fn update("),
    ORACLE_FEED_SOURCE.indexOf("fn latest()"),
  );
  assert.ok(
    update.includes("require(caller == owner, \"only operator can update\")"),
    "update must restrict to caller == owner",
  );
});

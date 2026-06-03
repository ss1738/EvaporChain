import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  issuePayload,
  spendPayload,
  currentHolderPayload,
  isSpentPayload,
  faceValuePayload,
  liveValuePayload,
  issuedEpochPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { EVAPORCASH_NOTE_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 5, energy: 100_000, halfLife: 365 });
  assert.equal(p.deployer, 5);
  assert.equal(p.source_code, EVAPORCASH_NOTE_SOURCE);
  assert.equal(p.energy, 100_000);
  assert.equal(p.half_life, 365);
});

test("issue carries (to, face_value) in canonical order", () => {
  const p = issuePayload({
    caller: 5, contractId: 42, toHex: "0xbeef", faceValue: 1_000_000, epoch: 100,
  });
  assert.equal(p.method, "issue");
  assert.deepEqual(p.args, ["0xbeef", 1_000_000]);
});

test("spend carries the recipient address", () => {
  const p = spendPayload({ caller: 9, contractId: 42, toHex: "0xcafe", epoch: 200 });
  assert.equal(p.method, "spend");
  assert.deepEqual(p.args, ["0xcafe"]);
});

test("no-arg views have correct method names + zero args", () => {
  for (const [fn, name] of [
    [currentHolderPayload, "current_holder"],
    [isSpentPayload, "is_spent"],
    [faceValuePayload, "face_value"],
    [liveValuePayload, "live_value"],
    [issuedEpochPayload, "issued_epoch"],
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

test("EVAPORCASH_NOTE_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "holder:",
    "face:",
    "issued_at:",
    "sealed:",
    "spent:",
    // mutators
    "fn issue(",
    "fn spend(",
    // views
    "fn current_holder()",
    "fn is_spent()",
    "fn face_value()",
    "fn live_value()",
    "fn issued_epoch()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only issuer can issue this note",
    "note already issued",
    "face value must be positive",
    "note not yet issued",
    "note already spent",
    "only the holder can spend",
    "note evaporated — value lost to hoarding",
  ]) {
    assert.ok(
      EVAPORCASH_NOTE_SOURCE.includes(name),
      `EVAPORCASH_NOTE_SOURCE missing: ${name}`,
    );
  }
});

test("EVAPORCASH_NOTE_SOURCE: live_value returns the `energy` builtin (NOT a stored snapshot)", () => {
  // The doctrine cornerstone — the note's spendable value IS the
  // chain's energy, never re-derived in-contract — depends on
  // live_value() returning the `energy` builtin directly. If this
  // ever drifts to a stored field (`self.live` or similar), the
  // demurrage premise collapses: a hoarded note would stop bleeding
  // value, and the Wörgl/Gesell incentive would die. Pin the
  // single-line shape.
  const liveValue = EVAPORCASH_NOTE_SOURCE.slice(
    EVAPORCASH_NOTE_SOURCE.indexOf("fn live_value()"),
    EVAPORCASH_NOTE_SOURCE.indexOf("fn issued_epoch()"),
  );
  assert.ok(
    liveValue.includes("return energy"),
    "live_value MUST return the `energy` builtin — not self.<anything>",
  );
  assert.ok(
    !liveValue.includes("return self."),
    "live_value MUST NOT return any stored field — that would freeze the demurrage clock",
  );
});

test("EVAPORCASH_NOTE_SOURCE: face_value returns the STORED snapshot (NOT the energy builtin)", () => {
  // The two-value separation is the whole doctrine. face is for
  // accounting (issue-time snapshot, never moves); live_value is for
  // what you can spend (tracks the chain). If face_value ever drifts
  // to `return energy`, the receipt/audit trail loses its issue-time
  // anchor and the entire UI premise (showing "issued at face X /
  // worth live Y now") collapses. Pin the field-return shape.
  const faceValue = EVAPORCASH_NOTE_SOURCE.slice(
    EVAPORCASH_NOTE_SOURCE.indexOf("fn face_value()"),
    EVAPORCASH_NOTE_SOURCE.indexOf("fn live_value()"),
  );
  assert.ok(
    faceValue.includes("return self.face"),
    "face_value MUST return self.face — the issue-time accounting snapshot",
  );
  assert.ok(
    !faceValue.match(/return\s+energy/),
    "face_value MUST NOT return the energy builtin — that's live_value's job",
  );
});

test("EVAPORCASH_NOTE_SOURCE: issue is one-shot (sealed-flag gate)", () => {
  // A note re-issuable post-seal would let the deployer rotate
  // bearers at will, defeating the bearer-instrument premise. Pin
  // the sealed-flag gate.
  const issue = EVAPORCASH_NOTE_SOURCE.slice(
    EVAPORCASH_NOTE_SOURCE.indexOf("fn issue("),
    EVAPORCASH_NOTE_SOURCE.indexOf("fn spend("),
  );
  assert.ok(
    issue.includes("require(self.sealed == false, \"note already issued\")"),
    "issue must reject re-issuance once sealed",
  );
  assert.ok(
    issue.includes("self.sealed = true"),
    "issue must flip sealed = true after binding the bearer",
  );
});

test("EVAPORCASH_NOTE_SOURCE: on_evaporate emits hoarding-loss only when spent == false", () => {
  // A spent note's evaporation is silent — the value was already
  // preserved off-chain when the coordinator reissued. Emitting
  // "value lost to hoarding" on a spent note would double-count the
  // outcome and corrupt the off-chain accounting. Pin the gate.
  const evap = EVAPORCASH_NOTE_SOURCE.slice(
    EVAPORCASH_NOTE_SOURCE.indexOf("on_evaporate()"),
    EVAPORCASH_NOTE_SOURCE.length,
  );
  assert.ok(
    evap.includes("if self.spent == false"),
    "on_evaporate must gate the hoarding-loss emit on spent == false",
  );
  assert.ok(
    evap.includes("value lost to hoarding"),
    "on_evaporate must emit the canonical hoarding-loss marker",
  );
});

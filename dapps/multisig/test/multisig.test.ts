import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  addSignerPayload,
  setThresholdPayload,
  proposePayload,
  signPayload,
  executePayload,
  signersTotalPayload,
  thresholdRequiredPayload,
  signaturesCollectedPayload,
  hasSignedPayload,
  isSignerPayload,
  proposalActionPayload,
  isExecutedPayload,
  isPendingPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { MULTISIG_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 1, energy: 100_000, halfLife: 365 });
  assert.equal(p.deployer, 1);
  assert.equal(p.source_code, MULTISIG_SOURCE);
  assert.equal(p.energy, 100_000);
  assert.equal(p.half_life, 365);
});

test("add_signer carries the signer address", () => {
  const p = addSignerPayload({ caller: 1, contractId: 42, whoHex: "0xab", epoch: 0 });
  assert.equal(p.method, "add_signer");
  assert.deepEqual(p.args, ["0xab"]);
});

test("set_threshold carries the threshold value", () => {
  const p = setThresholdPayload({ caller: 1, contractId: 42, threshold: 3, epoch: 0 });
  assert.equal(p.method, "set_threshold");
  assert.deepEqual(p.args, [3]);
});

test("propose carries the action string", () => {
  const p = proposePayload({
    caller: 1,
    contractId: 42,
    action: "transfer 1000 to 0xdeadbeef",
    epoch: 10,
  });
  assert.equal(p.method, "propose");
  assert.deepEqual(p.args, ["transfer 1000 to 0xdeadbeef"]);
});

test("address-arg views carry the queried address", () => {
  for (const [fn, name] of [
    [hasSignedPayload, "has_signed"],
    [isSignerPayload, "is_signer"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 42, whoHex: "0xcd", epoch: 0 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, ["0xcd"]);
  }
});

test("sign + execute + no-arg views have correct method names + zero args", () => {
  for (const [fn, name] of [
    [signPayload, "sign"],
    [executePayload, "execute"],
    [signersTotalPayload, "signers_total"],
    [thresholdRequiredPayload, "threshold_required"],
    [signaturesCollectedPayload, "signatures_collected"],
    [proposalActionPayload, "proposal_action"],
    [isExecutedPayload, "is_executed"],
    [isPendingPayload, "is_pending"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 2, epoch: 3 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("epoch + caller + contract_id thread through every call payload", () => {
  const cases = [
    addSignerPayload({ caller: 99, contractId: 88, whoHex: "0x1", epoch: 77 }),
    setThresholdPayload({ caller: 99, contractId: 88, threshold: 2, epoch: 77 }),
    proposePayload({ caller: 99, contractId: 88, action: "x", epoch: 77 }),
    signPayload({ caller: 99, contractId: 88, epoch: 77 }),
    executePayload({ caller: 99, contractId: 88, epoch: 77 }),
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

test("MULTISIG_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "signer_set:",
    "signed_set:",
    "signer_count:",
    "threshold:",
    "sealed:",
    "action:",
    "signature_count:",
    "executed:",
    "expired:",
    // mutators
    "fn add_signer(",
    "fn set_threshold(",
    "fn propose(",
    "fn sign()",
    "fn execute()",
    // views
    "fn signers_total()",
    "fn threshold_required()",
    "fn signatures_collected()",
    "fn has_signed(",
    "fn is_signer(",
    "fn proposal_action()",
    "fn is_executed()",
    "fn is_pending()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only owner can add signers",
    "only owner can set threshold",
    "only owner can propose",
    "already signed",
    "not a signer",
    "threshold not yet reached",
    "threshold exceeds signer count",
  ]) {
    assert.ok(MULTISIG_SOURCE.includes(name), `MULTISIG_SOURCE missing: ${name}`);
  }
});

test("MULTISIG_SOURCE: set_threshold rejects t > signer_count (no bricked contract)", () => {
  // A threshold higher than the registered signer count would
  // leave no quorum able to satisfy it. The contract would never
  // execute; on evaporation it expires. The doctrine wants this
  // rejected at config time, not silently allowed.
  const block = MULTISIG_SOURCE.slice(
    MULTISIG_SOURCE.indexOf("fn set_threshold("),
    MULTISIG_SOURCE.indexOf("fn propose("),
  );
  assert.ok(
    block.includes("t <= self.signer_count"),
    "set_threshold must require t <= signer_count",
  );
});

test("MULTISIG_SOURCE: sign() requires registered signer + no double-sign", () => {
  // The doctrine of one-decision-per-contract means signatures
  // must be unforgeable + non-duplicate. Two checks: caller must
  // be in signer_set, and signed_set[caller] must be 0.
  const block = MULTISIG_SOURCE.slice(
    MULTISIG_SOURCE.indexOf("fn sign()"),
    MULTISIG_SOURCE.indexOf("fn execute()"),
  );
  assert.ok(
    block.includes("self.signer_set[caller] > 0"),
    "sign() must check caller is a registered signer",
  );
  assert.ok(
    block.includes("self.signed_set[caller] == 0"),
    "sign() must reject duplicate signatures",
  );
});

test("MULTISIG_SOURCE: on_evaporate flips expired ONLY if not executed", () => {
  // An executed multisig discharged its purpose; expired stays
  // false. An unexecuted multisig at evaporation = expired
  // (decision lapsed). This split is doctrinally load-bearing —
  // off-chain coordinators need to distinguish "the vote carried"
  // from "the vote timed out."
  const evap = MULTISIG_SOURCE.slice(
    MULTISIG_SOURCE.indexOf("on_evaporate()"),
    MULTISIG_SOURCE.length,
  );
  assert.ok(
    evap.includes("if self.executed == false"),
    "on_evaporate must gate on executed == false before flipping expired",
  );
  assert.ok(
    evap.includes("self.expired = true"),
    "on_evaporate must flip expired when not executed",
  );
});

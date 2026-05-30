import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  armPayload,
  exercisePayload,
  revokePayload,
  isActivePayload,
  epochsRemainingPayload,
  isLesseePayload,
  verbViewPayload,
  objectViewPayload,
  exercisesTotalPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { SCL_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source", () => {
  const p = deployPayload({ deployer: 1, energy: 1000, halfLife: 100 });
  assert.equal(p.source_code, SCL_SOURCE);
});

test("armPayload carries lessee/verb/object/duration in order", () => {
  const p = armPayload({
    caller: 1,
    contractId: 7,
    lesseeHex: "0x22",
    verb: "read",
    objectHex: "0xabcd",
    durationEpochs: 1000,
    epoch: 0,
  });
  assert.equal(p.method, "arm");
  assert.deepEqual(p.args, ["0x22", "read", "0xabcd", 1000]);
});

test("exercise / revoke / view payloads are no-arg with correct names", () => {
  assert.equal(exercisePayload({ caller: 1, contractId: 7, epoch: 0 }).method, "exercise");
  assert.deepEqual(exercisePayload({ caller: 1, contractId: 7, epoch: 0 }).args, []);

  assert.equal(revokePayload({ caller: 1, contractId: 7, epoch: 0 }).method, "revoke");

  for (const [fn, name] of [
    [isActivePayload, "is_active"],
    [epochsRemainingPayload, "epochs_remaining"],
    [verbViewPayload, "verb_view"],
    [objectViewPayload, "object_view"],
    [exercisesTotalPayload, "exercises_total"],
  ] as const) {
    assert.equal(fn({ caller: 1, contractId: 7, epoch: 0 }).method, name);
  }
});

test("isLesseePayload carries the candidate address", () => {
  const p = isLesseePayload({ caller: 1, contractId: 7, whoHex: "0x22", epoch: 0 });
  assert.equal(p.method, "is_lessee");
  assert.deepEqual(p.args, ["0x22"]);
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("SCL_SOURCE contains all methods + lifecycle hooks", () => {
  for (const name of [
    "arm",
    "exercise",
    "revoke",
    "is_active",
    "epochs_remaining",
    "is_lessee",
    "verb_view",
    "object_view",
    "on_grace",
    "on_refresh",
    "on_evaporate",
    "structurally revoked", // doctrine-flagging event string
  ]) {
    assert.ok(SCL_SOURCE.includes(name), `SCL_SOURCE missing: ${name}`);
  }
});

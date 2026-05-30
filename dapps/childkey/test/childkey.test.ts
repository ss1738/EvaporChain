import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  addCommitteeMemberPayload,
  armPayload,
  voteEmergencyPayload,
  finalizeEmergencyUnlockPayload,
  finalizeNaturalUnlockPayload,
  readContentPayload,
  isCommitteeMemberPayload,
  voteProgressPayload,
  thresholdRequiredPayload,
  isArmedPayload,
  isUnlockedPayload,
  unlockAtPayload,
  epochsUntilUnlockPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { CHILDKEY_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + params", () => {
  const p = deployPayload({ deployer: 1, energy: 1000000, halfLife: 100 });
  assert.equal(p.source_code, CHILDKEY_SOURCE);
});

test("add_committee_member carries the address", () => {
  const p = addCommitteeMemberPayload({ caller: 1, contractId: 7, memberHex: "0xA1", epoch: 0 });
  assert.equal(p.method, "add_committee_member");
  assert.deepEqual(p.args, ["0xA1"]);
});

test("arm carries the 4 positional args in the right order", () => {
  const p = armPayload({
    caller: 1,
    contractId: 7,
    recipientHex: "0x22",
    unlockEpoch: 6570,
    contentHash: "ipfs://bafy...",
    threshold: 3,
    epoch: 0,
  });
  assert.equal(p.method, "arm");
  assert.deepEqual(p.args, ["0x22", 6570, "ipfs://bafy...", 3]);
});

test("committee-side payloads have correct method names + no args", () => {
  assert.equal(voteEmergencyPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "vote_emergency");
  assert.deepEqual(voteEmergencyPayload({ caller: 1, contractId: 7, epoch: 0 }).args, []);

  assert.equal(
    finalizeEmergencyUnlockPayload({ caller: 1, contractId: 7, epoch: 0 }).method,
    "finalize_emergency_unlock",
  );
  assert.equal(
    finalizeNaturalUnlockPayload({ caller: 1, contractId: 7, epoch: 0 }).method,
    "finalize_natural_unlock",
  );
  assert.equal(readContentPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "read_content");
});

test("view payloads have correct method names + args", () => {
  const cm = isCommitteeMemberPayload({ caller: 1, contractId: 7, whoHex: "0xA1", epoch: 0 });
  assert.equal(cm.method, "is_committee_member");
  assert.deepEqual(cm.args, ["0xA1"]);

  for (const [fn, name] of [
    [voteProgressPayload, "vote_progress"],
    [thresholdRequiredPayload, "threshold_required"],
    [isArmedPayload, "is_armed"],
    [isUnlockedPayload, "is_unlocked"],
    [unlockAtPayload, "unlock_at"],
    [epochsUntilUnlockPayload, "epochs_until_unlock"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 7, epoch: 0 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("CHILDKEY_SOURCE contains all methods + lifecycle hooks", () => {
  for (const name of [
    "add_committee_member",
    "arm",
    "vote_emergency",
    "finalize_emergency_unlock",
    "finalize_natural_unlock",
    "read_content",
    "is_committee_member",
    "epochs_until_unlock",
    "on_grace",
    "on_refresh",
    "on_evaporate",
  ]) {
    assert.ok(CHILDKEY_SOURCE.includes(name), `CHILDKEY_SOURCE missing: ${name}`);
  }
});

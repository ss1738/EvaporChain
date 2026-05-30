import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  addMemberPayload,
  refreshMembershipPayload,
  openProposalPayload,
  voteForPayload,
  voteAgainstPayload,
  closeProposalPayload,
  memberCountPayload,
  isMemberPayload,
  isActivePayload,
  weightOfPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { MORTAL_DAO_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + params", () => {
  const p = deployPayload({ deployer: 1, energy: 1000, halfLife: 100 });
  assert.equal(p.source_code, MORTAL_DAO_SOURCE);
  assert.equal(p.energy, 1000);
  assert.equal(p.half_life, 100);
  assert.equal(p.deployer, 1);
});

test("add_member carries the address + correct method + contract id", () => {
  const p = addMemberPayload({ caller: 1, contractId: 7, memberHex: "0x21", epoch: 0 });
  assert.equal(p.method, "add_member");
  assert.deepEqual(p.args, ["0x21"]);
  assert.equal(p.contract_id, 7);
  assert.equal(p.caller, 1);
});

test("no-arg member methods take empty args + correct method names", () => {
  assert.deepEqual(refreshMembershipPayload({ caller: 1, contractId: 7, epoch: 0 }).args, []);
  assert.equal(refreshMembershipPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "refresh_membership");

  assert.deepEqual(voteForPayload({ caller: 1, contractId: 7, epoch: 0 }).args, []);
  assert.equal(voteForPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "vote_for");

  assert.deepEqual(voteAgainstPayload({ caller: 1, contractId: 7, epoch: 0 }).args, []);
  assert.equal(voteAgainstPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "vote_against");

  assert.deepEqual(closeProposalPayload({ caller: 1, contractId: 7, epoch: 50 }).args, []);
  assert.equal(closeProposalPayload({ caller: 1, contractId: 7, epoch: 50 }).method, "close_proposal");
});

test("open_proposal carries the proposal text as a single string arg", () => {
  const p = openProposalPayload({ caller: 1, contractId: 7, text: "lower the gas floor", epoch: 0 });
  assert.equal(p.method, "open_proposal");
  assert.deepEqual(p.args, ["lower the gas floor"]);
  assert.equal(p.caller, 1);
  assert.equal(p.epoch, 0);
});

test("view methods on whole-DAO state take no args", () => {
  const p = memberCountPayload({ caller: 1, contractId: 7, epoch: 100 });
  assert.equal(p.method, "member_count_now");
  assert.deepEqual(p.args, []);
});

test("view methods on a specific address pass the hex address", () => {
  const im = isMemberPayload({ caller: 1, contractId: 7, whoHex: "0x21", epoch: 0 });
  assert.equal(im.method, "is_member");
  assert.deepEqual(im.args, ["0x21"]);

  const ia = isActivePayload({ caller: 1, contractId: 7, whoHex: "0x21", epoch: 50 });
  assert.equal(ia.method, "is_active");
  assert.deepEqual(ia.args, ["0x21"]);

  const w = weightOfPayload({ caller: 1, contractId: 7, whoHex: "0x21", epoch: 100 });
  assert.equal(w.method, "weight_of");
  assert.deepEqual(w.args, ["0x21"]);
});

test("endpoint paths match the node tx API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("MORTAL_DAO_SOURCE contains all the lifecycle hooks + decay primitive markers", () => {
  // Sanity-check the inlined .es source vs the on-chain semantic surface.
  // (Stays loose — these are the public method names, byte-stable.)
  for (const name of [
    "add_member",
    "refresh_membership",
    "open_proposal",
    "vote_for",
    "vote_against",
    "close_proposal",
    "weight_of",
    "on_grace",
    "on_refresh",
    "on_evaporate",
  ]) {
    assert.ok(MORTAL_DAO_SOURCE.includes(name), `MORTAL_DAO_SOURCE missing method: ${name}`);
  }
});

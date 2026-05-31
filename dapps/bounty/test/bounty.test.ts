import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  setBountyPayload,
  submitPayload,
  acceptPayload,
  claimPayload,
  cancelPayload,
  taskOfPayload,
  rewardPayload,
  submissionsTotalPayload,
  submissionOfPayload,
  winnerOfPayload,
  isAcceptedPayload,
  isClaimedPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { BOUNTY_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 3, energy: 1_000_000, halfLife: 100 });
  assert.equal(p.deployer, 3);
  assert.equal(p.source_code, BOUNTY_SOURCE);
  assert.equal(p.energy, 1_000_000);
  assert.equal(p.half_life, 100);
});

test("set_bounty carries (task, reward) in canonical order", () => {
  const p = setBountyPayload({
    caller: 3,
    contractId: 42,
    task: "find an integer overflow in /api/tx/deploy-script",
    reward: 5000,
    epoch: 10,
  });
  assert.equal(p.method, "set_bounty");
  assert.deepEqual(p.args, [
    "find an integer overflow in /api/tx/deploy-script",
    5000,
  ]);
});

test("submit carries the solution string", () => {
  const p = submitPayload({
    caller: 9,
    contractId: 42,
    solution: "see proof at https://example.com/repro.txt",
    epoch: 25,
  });
  assert.equal(p.method, "submit");
  assert.deepEqual(p.args, ["see proof at https://example.com/repro.txt"]);
});

test("accept carries the winner address", () => {
  const p = acceptPayload({
    caller: 3,
    contractId: 42,
    winnerHex: "0xaa",
    epoch: 30,
  });
  assert.equal(p.method, "accept");
  assert.deepEqual(p.args, ["0xaa"]);
});

test("submission_of carries the queried address", () => {
  const p = submissionOfPayload({
    caller: 1,
    contractId: 42,
    whoHex: "0xbb",
    epoch: 0,
  });
  assert.equal(p.method, "submission_of");
  assert.deepEqual(p.args, ["0xbb"]);
});

test("no-arg payloads have correct method names + zero args", () => {
  for (const [fn, name] of [
    [claimPayload, "claim"],
    [cancelPayload, "cancel"],
    [taskOfPayload, "task_of"],
    [rewardPayload, "reward"],
    [submissionsTotalPayload, "submissions_total"],
    [winnerOfPayload, "winner_of"],
    [isAcceptedPayload, "is_accepted"],
    [isClaimedPayload, "is_claimed"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 2, epoch: 3 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("epoch + caller + contract_id thread through every call payload", () => {
  const setB = setBountyPayload({
    caller: 99, contractId: 88, task: "x", reward: 1, epoch: 77,
  });
  assert.equal(setB.caller, 99);
  assert.equal(setB.contract_id, 88);
  assert.equal(setB.epoch, 77);

  const sub = submitPayload({ caller: 99, contractId: 88, solution: "x", epoch: 77 });
  assert.equal(sub.caller, 99);
  assert.equal(sub.contract_id, 88);
  assert.equal(sub.epoch, 77);

  const acc = acceptPayload({ caller: 99, contractId: 88, winnerHex: "0x1", epoch: 77 });
  assert.equal(acc.caller, 99);
  assert.equal(acc.contract_id, 88);
  assert.equal(acc.epoch, 77);
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("BOUNTY_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "poster:",
    "task:",
    "reward_amount:",
    "sealed:",
    "submissions:",
    "has_submitted:",
    "submission_count:",
    "accepted:",
    "winner:",
    "claimed:",
    "cancelled:",
    "refunded:",
    // mutators
    "fn set_bounty(",
    "fn submit(",
    "fn accept(",
    "fn claim()",
    "fn cancel()",
    // views
    "fn task_of()",
    "fn reward()",
    "fn submissions_total()",
    "fn submission_of(",
    "fn winner_of()",
    "fn is_accepted()",
    "fn is_claimed()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only poster can set bounty",
    "only poster can accept",
    "only poster can cancel",
    "only winner can claim",
    "submissions exist — cannot cancel",
    "refund to poster",
  ]) {
    assert.ok(BOUNTY_SOURCE.includes(name), `BOUNTY_SOURCE missing: ${name}`);
  }
});

test("BOUNTY_SOURCE: cancel() blocked once any hunter has submitted (no rug-pull)", () => {
  // The poster's clawback right ends the moment the first hunter
  // commits work. Without this guard the poster could let hunters
  // do investigation, then cancel before accepting anyone — a
  // doctrine-critical anti-rug-pull invariant.
  const cancel = BOUNTY_SOURCE.slice(
    BOUNTY_SOURCE.indexOf("fn cancel()"),
    BOUNTY_SOURCE.indexOf("fn task_of()"),
  );
  assert.ok(
    cancel.includes("self.submission_count == 0"),
    "cancel() must require submission_count == 0",
  );
});

test("BOUNTY_SOURCE: BOUNTY-1 — submission_of guards against missing-key-returns-zero", () => {
  // EvaporScript maps return U64(0) for missing keys regardless of
  // declared value type. self.submissions[who] on a non-submitter
  // would return 0 (not ""), corrupting downstream string ops. The
  // parallel `has_submitted` presence map is the correct guard.
  const subOf = BOUNTY_SOURCE.slice(
    BOUNTY_SOURCE.indexOf("fn submission_of("),
    BOUNTY_SOURCE.indexOf("fn winner_of()"),
  );
  assert.ok(
    subOf.includes("self.has_submitted[who] == 0"),
    "submission_of must guard with has_submitted presence map",
  );
  assert.ok(subOf.includes('return ""'), "submission_of must return empty string for missing");
});

test("BOUNTY_SOURCE: on_evaporate flips refunded ONLY if not accepted", () => {
  // The refund path is the chain-as-keeper doctrine in action.
  // If the bounty has been accepted, the winner is owed the reward;
  // no auto-refund. If unaccepted, the poster gets their funds back
  // automatically — no off-chain liquidator needed.
  const evap = BOUNTY_SOURCE.slice(
    BOUNTY_SOURCE.indexOf("on_evaporate()"),
    BOUNTY_SOURCE.length,
  );
  assert.ok(
    evap.includes("if self.accepted == false"),
    "on_evaporate must gate on accepted == false before refunding",
  );
  assert.ok(evap.includes("self.refunded = true"), "on_evaporate must flip refunded when not accepted");
});

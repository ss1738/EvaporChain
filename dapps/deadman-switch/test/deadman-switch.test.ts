import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  armPayload,
  refreshPayload,
  triggerEarlyPayload,
  releaseDeadPayload,
  transferHolderPayload,
  isAlivePayload,
  isReleasablePayload,
  isReleasedPayload,
  epochsUntilDeadlinePayload,
  secretHashViewPayload,
  revealedSecretViewPayload,
  releasedAtViewPayload,
  refreshCountPayload,
  lastRefreshPayload,
  holderViewPayload,
  isHolderPayload,
  isArmedPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { DEADMAN_SWITCH_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 1, energy: 1_000_000, halfLife: 100 });
  assert.equal(p.deployer, 1);
  assert.equal(p.source_code, DEADMAN_SWITCH_SOURCE);
  assert.equal(p.energy, 1_000_000);
  assert.equal(p.half_life, 100);
});

test("arm carries (holder, payload_hash, window) in canonical order", () => {
  const p = armPayload({
    caller: 1,
    contractId: 42,
    holderHex: "0xab",
    payloadHash: "0xdeadbeef",
    windowEpochs: 100,
    epoch: 0,
  });
  assert.equal(p.method, "arm");
  assert.deepEqual(p.args, ["0xab", "0xdeadbeef", 100]);
  assert.equal(p.epoch, 0);
});

test("trigger_early + release_dead carry the optional plaintext arg", () => {
  const earlyEmpty = triggerEarlyPayload({ caller: 1, contractId: 42, plaintext: "", epoch: 50 });
  assert.equal(earlyEmpty.method, "trigger_early");
  assert.deepEqual(earlyEmpty.args, [""]);

  const earlyText = triggerEarlyPayload({
    caller: 1,
    contractId: 42,
    plaintext: "the password is hunter2",
    epoch: 50,
  });
  assert.deepEqual(earlyText.args, ["the password is hunter2"]);

  const releaseEmpty = releaseDeadPayload({ caller: 7, contractId: 42, plaintext: "", epoch: 200 });
  assert.equal(releaseEmpty.method, "release_dead");
  assert.deepEqual(releaseEmpty.args, [""]);

  const releaseText = releaseDeadPayload({
    caller: 7,
    contractId: 42,
    plaintext: "released payload",
    epoch: 200,
  });
  assert.deepEqual(releaseText.args, ["released payload"]);
});

test("transfer_holder carries the new holder address", () => {
  const p = transferHolderPayload({
    caller: 1,
    contractId: 42,
    newHolderHex: "0xcd",
    epoch: 30,
  });
  assert.equal(p.method, "transfer_holder");
  assert.deepEqual(p.args, ["0xcd"]);
});

test("refresh + no-arg views have correct method names + zero args", () => {
  for (const [fn, name] of [
    [refreshPayload, "refresh"],
    [isAlivePayload, "is_alive"],
    [isReleasablePayload, "is_releasable"],
    [isReleasedPayload, "is_released"],
    [epochsUntilDeadlinePayload, "epochs_until_deadline"],
    [secretHashViewPayload, "secret_hash_view"],
    [revealedSecretViewPayload, "revealed_secret_view"],
    [releasedAtViewPayload, "released_at_view"],
    [refreshCountPayload, "refresh_count_view"],
    [lastRefreshPayload, "last_refresh_view"],
    [holderViewPayload, "holder_view"],
    [isArmedPayload, "is_armed"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 42, epoch: 0 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("is_holder carries the queried address", () => {
  const p = isHolderPayload({
    caller: 1,
    contractId: 42,
    whoHex: "0xab",
    epoch: 10,
  });
  assert.equal(p.method, "is_holder");
  assert.deepEqual(p.args, ["0xab"]);
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("epoch field threads through every call payload", () => {
  for (const [name, p] of [
    ["arm", armPayload({ caller: 1, contractId: 42, holderHex: "0x1", payloadHash: "0x2", windowEpochs: 50, epoch: 17 })],
    ["refresh", refreshPayload({ caller: 1, contractId: 42, epoch: 17 })],
    ["trigger_early", triggerEarlyPayload({ caller: 1, contractId: 42, plaintext: "", epoch: 17 })],
    ["release_dead", releaseDeadPayload({ caller: 1, contractId: 42, plaintext: "", epoch: 17 })],
    ["transfer_holder", transferHolderPayload({ caller: 1, contractId: 42, newHolderHex: "0x3", epoch: 17 })],
    ["is_alive", isAlivePayload({ caller: 1, contractId: 42, epoch: 17 })],
    ["is_releasable", isReleasablePayload({ caller: 1, contractId: 42, epoch: 17 })],
  ] as const) {
    assert.equal(p.epoch, 17, `${name}: epoch arg should reach the payload`);
  }
});

test("caller + contract_id thread through every call payload", () => {
  const arm = armPayload({ caller: 99, contractId: 88, holderHex: "0x1", payloadHash: "0x2", windowEpochs: 50, epoch: 0 });
  assert.equal(arm.caller, 99);
  assert.equal(arm.contract_id, 88);

  const release = releaseDeadPayload({ caller: 99, contractId: 88, plaintext: "x", epoch: 0 });
  assert.equal(release.caller, 99);
  assert.equal(release.contract_id, 88);
});

test("DEADMAN_SWITCH_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "holder:",
    "secret_hash:",
    "refresh_window:",
    "last_refresh_epoch:",
    "has_refreshed:",
    "released:",
    "released_at_epoch:",
    "revealed_secret:",
    "sealed:",
    // mutators
    "fn arm(",
    "fn refresh()",
    "fn trigger_early(",
    "fn release_dead(",
    "fn transfer_holder(",
    // views
    "fn is_armed()",
    "fn is_released()",
    "fn is_alive()",
    "fn is_releasable()",
    "fn epochs_until_deadline()",
    "fn secret_hash_view()",
    "fn revealed_secret_view()",
    "fn released_at_view()",
    "fn refresh_count_view()",
    "fn last_refresh_view()",
    "fn holder_view()",
    "fn is_holder(",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only deployer arms",
    "only holder refreshes",
    "only holder triggers early",
    "deadline not yet passed",
  ]) {
    assert.ok(DEADMAN_SWITCH_SOURCE.includes(name), `DEADMAN_SWITCH_SOURCE missing: ${name}`);
  }
});

test("DEADMAN_SWITCH_SOURCE: release_dead requires deadline >= last_refresh + window", () => {
  // The critical safety check: anyone can call release_dead, but
  // ONLY after the holder has gone silent for `refresh_window`
  // epochs. The .es source must contain the comparison; a typo
  // (e.g., `>` instead of `>=`, or swapped operands) would silently
  // break the doctrine.
  assert.ok(
    DEADMAN_SWITCH_SOURCE.includes("epoch >= self.last_refresh_epoch + self.refresh_window"),
    "release_dead must gate on epoch >= last_refresh + window",
  );
});

test("DEADMAN_SWITCH_SOURCE: arm() seeds the deadline (last_refresh_epoch + has_refreshed=true)", () => {
  // Without this, the dead-man state starts immediately at deploy:
  // anyone could call release_dead() on the freshly-armed switch
  // before the holder has even had a chance to refresh. The arm()
  // body must seed last_refresh_epoch + has_refreshed so the
  // deadline countdown starts from arm-time, not from epoch 0.
  const armBlock = DEADMAN_SWITCH_SOURCE.slice(
    DEADMAN_SWITCH_SOURCE.indexOf("fn arm("),
    DEADMAN_SWITCH_SOURCE.indexOf("fn refresh()"),
  );
  assert.ok(
    armBlock.includes("self.last_refresh_epoch = epoch"),
    "arm() must seed last_refresh_epoch",
  );
  assert.ok(armBlock.includes("self.has_refreshed = true"), "arm() must set has_refreshed");
  assert.ok(armBlock.includes("self.refresh_count = 1"), "arm() should count as the first refresh");
});

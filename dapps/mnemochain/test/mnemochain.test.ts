import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  armPayload,
  reviewPayload,
  transferPayload,
  retrievabilityBpPayload,
  isDuePayload,
  epochsUntilDuePayload,
  stabilityViewPayload,
  reviewCountPayload,
  isHolderPayload,
  cardContentPayload,
  isArmedPayload,
  RATING_AGAIN,
  RATING_HARD,
  RATING_GOOD,
  RATING_EASY,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { MNEMOCHAIN_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source", () => {
  const p = deployPayload({ deployer: 1, energy: 1000, halfLife: 100 });
  assert.equal(p.source_code, MNEMOCHAIN_SOURCE);
});

test("rating constants match Anki convention", () => {
  assert.equal(RATING_AGAIN, 1);
  assert.equal(RATING_HARD, 2);
  assert.equal(RATING_GOOD, 3);
  assert.equal(RATING_EASY, 4);
});

test("armPayload carries (holder, content, initial_stability)", () => {
  const p = armPayload({
    caller: 1,
    contractId: 7,
    holderHex: "0x22",
    contentHash: "ipfs://card",
    initialStability: 10,
    epoch: 0,
  });
  assert.equal(p.method, "arm");
  assert.deepEqual(p.args, ["0x22", "ipfs://card", 10]);
});

test("reviewPayload carries the rating", () => {
  const p = reviewPayload({ caller: 1, contractId: 7, rating: RATING_GOOD, epoch: 5 });
  assert.equal(p.method, "review");
  assert.deepEqual(p.args, [3]);
});

test("transferPayload carries the new holder address", () => {
  const p = transferPayload({ caller: 1, contractId: 7, toHex: "0x23", epoch: 10 });
  assert.equal(p.method, "transfer");
  assert.deepEqual(p.args, ["0x23"]);
});

test("view payloads have correct method names + args shape", () => {
  for (const [fn, name] of [
    [retrievabilityBpPayload, "retrievability_bp"],
    [isDuePayload, "is_due"],
    [epochsUntilDuePayload, "epochs_until_due"],
    [stabilityViewPayload, "stability_view"],
    [reviewCountPayload, "review_count_view"],
    [cardContentPayload, "card_content_view"],
    [isArmedPayload, "is_armed"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 7, epoch: 0 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("isHolderPayload carries the address", () => {
  const p = isHolderPayload({ caller: 1, contractId: 7, whoHex: "0x22", epoch: 0 });
  assert.equal(p.method, "is_holder");
  assert.deepEqual(p.args, ["0x22"]);
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("MNEMOCHAIN_SOURCE contains all methods + lifecycle markers", () => {
  for (const name of [
    "arm",
    "review",
    "transfer",
    "retrievability_bp",
    "is_due",
    "epochs_until_due",
    "stability_view",
    "again_count",
    "good_count",
    "hard_count",
    "on_grace",
    "on_refresh",
    "on_evaporate",
  ]) {
    assert.ok(MNEMOCHAIN_SOURCE.includes(name), `MNEMOCHAIN_SOURCE missing: ${name}`);
  }
});

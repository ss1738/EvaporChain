import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  armPayload,
  sellPayload,
  withdrawPayload,
  isReleasablePayload,
  epochsUntilReleasePayload,
  isBeneficiaryPayload,
  isOriginalFutureSelfPayload,
  depositAmountPayload,
  releaseAtPayload,
  isArmedPayload,
  isSoldPayload,
  isWithdrawnPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { SFSV_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source", () => {
  const p = deployPayload({ deployer: 1, energy: 1000, halfLife: 100 });
  assert.equal(p.source_code, SFSV_SOURCE);
});

test("arm carries (future_self, amount, release_at) in order", () => {
  const p = armPayload({
    caller: 1,
    contractId: 7,
    futureSelfHex: "0x22",
    depositAmount: 1000,
    releaseEpoch: 50,
    epoch: 0,
  });
  assert.equal(p.method, "arm");
  assert.deepEqual(p.args, ["0x22", 1000, 50]);
});

test("sell carries the buyer address", () => {
  const p = sellPayload({ caller: 1, contractId: 7, buyerHex: "0x33", epoch: 10 });
  assert.equal(p.method, "sell");
  assert.deepEqual(p.args, ["0x33"]);
});

test("withdraw + view payloads have correct method names + args", () => {
  assert.equal(withdrawPayload({ caller: 1, contractId: 7, epoch: 50 }).method, "withdraw");
  assert.deepEqual(withdrawPayload({ caller: 1, contractId: 7, epoch: 50 }).args, []);

  for (const [fn, name] of [
    [isReleasablePayload, "is_releasable"],
    [epochsUntilReleasePayload, "epochs_until_release"],
    [depositAmountPayload, "deposit_amount_view"],
    [releaseAtPayload, "release_at"],
    [isArmedPayload, "is_armed"],
    [isSoldPayload, "is_sold"],
    [isWithdrawnPayload, "is_withdrawn"],
  ] as const) {
    assert.equal(fn({ caller: 1, contractId: 7, epoch: 0 }).method, name);
  }
});

test("address-arg view payloads carry the address", () => {
  const ib = isBeneficiaryPayload({ caller: 1, contractId: 7, whoHex: "0x22", epoch: 0 });
  assert.equal(ib.method, "is_beneficiary");
  assert.deepEqual(ib.args, ["0x22"]);

  const iofs = isOriginalFutureSelfPayload({ caller: 1, contractId: 7, whoHex: "0x22", epoch: 0 });
  assert.equal(iofs.method, "is_original_future_self");
  assert.deepEqual(iofs.args, ["0x22"]);
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("SFSV_SOURCE contains all methods + lifecycle hooks + doctrine flag", () => {
  for (const name of [
    "arm",
    "sell",
    "withdraw",
    "is_releasable",
    "is_beneficiary",
    "is_original_future_self",
    "epochs_until_release",
    "on_grace",
    "on_refresh",
    "on_evaporate",
    "deposit forfeit", // doctrine-flagging event string
  ]) {
    assert.ok(SFSV_SOURCE.includes(name), `SFSV_SOURCE missing: ${name}`);
  }
});

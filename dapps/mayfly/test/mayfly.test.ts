import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  hatchPayload,
  transferPayload,
  readMetadataPayload,
  isHolderPayload,
  ageEpochsPayload,
  transfersTotalPayload,
  isHatchedPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { MAYFLY_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + short-life defaults", () => {
  const p = deployPayload({ deployer: 1, energy: 1000, halfLife: 10 });
  assert.equal(p.source_code, MAYFLY_SOURCE);
  assert.equal(p.energy, 1000);
  assert.equal(p.half_life, 10);
});

test("hatchPayload carries metadata as the single string arg", () => {
  const p = hatchPayload({ caller: 1, contractId: 7, metadata: "nymph→imago", epoch: 0 });
  assert.equal(p.method, "hatch");
  assert.deepEqual(p.args, ["nymph→imago"]);
});

test("transferPayload carries the target address", () => {
  const p = transferPayload({ caller: 1, contractId: 7, toHex: "0x21", epoch: 5 });
  assert.equal(p.method, "transfer");
  assert.deepEqual(p.args, ["0x21"]);
});

test("view payloads have correct method names + args", () => {
  assert.equal(readMetadataPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "read_metadata");
  assert.deepEqual(readMetadataPayload({ caller: 1, contractId: 7, epoch: 0 }).args, []);

  const ih = isHolderPayload({ caller: 1, contractId: 7, whoHex: "0x21", epoch: 0 });
  assert.equal(ih.method, "is_holder");
  assert.deepEqual(ih.args, ["0x21"]);

  assert.equal(ageEpochsPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "age_epochs");
  assert.equal(transfersTotalPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "transfers_total");
  assert.equal(isHatchedPayload({ caller: 1, contractId: 7, epoch: 0 }).method, "is_hatched");
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("MAYFLY_SOURCE contains all methods + lifecycle hooks", () => {
  for (const name of [
    "hatch",
    "transfer",
    "read_metadata",
    "is_holder",
    "age_epochs",
    "transfers_total",
    "is_hatched",
    "on_grace",
    "on_refresh",
    "on_evaporate",
    "mayfly refreshed — defying nature", // doctrine-flagging event string
  ]) {
    assert.ok(MAYFLY_SOURCE.includes(name), `MAYFLY_SOURCE missing: ${name}`);
  }
});

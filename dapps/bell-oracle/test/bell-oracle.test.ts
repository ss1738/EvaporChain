import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  armPayload,
  submitReadingPayload,
  isCertifiedNowPayload,
  latestSMilliPayload,
  lastHeightPayload,
  acceptedTotalPayload,
  isSubmissionWorthy,
  LOCAL_REALISM_FLOOR_MILLI,
  BELL_LATEST_PATH,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { BELL_ORACLE_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + params", () => {
  const p = deployPayload({ deployer: 1, energy: 1000, halfLife: 100 });
  assert.equal(p.source_code, BELL_ORACLE_SOURCE);
  assert.equal(p.energy, 1000);
  assert.equal(p.half_life, 100);
  assert.equal(p.deployer, 1);
});

test("arm carries max_age as a single u64 arg", () => {
  const p = armPayload({ caller: 1, contractId: 7, maxAgeEpochs: 10, epoch: 0 });
  assert.equal(p.method, "arm");
  assert.deepEqual(p.args, [10]);
  assert.equal(p.contract_id, 7);
});

test("submit_reading carries s_milli + height in that order", () => {
  const p = submitReadingPayload({
    caller: 1,
    contractId: 7,
    sMilli: 2828,
    height: 42,
    epoch: 0,
  });
  assert.equal(p.method, "submit_reading");
  assert.deepEqual(p.args, [2828, 42]);
});

test("view payloads are no-arg with correct method names", () => {
  assert.equal(
    isCertifiedNowPayload({ caller: 1, contractId: 7, epoch: 0 }).method,
    "is_certified_now",
  );
  assert.deepEqual(
    isCertifiedNowPayload({ caller: 1, contractId: 7, epoch: 0 }).args,
    [],
  );

  assert.equal(
    latestSMilliPayload({ caller: 1, contractId: 7, epoch: 0 }).method,
    "latest_s_milli_view",
  );
  assert.equal(
    lastHeightPayload({ caller: 1, contractId: 7, epoch: 0 }).method,
    "last_height",
  );
  assert.equal(
    acceptedTotalPayload({ caller: 1, contractId: 7, epoch: 0 }).method,
    "accepted_total",
  );
});

test("LOCAL_REALISM_FLOOR_MILLI matches the contract default", () => {
  assert.equal(LOCAL_REALISM_FLOOR_MILLI, 2000);
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
  assert.equal(BELL_LATEST_PATH, "/api/bell/latest");
});

test("isSubmissionWorthy rejects null / no_data / un-certified / sub-floor / stale", () => {
  // null (network error)
  assert.equal(isSubmissionWorthy(null, 0), false);

  // no_data (devnet, no measurements yet)
  assert.equal(isSubmissionWorthy({ status: "no_data" }, 0), false);

  // ok but bell_certified=false
  assert.equal(
    isSubmissionWorthy(
      { status: "ok", s_value_milli: 2500, threshold_milli: 2000, bell_certified: false, height: 1 },
      0,
    ),
    false,
  );

  // ok + certified but s_milli exactly at floor (gate is strict)
  assert.equal(
    isSubmissionWorthy(
      { status: "ok", s_value_milli: 2000, threshold_milli: 2000, bell_certified: true, height: 1 },
      0,
    ),
    false,
  );

  // stale height (not strictly greater than lastPosted)
  assert.equal(
    isSubmissionWorthy(
      { status: "ok", s_value_milli: 2828, threshold_milli: 2000, bell_certified: true, height: 5 },
      5,
    ),
    false,
  );

  // happy path
  assert.equal(
    isSubmissionWorthy(
      { status: "ok", s_value_milli: 2828, threshold_milli: 2000, bell_certified: true, height: 6 },
      5,
    ),
    true,
  );

  // happy path with alt 'block_height' field name
  assert.equal(
    isSubmissionWorthy(
      { status: "ok", s_value_milli: 2828, threshold_milli: 2000, bell_certified: true, block_height: 6 },
      5,
    ),
    true,
  );
});

test("BELL_ORACLE_SOURCE contains all the method names + lifecycle hooks", () => {
  for (const name of [
    "arm",
    "submit_reading",
    "is_certified_now",
    "latest_s_milli_view",
    "is_fresh",
    "rejected_below_floor",
    "rejected_stale_height",
    "on_grace",
    "on_refresh",
    "on_evaporate",
  ]) {
    assert.ok(
      BELL_ORACLE_SOURCE.includes(name),
      `BELL_ORACLE_SOURCE missing identifier: ${name}`,
    );
  }
});

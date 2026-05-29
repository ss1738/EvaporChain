import { test } from "node:test";
import assert from "node:assert/strict";
import {
  energyAtEpoch,
  isValidAt,
  expiryEpochOffset,
  type PassState,
} from "../src/validity.ts";
import {
  deployPayload,
  issuePayload,
  revokePayload,
  isValidPayload,
  requireValidPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { DECAY_ACCESS_PASS_SOURCE } from "../src/contract.ts";

test("energyAtEpoch is a byte-exact port of the on-chain decay curve", () => {
  assert.equal(energyAtEpoch(1_000_000n, 100n, 0n), 1_000_000n);
  assert.equal(energyAtEpoch(1_000_000n, 100n, 100n), 500_000n); // one half-life
  assert.equal(energyAtEpoch(1_000_000n, 100n, 200n), 250_000n); // two half-lives
  assert.equal(energyAtEpoch(1_000_000n, 100n, 260n), 175_000n); // within-halving interp
  assert.equal(energyAtEpoch(1_000_000n, 100n, 10_000n), 0n); // fully decayed
  assert.equal(energyAtEpoch(1_000_000n, 0n, 5n), 0n); // zero half-life
});

test("isValidAt mirrors the contract's is_valid gate", () => {
  const base: PassState = {
    energy: 1_000_000n,
    halfLife: 100n,
    validityFloor: 250_000n,
    sealed: true,
    revoked: false,
  };
  assert.equal(isValidAt(base, 0n), true); // 1M >= 250k
  assert.equal(isValidAt(base, 200n), true); // exactly at floor
  assert.equal(isValidAt(base, 260n), false); // 175k < 250k → expired
  assert.equal(isValidAt({ ...base, revoked: true }, 0n), false); // revoked
  assert.equal(isValidAt({ ...base, sealed: false }, 0n), false); // unissued
});

test("expiryEpochOffset forecasts the floor crossing", () => {
  const e = expiryEpochOffset(1_000_000n, 100n, 250_000n);
  assert.equal(typeof e, "number");
  // e is the FIRST epoch strictly below the floor; e-1 is still valid.
  assert.ok(energyAtEpoch(1_000_000n, 100n, BigInt(e as number)) < 250_000n);
  assert.ok(energyAtEpoch(1_000_000n, 100n, BigInt((e as number) - 1)) >= 250_000n);
  assert.equal(expiryEpochOffset(1_000_000n, 100n, 0n), null); // floor 0 → never expires
  assert.equal(expiryEpochOffset(100n, 100n, 200n), 0); // floor > initial → born invalid
});

test("deployPayload carries the contract source + params", () => {
  const p = deployPayload({ deployer: 1, energy: 1000, halfLife: 100 });
  assert.equal(p.source_code, DECAY_ACCESS_PASS_SOURCE);
  assert.equal(p.energy, 1000);
  assert.equal(p.half_life, 100);
  assert.equal(p.deployer, 1);
});

test("call payloads have the right method + args shape", () => {
  const iss = issuePayload({ caller: 1, contractId: 7, holderHex: "0x22", floor: 250, epoch: 0 });
  assert.equal(iss.method, "issue");
  assert.deepEqual(iss.args, ["0x22", 250]);
  assert.equal(iss.contract_id, 7);

  assert.equal(revokePayload({ caller: 1, contractId: 7, epoch: 0 }).method, "revoke");
  assert.deepEqual(revokePayload({ caller: 1, contractId: 7, epoch: 0 }).args, []);
  assert.equal(isValidPayload({ caller: 1, contractId: 7, epoch: 5 }).method, "is_valid");

  const rv = requireValidPayload({ caller: 2, contractId: 7, epoch: 5 });
  assert.equal(rv.method, "require_valid");
  assert.equal(rv.caller, 2);
});

test("endpoint paths match the node tx API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

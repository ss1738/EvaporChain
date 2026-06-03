// OracleFeed — chain client. Generic decaying oracle: the feed IS a
// decaying contract, `max_age` is a hard ceiling on read-time
// freshness, `is_fresh()` flips false structurally rather than by
// consumer convention. on_evaporate ends the publication surface;
// consumers who depended on the feed must rebind to a fresh one.
//
// Doctrine inversion: standard oracles publish `(value, timestamp)`
// and force every consumer to remember to check staleness; OracleFeed
// makes "no value" and "value past max_age" structurally !fresh, so
// the only way to consume a stale or unset feed is to explicitly
// ignore `is_fresh()` — much harder to do silently.
//
// Operator surface (caller == owner):
//   - set_feed(label, max_age)  one-shot
//   - update(value)             at any cadence
//
// Open surface:
//   - dispute()                 public counter; arbitration is paired
//
// Read surface (anyone):
//   - latest()                  reverts when value_set == false
//   - age()                     0 when value_set == false
//   - is_fresh()                false when value_set == false
//   - feed_label / updates_total / disputes_total / last_updated

import { ORACLE_FEED_SOURCE } from "./contract.ts";

export const DEPLOY_PATH = "/api/tx/deploy-script";
export const CALL_PATH = "/api/tx/call-script";

export interface DeployPayload {
  deployer: number;
  source_code: string;
  energy: number;
  half_life: number;
}

export interface CallPayload {
  caller: number;
  contract_id: number;
  method: string;
  args: Array<string | number>;
  epoch: number;
}

export function deployPayload(opts: {
  deployer: number;
  energy: number;
  halfLife: number;
}): DeployPayload {
  return {
    deployer: opts.deployer,
    source_code: ORACLE_FEED_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Operator-only, one-shot: lock the feed's `label` + `max_age`.
 *  After this call the feed is `sealed`; both fields are immutable
 *  for the contract's lifetime. */
export function setFeedPayload(opts: {
  caller: number;
  contractId: number;
  feedLabel: string;
  freshnessWindow: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "set_feed",
    args: [opts.feedLabel, opts.freshnessWindow],
    epoch: opts.epoch,
  };
}

/** Operator-only: publish a new value. Stamps `updated_at_epoch` and
 *  bumps `update_count`. Disputes are unaffected. */
export function updatePayload(opts: {
  caller: number;
  contractId: number;
  newValue: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "update",
    args: [opts.newValue],
    epoch: opts.epoch,
  };
}

/** Open call: anyone may dispute. The counter is a public signal;
 *  arbitration belongs in a paired governance contract. */
export function disputePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("dispute", opts.caller, opts.contractId, opts.epoch);
}

// ── Views ────────────────────────────────────────────────────────

/** View: latest value. Reverts when no value has been published
 *  (structural alternative to a sentinel return — consumers can't
 *  silently consume an unset feed). */
export function latestPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("latest", opts.caller, opts.contractId, opts.epoch);
}

/** View: epochs since the last `update()`. Returns 0 when no value
 *  has ever been published. */
export function agePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("age", opts.caller, opts.contractId, opts.epoch);
}

/** View: `value_set AND age <= max_age`. Returns false pre-update
 *  even when `max_age` is huge — "no value" is structurally !fresh. */
export function isFreshPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_fresh", opts.caller, opts.contractId, opts.epoch);
}

/** View: the feed's label (operator-supplied at `set_feed()` time). */
export function feedLabelPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("feed_label", opts.caller, opts.contractId, opts.epoch);
}

/** View: cumulative count of `update()` calls. */
export function updatesTotalPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("updates_total", opts.caller, opts.contractId, opts.epoch);
}

/** View: cumulative count of `dispute()` calls. */
export function disputesTotalPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("disputes_total", opts.caller, opts.contractId, opts.epoch);
}

/** View: epoch of the most recent `update()` (0 if none yet). */
export function lastUpdatedPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("last_updated", opts.caller, opts.contractId, opts.epoch);
}

function noArgCall(method: string, caller: number, contractId: number, epoch: number): CallPayload {
  return { caller, contract_id: contractId, method, args: [], epoch };
}

// Auth-injected POST: reads the session token from localStorage
// (set by `dapps/wallet/`) and adds the Authorization header.
// See `dapps/shared/auth.ts` for the contract.
import { authedPost, type TxResponse } from "../../shared/auth.ts";
export type { TxResponse };

const post = authedPost;

export const deployTx = (baseUrl: string, o: Parameters<typeof deployPayload>[0]) =>
  post(baseUrl, DEPLOY_PATH, deployPayload(o));
export const setFeedTx = (baseUrl: string, o: Parameters<typeof setFeedPayload>[0]) =>
  post(baseUrl, CALL_PATH, setFeedPayload(o));
export const updateTx = (baseUrl: string, o: Parameters<typeof updatePayload>[0]) =>
  post(baseUrl, CALL_PATH, updatePayload(o));
export const disputeTx = (baseUrl: string, o: Parameters<typeof disputePayload>[0]) =>
  post(baseUrl, CALL_PATH, disputePayload(o));

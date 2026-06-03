// EvaporCashNote — chain client. Native demurrage bearer-note (Money
// lane): ONE note = ONE contract instance; the note's own `energy`
// builtin IS its spendable value, so a hoarded note loses value by
// chain physics with no keeper bot, no in-contract decay formula, and
// no off-chain timer. The Wörgl / Gesell incentive native.
//
// Lifecycle:
//   1. Deployer deploys with energy = face value, half_life = demurrage
//      rate. The act of deploying IS the act of funding the note.
//   2. Deployer (caller == owner) calls one-shot `issue(to, face_value)`
//      to bind the bearer. `face_value` is the issue-time snapshot
//      (accounting only); the spendable value is the live `energy`.
//   3. Current holder calls one-shot `spend(to)` to retire THIS note;
//      the off-chain coordinator reads the emit and reissues a fresh
//      note carrying the live value to `to`. Circulating BEFORE
//      evaporation is how the holder preserves value.
//   4. If never spent before energy decays out, on_evaporate emits
//      "value lost to hoarding" — demurrage taken to its physical limit.
//
// Two-value separation (doctrine cornerstone):
//   - `face_value()`  — issue-time snapshot, NEVER tracks decay (accounting)
//   - `live_value()`  — reads the `energy` builtin at call time (spendable)
//
// Off-chain coordinator contract:
//   - subscribes to "note spent" emits → reissues fresh note to recipient
//   - subscribes to "note evaporated — value lost to hoarding" emits →
//     records the forfeiture (no payout)

import { EVAPORCASH_NOTE_SOURCE } from "./contract.ts";

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
    source_code: EVAPORCASH_NOTE_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Issuer-only, one-shot: bind the bearer + lock the face snapshot.
 *  The deployer (caller == owner) supplies the bearer address `to` and
 *  the `faceValue` accounting snapshot. After this call the note is
 *  `sealed`. */
export function issuePayload(opts: {
  caller: number;
  contractId: number;
  toHex: string;
  faceValue: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "issue",
    args: [opts.toHex, opts.faceValue],
    epoch: opts.epoch,
  };
}

/** Holder-only, one-shot: retire THIS note and transfer the right-to-
 *  reissue to `to`. The off-chain coordinator reads the emit and
 *  reissues a fresh note carrying the live value. Spending BEFORE the
 *  energy decays out is how the holder preserves value — the Wörgl /
 *  Gesell incentive made structural. */
export function spendPayload(opts: {
  caller: number;
  contractId: number;
  toHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "spend",
    args: [opts.toHex],
    epoch: opts.epoch,
  };
}

// ── Views ────────────────────────────────────────────────────────

/** View: current bearer address. */
export function currentHolderPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("current_holder", opts.caller, opts.contractId, opts.epoch);
}

/** View: has this note already been spent? */
export function isSpentPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_spent", opts.caller, opts.contractId, opts.epoch);
}

/** View: issue-time face snapshot (accounting only — NEVER tracks decay). */
export function faceValuePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("face_value", opts.caller, opts.contractId, opts.epoch);
}

/** View: spendable value RIGHT NOW. Reads the chain's `energy`
 *  builtin at call time, decayed by the evaporation engine. A hoarded
 *  note's live_value bleeds toward zero by physics. */
export function liveValuePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("live_value", opts.caller, opts.contractId, opts.epoch);
}

/** View: the epoch at which `issue()` ran. */
export function issuedEpochPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("issued_epoch", opts.caller, opts.contractId, opts.epoch);
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
export const issueTx = (baseUrl: string, o: Parameters<typeof issuePayload>[0]) =>
  post(baseUrl, CALL_PATH, issuePayload(o));
export const spendTx = (baseUrl: string, o: Parameters<typeof spendPayload>[0]) =>
  post(baseUrl, CALL_PATH, spendPayload(o));

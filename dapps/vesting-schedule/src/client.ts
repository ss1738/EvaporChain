// VestingSchedule — chain client. Classic linear vest with cliff,
// with a doctrine-novel twist: the post-vest claim window is bounded
// by the contract's own energy. If the beneficiary stops claiming
// and the contract evaporates, on_evaporate stamps vested_at_evaporate
// and flips forfeit_signaled so the off-chain coordinator returns the
// unclaimed remainder to the grantor.
//
// Two roles:
//
//   (a) Grantor (deployer/owner) — calls set_terms(beneficiary,
//       grant, cliff, duration) exactly once to arm. May cancel()
//       BEFORE the beneficiary's first claim (post-claim the
//       schedule is immutable; chain becomes the source of truth).
//
//   (b) Beneficiary — calls claim() periodically; returns the delta
//       between vested-now and claimed-so-far. Monotonic
//       claimed_amount.
//
// The vest math (cliff + linear ramp) is duplicated across vested_now,
// claim, vested_amount, pending_amount, and on_evaporate because
// EvaporScript V1/V2 has no contract-internal method dispatch. VEST-1
// (audit 2026-05-17): all five sites use division-first arithmetic to
// avoid u64 overflow at large grants.

import { VESTING_SCHEDULE_SOURCE } from "./contract.ts";

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
    source_code: VESTING_SCHEDULE_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Grantor-only, one-shot: arm the vesting schedule.
 *  - `grant` must be positive
 *  - `duration` must be positive
 *  - `cliff` must be <= `duration` (cliff > duration would never vest)
 *  `start_epoch` is captured at the moment of set_terms; cliff_at +
 *  fully_vested_at are computed from that start. */
export function setTermsPayload(opts: {
  caller: number;
  contractId: number;
  beneficiaryHex: string;
  grant: number;
  cliffEpochs: number;
  durationEpochs: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "set_terms",
    args: [opts.beneficiaryHex, opts.grant, opts.cliffEpochs, opts.durationEpochs],
    epoch: opts.epoch,
  };
}

/** Beneficiary-only: claim the delta between vested-now and
 *  claimed-so-far. Returns the delta. Monotonic — claimed_amount
 *  never decreases. Reverts with "nothing to claim" if vested ==
 *  claimed (e.g., calling before the cliff). */
export function claimPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("claim", opts.caller, opts.contractId, opts.epoch);
}

/** Grantor-only: cancel the schedule. Allowed only while
 *  `claimed_amount == 0` — once the beneficiary has touched the
 *  grant the schedule is immutable. One-shot. */
export function cancelPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("cancel", opts.caller, opts.contractId, opts.epoch);
}

// ── Views ────────────────────────────────────────────────────────

/** View: cumulative vested amount as of current epoch (regardless of
 *  what the beneficiary has actually claimed). */
export function vestedNowPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("vested_now", opts.caller, opts.contractId, opts.epoch);
}

/** View: alias for `vested_now`; named separately to distinguish
 *  documentation cases ("vested" cumulative vs "pending" claimable). */
export function vestedAmountPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("vested_amount", opts.caller, opts.contractId, opts.epoch);
}

/** View: vested-but-not-yet-claimed (what `claim()` would return). */
export function pendingAmountPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("pending_amount", opts.caller, opts.contractId, opts.epoch);
}

/** View: who the schedule pays out to. */
export function beneficiaryOfPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("beneficiary_of", opts.caller, opts.contractId, opts.epoch);
}

/** View: total grant amount. */
export function grantTotalPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("grant_total", opts.caller, opts.contractId, opts.epoch);
}

/** View: epoch at which the cliff ends (`start_epoch + cliff_epochs`). */
export function cliffAtPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("cliff_at", opts.caller, opts.contractId, opts.epoch);
}

/** View: epoch at which the full grant is vested
 *  (`start_epoch + duration_epochs`). */
export function fullyVestedAtPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("fully_vested_at", opts.caller, opts.contractId, opts.epoch);
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
export const setTermsTx = (baseUrl: string, o: Parameters<typeof setTermsPayload>[0]) =>
  post(baseUrl, CALL_PATH, setTermsPayload(o));
export const claimTx = (baseUrl: string, o: Parameters<typeof claimPayload>[0]) =>
  post(baseUrl, CALL_PATH, claimPayload(o));
export const cancelTx = (baseUrl: string, o: Parameters<typeof cancelPayload>[0]) =>
  post(baseUrl, CALL_PATH, cancelPayload(o));

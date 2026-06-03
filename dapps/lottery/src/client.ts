// Lottery — chain client. Single-draw lottery with chain-VRF winner
// selection. Operator configures `prize` + `stake` once (sealed),
// opens enrolment, any address enters exactly once, operator triggers
// the draw; the winning index is `random_range(entry_count)` derived
// from the chain's VRF beacon (LOTTERY-1, audit 2026-05-17 — operator
// influence is restricted to WHEN, never WHO).
//
// Doctrine: unresolved at evaporation = `voided = true`; the
// coordinator reads the void flag and refunds entries off-chain.
// Same chain-as-keeper pattern as the rest of the Marketplace escrow
// family — no off-chain reaper, no rescue contract.
//
// Phase machine (implicit; not a numbered phase variable):
//   pre-set_event   → configuration not sealed
//   sealed, not drawn → enrolment open
//   drawn, not claimed → winner can pull
//   drawn + claimed → terminal-success
//   evaporated, not drawn → voided

import { LOTTERY_SOURCE } from "./contract.ts";

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
    source_code: LOTTERY_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Operator-only, one-shot: lock `prize` + `stake`. After this call
 *  the lottery is `sealed`; enrolment opens immediately. */
export function setEventPayload(opts: {
  caller: number;
  contractId: number;
  prizeAmount: number;
  stakeAmount: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "set_event",
    args: [opts.prizeAmount, opts.stakeAmount],
    epoch: opts.epoch,
  };
}

/** Open call: enter the lottery. One entry per address; the
 *  `entered` map deduplicates. The parallel `entry_by_index` map is
 *  stamped BEFORE the counter increments so `draw()` can look up by
 *  the random index (LOTTERY-1). */
export function enterPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("enter", opts.caller, opts.contractId, opts.epoch);
}

/** Operator-only: trigger the draw. The winning index is
 *  `random_range(entry_count)` derived from the chain's VRF beacon —
 *  the operator can choose WHEN to draw, never WHO wins. One-shot. */
export function drawPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("draw", opts.caller, opts.contractId, opts.epoch);
}

/** Winner-only, one-shot: claim the prize. Returns the prize amount
 *  for the coordinator to settle off-chain. The contract gates on
 *  `caller == self.winner` + `claimed == false`. */
export function claimPrizePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("claim_prize", opts.caller, opts.contractId, opts.epoch);
}

// ── Views ────────────────────────────────────────────────────────

/** View: total entries recorded (== `entry_count`). */
export function entriesTotalPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("entries_total", opts.caller, opts.contractId, opts.epoch);
}

/** View: has `who` entered? Returns true if `entered[who] > 0`. */
export function isEnteredPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "is_entered",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: the drawn winner's address (zero address if not yet drawn). */
export function winnerOfPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("winner_of", opts.caller, opts.contractId, opts.epoch);
}

/** View: has the draw happened? */
export function isDrawnPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_drawn", opts.caller, opts.contractId, opts.epoch);
}

/** View: was the lottery voided by evaporation? */
export function isVoidedPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_voided", opts.caller, opts.contractId, opts.epoch);
}

/** View: the prize amount (0 if not yet configured). */
export function prizeSizePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("prize_size", opts.caller, opts.contractId, opts.epoch);
}

/** View: the per-entry stake (0 if not yet configured). */
export function stakePerEntryPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("stake_per_entry", opts.caller, opts.contractId, opts.epoch);
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
export const setEventTx = (baseUrl: string, o: Parameters<typeof setEventPayload>[0]) =>
  post(baseUrl, CALL_PATH, setEventPayload(o));
export const enterTx = (baseUrl: string, o: Parameters<typeof enterPayload>[0]) =>
  post(baseUrl, CALL_PATH, enterPayload(o));
export const drawTx = (baseUrl: string, o: Parameters<typeof drawPayload>[0]) =>
  post(baseUrl, CALL_PATH, drawPayload(o));
export const claimPrizeTx = (baseUrl: string, o: Parameters<typeof claimPrizePayload>[0]) =>
  post(baseUrl, CALL_PATH, claimPrizePayload(o));

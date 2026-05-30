// WitnessFit — chain client. Single-user-per-contract: the deployer
// is the wearer; only they call `check_in()` / `reset_peak()`.

import { WITNESSFIT_SOURCE } from "./contract.ts";

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

/** Deploy a fresh witnessfit contract. The deployer becomes the
 *  wearer (the only address allowed to call check_in / reset_peak). */
export function deployPayload(opts: { deployer: number; energy: number; halfLife: number }): DeployPayload {
  return {
    deployer: opts.deployer,
    source_code: WITNESSFIT_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Wearer-only: record a check-in at the given epoch. */
export function checkInPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("check_in", opts.caller, opts.contractId, opts.epoch);
}

/** Wearer-only: declare a new chapter — peak drops to the current
 *  streak. Doesn't touch the current streak counter. */
export function resetPeakPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("reset_peak", opts.caller, opts.contractId, opts.epoch);
}

/** View: current streak (decay-aware — returns 0 if window elapsed). */
export function currentStreakPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("current_streak", opts.caller, opts.contractId, opts.epoch);
}

/** View: boost gate — true iff current streak ≥ threshold % of peak. */
export function hasBoostPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("has_boost", opts.caller, opts.contractId, opts.epoch);
}

/** View: epochs left before the next check-in would reset the streak. */
export function windowRemainingPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("window_remaining", opts.caller, opts.contractId, opts.epoch);
}

/** View: historical peak. */
export function peakPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("peak", opts.caller, opts.contractId, opts.epoch);
}

function noArgCall(method: string, caller: number, contractId: number, epoch: number): CallPayload {
  return { caller, contract_id: contractId, method, args: [], epoch };
}

export interface TxResponse {
  success: boolean;
  tx_hash?: string;
  message: string;
}

async function post(baseUrl: string, path: string, body: unknown): Promise<TxResponse> {
  const res = await fetch(`${baseUrl}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  return (await res.json()) as TxResponse;
}

export const deployTx = (baseUrl: string, o: Parameters<typeof deployPayload>[0]) =>
  post(baseUrl, DEPLOY_PATH, deployPayload(o));
export const checkInTx = (baseUrl: string, o: Parameters<typeof checkInPayload>[0]) =>
  post(baseUrl, CALL_PATH, checkInPayload(o));
export const resetPeakTx = (baseUrl: string, o: Parameters<typeof resetPeakPayload>[0]) =>
  post(baseUrl, CALL_PATH, resetPeakPayload(o));

// Mayfly — chain client. Deploy short-half-life contracts (default
// `half_life=10`); hatch with the metadata; transfer between holders
// while the contract is alive. There's no "kill" call — the contract
// just decays out naturally.

import { MAYFLY_SOURCE } from "./contract.ts";

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

/** Deploy. Catalogue defaults: energy=1000, half_life=10 (finishes
 *  in ~100 epochs). Override to make a longer-lived NFT — but at
 *  some point it stops being a mayfly. */
export function deployPayload(opts: { deployer: number; energy: number; halfLife: number }): DeployPayload {
  return {
    deployer: opts.deployer,
    source_code: MAYFLY_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Owner-only: seal metadata and become the first holder. */
export function hatchPayload(opts: {
  caller: number;
  contractId: number;
  metadata: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "hatch",
    args: [opts.metadata],
    epoch: opts.epoch,
  };
}

/** Current holder transfers to `to`. */
export function transferPayload(opts: {
  caller: number;
  contractId: number;
  toHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "transfer",
    args: [opts.toHex],
    epoch: opts.epoch,
  };
}

/** View: metadata (reverts pre-hatch). */
export function readMetadataPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("read_metadata", opts.caller, opts.contractId, opts.epoch);
}

/** View: is `who` the current holder? */
export function isHolderPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "is_holder",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: age in epochs since hatch (0 pre-hatch). */
export function ageEpochsPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("age_epochs", opts.caller, opts.contractId, opts.epoch);
}

/** View: how many transfers have happened. */
export function transfersTotalPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("transfers_total", opts.caller, opts.contractId, opts.epoch);
}

/** View: has the contract been hatched yet? */
export function isHatchedPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_hatched", opts.caller, opts.contractId, opts.epoch);
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
export const hatchTx = (baseUrl: string, o: Parameters<typeof hatchPayload>[0]) =>
  post(baseUrl, CALL_PATH, hatchPayload(o));
export const transferTx = (baseUrl: string, o: Parameters<typeof transferPayload>[0]) =>
  post(baseUrl, CALL_PATH, transferPayload(o));

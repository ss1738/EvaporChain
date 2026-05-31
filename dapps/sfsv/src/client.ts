// SFSV — chain client. Three roles:
//
//   (a) Depositor (owner) — deploys + arm()s with (future_self,
//       amount, release_epoch). The deposit is recorded on-chain;
//       the actual token movement happens in a paired tx.
//   (b) Current beneficiary — starts as future_self; may call
//       sell(buyer) one time to transfer the claim. After
//       release_epoch, calls withdraw().
//   (c) Anyone — queries is_releasable(), is_beneficiary(who),
//       epochs_until_release() to drive the off-chain auction UX.

import { SFSV_SOURCE } from "./contract.ts";

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

export function deployPayload(opts: { deployer: number; energy: number; halfLife: number }): DeployPayload {
  return {
    deployer: opts.deployer,
    source_code: SFSV_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Owner-only, one-shot: arm with (future_self, deposit_amount, release_epoch). */
export function armPayload(opts: {
  caller: number;
  contractId: number;
  futureSelfHex: string;
  depositAmount: number;
  releaseEpoch: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "arm",
    args: [opts.futureSelfHex, opts.depositAmount, opts.releaseEpoch],
    epoch: opts.epoch,
  };
}

/** Current beneficiary: sell the claim to a buyer. One-shot. */
export function sellPayload(opts: {
  caller: number;
  contractId: number;
  buyerHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "sell",
    args: [opts.buyerHex],
    epoch: opts.epoch,
  };
}

/** Current beneficiary: withdraw after release_epoch. */
export function withdrawPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("withdraw", opts.caller, opts.contractId, opts.epoch);
}

/** View: composite gate. True iff armed AND not withdrawn AND
 *  epoch ≥ release_epoch. */
export function isReleasablePayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_releasable", opts.caller, opts.contractId, opts.epoch);
}

/** View: epochs left before release; 0 if armed and past release. */
export function epochsUntilReleasePayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("epochs_until_release", opts.caller, opts.contractId, opts.epoch);
}

/** View: is `who` the current beneficiary (can sell or withdraw)? */
export function isBeneficiaryPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "is_beneficiary",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: is `who` the ORIGINAL future-self (audit-trail-stable
 *  even after sell())? */
export function isOriginalFutureSelfPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "is_original_future_self",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

export function depositAmountPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("deposit_amount_view", opts.caller, opts.contractId, opts.epoch);
}

export function releaseAtPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("release_at", opts.caller, opts.contractId, opts.epoch);
}

export function isArmedPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_armed", opts.caller, opts.contractId, opts.epoch);
}

export function isSoldPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_sold", opts.caller, opts.contractId, opts.epoch);
}

export function isWithdrawnPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_withdrawn", opts.caller, opts.contractId, opts.epoch);
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
export const armTx = (baseUrl: string, o: Parameters<typeof armPayload>[0]) =>
  post(baseUrl, CALL_PATH, armPayload(o));
export const sellTx = (baseUrl: string, o: Parameters<typeof sellPayload>[0]) =>
  post(baseUrl, CALL_PATH, sellPayload(o));
export const withdrawTx = (baseUrl: string, o: Parameters<typeof withdrawPayload>[0]) =>
  post(baseUrl, CALL_PATH, withdrawPayload(o));

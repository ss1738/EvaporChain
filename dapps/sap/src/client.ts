// SAP — chain client. Two roles:
//
//   (a) Issuer (owner) — deploys + arm()s with the curve + rate-cap
//       policy, then issue(recipient) to mint AQs.
//   (b) Recipients — redeem() their own AQ once.
//
// View `current_value(who)` to price an AQ in the marketplace
// (linear-decay from `initial_value` to 0 over `2 * half_life`
// epochs). The BigInt port in `./value.ts` mirrors this off-chain
// so UIs preview without a round-trip.

import { SAP_SOURCE } from "./contract.ts";

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
    source_code: SAP_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Owner-only, one-shot: configure the AQ curve + rate cap. */
export function armPayload(opts: {
  caller: number;
  contractId: number;
  initialValue: number;
  halfLife: number;
  maxAqPerWindow: number;
  windowEpochs: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "arm",
    args: [opts.initialValue, opts.halfLife, opts.maxAqPerWindow, opts.windowEpochs],
    epoch: opts.epoch,
  };
}

/** Owner-only: mint an AQ for recipient. Reverts if the recipient
 *  already holds an outstanding AQ or the rate-cap is hit. */
export function issuePayload(opts: {
  caller: number;
  contractId: number;
  recipientHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "issue",
    args: [opts.recipientHex],
    epoch: opts.epoch,
  };
}

/** Recipient: redeem their own AQ. */
export function redeemPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("redeem", opts.caller, opts.contractId, opts.epoch);
}

/** View: current value of `who`'s AQ (post-decay, post-redemption). */
export function currentValuePayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "current_value",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: is `who`'s AQ currently active (minted, not redeemed, not expired)? */
export function hasActiveAqPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "has_active_aq",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: epochs left before `who`'s AQ value hits zero. */
export function epochsUntilExpiryPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "epochs_until_expiry",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: how many AQs have been issued in the current rolling window. */
export function issuedInCurrentWindowPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("issued_in_current_window", opts.caller, opts.contractId, opts.epoch);
}

/** View: how many more AQs can be issued in this window. */
export function slotsLeftInWindowPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("slots_left_in_window", opts.caller, opts.contractId, opts.epoch);
}

export function isArmedPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_armed", opts.caller, opts.contractId, opts.epoch);
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
export const issueTx = (baseUrl: string, o: Parameters<typeof issuePayload>[0]) =>
  post(baseUrl, CALL_PATH, issuePayload(o));
export const redeemTx = (baseUrl: string, o: Parameters<typeof redeemPayload>[0]) =>
  post(baseUrl, CALL_PATH, redeemPayload(o));

// Decay Access Pass — chain client.
//
// Pure `*Payload` builders construct the exact request bodies the node's
// tx endpoints expect (modeled on the mortal-messages dApp), so they're
// unit-testable with no node. Thin `*Tx` wrappers POST them.
//
//   deploy → POST /api/tx/deploy-script {deployer, source_code, energy, half_life}
//   call   → POST /api/tx/call-script   {caller, contract_id, method, args, epoch}

import { DECAY_ACCESS_PASS_SOURCE } from "./contract.ts";

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

/** Deploy a fresh DecayAccessPass: `energy` = initial strength,
 *  `half_life` = decay rate. The pass is unissued until `issue`. */
export function deployPayload(opts: { deployer: number; energy: number; halfLife: number }): DeployPayload {
  return {
    deployer: opts.deployer,
    source_code: DECAY_ACCESS_PASS_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Issue the pass to `holderHex` with a validity floor. Issuer-only. */
export function issuePayload(opts: {
  caller: number;
  contractId: number;
  holderHex: string;
  floor: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "issue",
    args: [opts.holderHex, opts.floor],
    epoch: opts.epoch,
  };
}

function noArgCall(method: string, caller: number, contractId: number, epoch: number): CallPayload {
  return { caller, contract_id: contractId, method, args: [], epoch };
}

/** Revoke (terminal, issuer-only). */
export function revokePayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("revoke", opts.caller, opts.contractId, opts.epoch);
}

/** Read validity (returns bool on-chain). */
export function isValidPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_valid", opts.caller, opts.contractId, opts.epoch);
}

/** Exercise the pass — reverts unless the caller is the holder and valid. */
export function requireValidPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("require_valid", opts.caller, opts.contractId, opts.epoch);
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
export const revokeTx = (baseUrl: string, o: Parameters<typeof revokePayload>[0]) =>
  post(baseUrl, CALL_PATH, revokePayload(o));
export const isValidTx = (baseUrl: string, o: Parameters<typeof isValidPayload>[0]) =>
  post(baseUrl, CALL_PATH, isValidPayload(o));
export const requireValidTx = (baseUrl: string, o: Parameters<typeof requireValidPayload>[0]) =>
  post(baseUrl, CALL_PATH, requireValidPayload(o));

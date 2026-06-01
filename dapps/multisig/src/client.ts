// Multisig — chain client. Lifecycle:
//
//   1. Deploy: owner sets up the contract instance.
//   2. Owner-only pre-propose: add_signer(addr) for each signer;
//      set_threshold(t).
//   3. Owner-only seal: propose(action_spec) locks the configuration;
//      after this, no more signers and no threshold changes.
//   4. Signers sign: each registered signer calls sign() once.
//   5. Anyone executes once signature_count >= threshold: execute().
//   6. On evaporation without execute: expired = true (the decision
//      lapsed; no follow-up resurrects it).
//
// Doctrine claim: one contract = one decision. Gnosis-Safe-style
// proposal-maps conflate the signer set with the proposal stream;
// EvaporChain inverts that — the contract IS the proposal.
// Multiple decisions = multiple contracts, evaporating independently.

import { MULTISIG_SOURCE } from "./contract.ts";

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
    source_code: MULTISIG_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Owner-only, pre-propose: register a signer. Duplicates rejected. */
export function addSignerPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "add_signer",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** Owner-only, pre-propose: set the approval threshold. Must be
 *  positive and not exceed the signer count (a threshold no quorum
 *  can satisfy would brick the contract). */
export function setThresholdPayload(opts: {
  caller: number;
  contractId: number;
  threshold: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "set_threshold",
    args: [opts.threshold],
    epoch: opts.epoch,
  };
}

/** Owner-only: seal the configuration with a proposal action.
 *  After this call, add_signer + set_threshold revert. The action
 *  string is opaque to the contract — the dApp interprets it. */
export function proposePayload(opts: {
  caller: number;
  contractId: number;
  action: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "propose",
    args: [opts.action],
    epoch: opts.epoch,
  };
}

/** Registered signer adds their signature. One per signer; reverts
 *  on duplicate or post-execute. */
export function signPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("sign", opts.caller, opts.contractId, opts.epoch);
}

/** Anyone may trigger execution once the threshold is reached.
 *  The contract records the authorisation; the dApp performs the
 *  off-chain side effect. */
export function executePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("execute", opts.caller, opts.contractId, opts.epoch);
}

// ── Views ────────────────────────────────────────────────────────

export function signersTotalPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("signers_total", opts.caller, opts.contractId, opts.epoch);
}

export function thresholdRequiredPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("threshold_required", opts.caller, opts.contractId, opts.epoch);
}

export function signaturesCollectedPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("signatures_collected", opts.caller, opts.contractId, opts.epoch);
}

export function hasSignedPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "has_signed",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

export function isSignerPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "is_signer",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

export function proposalActionPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("proposal_action", opts.caller, opts.contractId, opts.epoch);
}

export function isExecutedPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_executed", opts.caller, opts.contractId, opts.epoch);
}

export function isPendingPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_pending", opts.caller, opts.contractId, opts.epoch);
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
export const addSignerTx = (baseUrl: string, o: Parameters<typeof addSignerPayload>[0]) =>
  post(baseUrl, CALL_PATH, addSignerPayload(o));
export const setThresholdTx = (baseUrl: string, o: Parameters<typeof setThresholdPayload>[0]) =>
  post(baseUrl, CALL_PATH, setThresholdPayload(o));
export const proposeTx = (baseUrl: string, o: Parameters<typeof proposePayload>[0]) =>
  post(baseUrl, CALL_PATH, proposePayload(o));
export const signTx = (baseUrl: string, o: Parameters<typeof signPayload>[0]) =>
  post(baseUrl, CALL_PATH, signPayload(o));
export const executeTx = (baseUrl: string, o: Parameters<typeof executePayload>[0]) =>
  post(baseUrl, CALL_PATH, executePayload(o));

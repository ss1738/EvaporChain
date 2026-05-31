// ChildKey — chain client. Three roles:
//
//   (a) Writer / owner — deploys, registers committee members
//       (add_committee_member, multisig-style — one call per member),
//       then arm() seals the recipient + unlock_epoch + content_hash
//       + threshold. After arm() the contract is immutable.
//
//   (b) Committee members — vote_emergency() to escalate an early
//       unlock. Each member votes at most once.
//
//   (c) Anyone — finalize_natural_unlock() once epoch ≥ unlock_epoch,
//       finalize_emergency_unlock() once vote_count ≥ threshold.
//       Gas sits with whoever wants to read first.
//
// Reading: recipient OR committee post-unlock returns content_hash.
// Cleartext content is off-chain; the chain holds only the hash.

import { CHILDKEY_SOURCE } from "./contract.ts";

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
    source_code: CHILDKEY_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Owner-only, pre-arm: register a committee member. */
export function addCommitteeMemberPayload(opts: {
  caller: number;
  contractId: number;
  memberHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "add_committee_member",
    args: [opts.memberHex],
    epoch: opts.epoch,
  };
}

/** Owner-only, one-shot: arm with recipient + unlock epoch + content
 *  hash + emergency threshold. After this call the contract is
 *  immutable until unlocked. */
export function armPayload(opts: {
  caller: number;
  contractId: number;
  recipientHex: string;
  unlockEpoch: number;
  contentHash: string;
  threshold: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "arm",
    args: [opts.recipientHex, opts.unlockEpoch, opts.contentHash, opts.threshold],
    epoch: opts.epoch,
  };
}

/** Committee-only: cast an emergency-unlock vote. */
export function voteEmergencyPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("vote_emergency", opts.caller, opts.contractId, opts.epoch);
}

/** Anyone: finalize the emergency unlock once threshold votes are in. */
export function finalizeEmergencyUnlockPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("finalize_emergency_unlock", opts.caller, opts.contractId, opts.epoch);
}

/** Anyone: finalize the natural unlock once epoch ≥ unlock_epoch. */
export function finalizeNaturalUnlockPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("finalize_natural_unlock", opts.caller, opts.contractId, opts.epoch);
}

/** Recipient or committee, post-unlock: read the content hash. */
export function readContentPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("read_content", opts.caller, opts.contractId, opts.epoch);
}

/** View: is `who` a committee member? */
export function isCommitteeMemberPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "is_committee_member",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: vote_progress / threshold_required / is_armed / is_unlocked / unlock_at / epochs_until_unlock. */
export function voteProgressPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("vote_progress", opts.caller, opts.contractId, opts.epoch);
}

export function thresholdRequiredPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("threshold_required", opts.caller, opts.contractId, opts.epoch);
}

export function isArmedPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_armed", opts.caller, opts.contractId, opts.epoch);
}

export function isUnlockedPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_unlocked", opts.caller, opts.contractId, opts.epoch);
}

export function unlockAtPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("unlock_at", opts.caller, opts.contractId, opts.epoch);
}

export function epochsUntilUnlockPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("epochs_until_unlock", opts.caller, opts.contractId, opts.epoch);
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
export const addCommitteeMemberTx = (baseUrl: string, o: Parameters<typeof addCommitteeMemberPayload>[0]) =>
  post(baseUrl, CALL_PATH, addCommitteeMemberPayload(o));
export const armTx = (baseUrl: string, o: Parameters<typeof armPayload>[0]) =>
  post(baseUrl, CALL_PATH, armPayload(o));
export const voteEmergencyTx = (baseUrl: string, o: Parameters<typeof voteEmergencyPayload>[0]) =>
  post(baseUrl, CALL_PATH, voteEmergencyPayload(o));
export const finalizeEmergencyUnlockTx = (baseUrl: string, o: Parameters<typeof finalizeEmergencyUnlockPayload>[0]) =>
  post(baseUrl, CALL_PATH, finalizeEmergencyUnlockPayload(o));
export const finalizeNaturalUnlockTx = (baseUrl: string, o: Parameters<typeof finalizeNaturalUnlockPayload>[0]) =>
  post(baseUrl, CALL_PATH, finalizeNaturalUnlockPayload(o));

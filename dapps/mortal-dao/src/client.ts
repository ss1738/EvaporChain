// Mortal-DAO — chain client.
//
// Pure `*Payload` builders construct the exact request bodies the
// node's tx endpoints expect (modeled on dapps/decay-access-pass), so
// they're unit-testable with no node. Thin `*Tx` wrappers POST them.
//
//   deploy → POST /api/tx/deploy-script {deployer, source_code, energy, half_life}
//   call   → POST /api/tx/call-script   {caller, contract_id, method, args, epoch}
//
// Method overview (see contract.ts / mortal_dao.es for the on-chain
// semantics):
//   add_member(who)             owner-only, register founding member
//   refresh_membership()        member-only, reset freshness + cap
//   open_proposal(text)         member-only, one slot at a time
//   vote_for()                  active member, +weight to FOR
//   vote_against()              active member, +weight to AGAINST
//   close_proposal() -> bool    after voting_window elapses; gates on quorum
//
// View methods (read-only):
//   member_count_now, is_member, is_active, weight_of, proposal_open_now,
//   for_count, against_count, weight_collected_now, peak,
//   carried_total, rejected_total, next_id

import { MORTAL_DAO_SOURCE } from "./contract.ts";

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

/** Deploy a fresh MortalDAO. `energy` = DAO's own lifespan, `half_life` =
 *  how fast that energy decays. Defaults align with the catalogue
 *  descriptor (energy=1000, half_life=100). */
export function deployPayload(opts: { deployer: number; energy: number; halfLife: number }): DeployPayload {
  return {
    deployer: opts.deployer,
    source_code: MORTAL_DAO_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Owner-only: register a founding member. */
export function addMemberPayload(opts: {
  caller: number;
  contractId: number;
  memberHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "add_member",
    args: [opts.memberHex],
    epoch: opts.epoch,
  };
}

/** Member-only: refresh staleness clock AND zero the per-member
 *  proposal counter (composes decay-credential + decay-rate-limit). */
export function refreshMembershipPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("refresh_membership", opts.caller, opts.contractId, opts.epoch);
}

/** Active member: open the single proposal slot. Rejected if the
 *  member is stale or has hit proposal_cap since their last refresh. */
export function openProposalPayload(opts: {
  caller: number;
  contractId: number;
  text: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "open_proposal",
    args: [opts.text],
    epoch: opts.epoch,
  };
}

/** Vote in favour. Weight = participations + 1 (decay-reputation). */
export function voteForPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("vote_for", opts.caller, opts.contractId, opts.epoch);
}

/** Vote against. Same weight semantics as vote_for. */
export function voteAgainstPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("vote_against", opts.caller, opts.contractId, opts.epoch);
}

/** Close the active proposal. Reverts before voting_window elapses or
 *  if the collected weight fails the running-peak quorum gate. */
export function closeProposalPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("close_proposal", opts.caller, opts.contractId, opts.epoch);
}

/** View: number of registered members. */
export function memberCountPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("member_count_now", opts.caller, opts.contractId, opts.epoch);
}

/** View: is `who` a registered member (regardless of freshness)? */
export function isMemberPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "is_member",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: is `who` BOTH a member AND fresh (within freshness_window)? */
export function isActivePayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "is_active",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: `who`'s current vote weight (participations + 1). */
export function weightOfPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "weight_of",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
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
export const addMemberTx = (baseUrl: string, o: Parameters<typeof addMemberPayload>[0]) =>
  post(baseUrl, CALL_PATH, addMemberPayload(o));
export const refreshMembershipTx = (baseUrl: string, o: Parameters<typeof refreshMembershipPayload>[0]) =>
  post(baseUrl, CALL_PATH, refreshMembershipPayload(o));
export const openProposalTx = (baseUrl: string, o: Parameters<typeof openProposalPayload>[0]) =>
  post(baseUrl, CALL_PATH, openProposalPayload(o));
export const voteForTx = (baseUrl: string, o: Parameters<typeof voteForPayload>[0]) =>
  post(baseUrl, CALL_PATH, voteForPayload(o));
export const voteAgainstTx = (baseUrl: string, o: Parameters<typeof voteAgainstPayload>[0]) =>
  post(baseUrl, CALL_PATH, voteAgainstPayload(o));
export const closeProposalTx = (baseUrl: string, o: Parameters<typeof closeProposalPayload>[0]) =>
  post(baseUrl, CALL_PATH, closeProposalPayload(o));

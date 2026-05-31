// Bounty — chain client. Three roles:
//
//   (a) Poster (deployer/owner) — deploys + set_bounty(task, reward)
//       once, then accept(winner) when a submission satisfies. May
//       cancel() ONLY before any hunter has submitted (no rug-pull
//       after work has been put in).
//
//   (b) Hunter — submit(solution) on the open call. Resubmissions
//       overwrite the stored solution but don't double-count.
//       Submission persists in chain state as a historical record
//       even without a payout.
//
//   (c) Winner (a specific hunter, after accept()) — claim() the
//       reward. One-shot.
//
// Doctrine claim: an unaccepted bounty refunds to the poster when
// the contract evaporates. on_evaporate flips refunded=true without
// an accepted winner; the off-chain coordinator returns the reward.
// No off-chain liquidator needed — the runtime is the closer.

import { BOUNTY_SOURCE } from "./contract.ts";

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
    source_code: BOUNTY_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Poster-only, one-shot: lock the task spec + reward amount. */
export function setBountyPayload(opts: {
  caller: number;
  contractId: number;
  task: string;
  reward: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "set_bounty",
    args: [opts.task, opts.reward],
    epoch: opts.epoch,
  };
}

/** Any address: submit a solution. Resubmissions overwrite but don't
 *  bump the submission_count (per-hunter idempotent on count). */
export function submitPayload(opts: {
  caller: number;
  contractId: number;
  solution: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "submit",
    args: [opts.solution],
    epoch: opts.epoch,
  };
}

/** Poster-only: declare a winner. The winner address must have at
 *  least one submission on file. One-shot. */
export function acceptPayload(opts: {
  caller: number;
  contractId: number;
  winnerHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "accept",
    args: [opts.winnerHex],
    epoch: opts.epoch,
  };
}

/** Winner-only: pull the reward. One-shot per bounty. Returns the
 *  reward_amount so the off-chain coordinator knows how much to send. */
export function claimPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("claim", opts.caller, opts.contractId, opts.epoch);
}

/** Poster-only: cancel BEFORE any submission exists. Blocked once
 *  the first hunter has submitted — no rug-pull on done work. */
export function cancelPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("cancel", opts.caller, opts.contractId, opts.epoch);
}

// ── Views ────────────────────────────────────────────────────────

/** View: the task spec (free-form string). */
export function taskOfPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("task_of", opts.caller, opts.contractId, opts.epoch);
}

/** View: reward amount. */
export function rewardPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("reward", opts.caller, opts.contractId, opts.epoch);
}

/** View: total distinct submitters so far. */
export function submissionsTotalPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("submissions_total", opts.caller, opts.contractId, opts.epoch);
}

/** View: the submission text for a given hunter. Returns empty
 *  string if the address has never submitted (parallel presence map
 *  guards against EvaporScript's missing-key-returns-zero gotcha). */
export function submissionOfPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "submission_of",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: winner address (zero address if not yet accepted). */
export function winnerOfPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("winner_of", opts.caller, opts.contractId, opts.epoch);
}

/** View: has the poster accepted a winner? */
export function isAcceptedPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_accepted", opts.caller, opts.contractId, opts.epoch);
}

/** View: has the reward been claimed? */
export function isClaimedPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_claimed", opts.caller, opts.contractId, opts.epoch);
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
export const setBountyTx = (baseUrl: string, o: Parameters<typeof setBountyPayload>[0]) =>
  post(baseUrl, CALL_PATH, setBountyPayload(o));
export const submitTx = (baseUrl: string, o: Parameters<typeof submitPayload>[0]) =>
  post(baseUrl, CALL_PATH, submitPayload(o));
export const acceptTx = (baseUrl: string, o: Parameters<typeof acceptPayload>[0]) =>
  post(baseUrl, CALL_PATH, acceptPayload(o));
export const claimTx = (baseUrl: string, o: Parameters<typeof claimPayload>[0]) =>
  post(baseUrl, CALL_PATH, claimPayload(o));
export const cancelTx = (baseUrl: string, o: Parameters<typeof cancelPayload>[0]) =>
  post(baseUrl, CALL_PATH, cancelPayload(o));

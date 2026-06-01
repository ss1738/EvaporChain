// TimeLock — chain client. Three roles:
//
//   (a) Grantor (deployer/owner) — calls set_terms(beneficiary,
//       amount, unlock_epoch) exactly once to arm. May revoke()
//       BEFORE the unlock epoch (post-unlock the promise is
//       irrevocable; the beneficiary's claim window is open).
//
//   (b) Beneficiary — calls claim() at or after unlock_epoch. The
//       call returns the locked amount; one-shot.
//
//   (c) Anyone observing — reads is_unlocked, locked, unlock_at,
//       beneficiary_of for UI status without affecting state.
//
// Doctrine claim: the contract's own energy doubles as the *claim
// window*. If the beneficiary never claims and the contract
// evaporates, on_evaporate flips forfeit_signaled and records
// unclaimed_at_evaporate for the off-chain coordinator to return
// the locked amount to the grantor. No off-chain reaper needed —
// the runtime is the deadline enforcer. Same chain-as-keeper
// doctrine as DEADMAN_SWITCH + SUBSCRIPTION_SERVICE + OPEN_BOUNTY,
// applied to the time-locked vault surface.

import { TIME_LOCK_SOURCE } from "./contract.ts";

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
    source_code: TIME_LOCK_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Grantor-only, one-shot: arm the lock. `unlock` must be strictly
 *  in the future; `amount` must be positive. After this call the
 *  terms are immutable. */
export function setTermsPayload(opts: {
  caller: number;
  contractId: number;
  beneficiaryHex: string;
  amount: number;
  unlockEpoch: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "set_terms",
    args: [opts.beneficiaryHex, opts.amount, opts.unlockEpoch],
    epoch: opts.epoch,
  };
}

/** Beneficiary-only: claim the locked amount once unlock_epoch is
 *  reached. Returns the locked amount; one-shot per lock. */
export function claimPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("claim", opts.caller, opts.contractId, opts.epoch);
}

/** Grantor-only: cancel the lock BEFORE unlock_epoch. Post-unlock
 *  the promise is irrevocable (beneficiary's claim window is open). */
export function revokePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("revoke", opts.caller, opts.contractId, opts.epoch);
}

// ── Views ────────────────────────────────────────────────────────

/** View: who the lock pays out to. */
export function beneficiaryOfPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("beneficiary_of", opts.caller, opts.contractId, opts.epoch);
}

/** View: locked amount (returns 0 post-claim, otherwise the full
 *  amount whether the lock is unlocked yet or not). */
export function lockedPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("locked", opts.caller, opts.contractId, opts.epoch);
}

/** View: the epoch at which the lock unlocks. */
export function unlockAtPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("unlock_at", opts.caller, opts.contractId, opts.epoch);
}

/** View: is the current epoch >= unlock_epoch? */
export function isUnlockedPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_unlocked", opts.caller, opts.contractId, opts.epoch);
}

/** View: has the beneficiary already claimed? */
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
export const setTermsTx = (baseUrl: string, o: Parameters<typeof setTermsPayload>[0]) =>
  post(baseUrl, CALL_PATH, setTermsPayload(o));
export const claimTx = (baseUrl: string, o: Parameters<typeof claimPayload>[0]) =>
  post(baseUrl, CALL_PATH, claimPayload(o));
export const revokeTx = (baseUrl: string, o: Parameters<typeof revokePayload>[0]) =>
  post(baseUrl, CALL_PATH, revokePayload(o));

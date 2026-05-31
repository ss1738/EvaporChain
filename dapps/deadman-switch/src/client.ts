// DeadMan Switch — chain client. Three roles:
//
//   (a) Deployer / Owner — deploys the .es source + arm()s it with
//       (holder address, secret commitment hash, refresh window).
//       After arm() the deployer has no further authority; the
//       configuration is locked.
//
//   (b) Holder — must refresh() within `refresh_window` epochs.
//       May trigger_early(plaintext) to release at will. May
//       transfer_holder(new_holder) to hand off the watching duty.
//
//   (c) Anyone (after deadline) — once the deadline lapses,
//       anyone observing the chain may call release_dead(plaintext)
//       to fire the switch. The chain's own epoch is the trigger;
//       no keeper service needed.
//
// All view functions are safe to call from any address. The
// `is_alive` / `is_releasable` / `epochs_until_deadline` triple is
// the natural UI surface for a "switch status" panel.

import { DEADMAN_SWITCH_SOURCE } from "./contract.ts";

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
    source_code: DEADMAN_SWITCH_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Owner-only, one-shot: lock the configuration.
 *  `payloadHash` is the public commitment — typically the BLAKE3
 *  of the off-chain secret. `windowEpochs` is how many epochs the
 *  holder may go silent before anyone can release. */
export function armPayload(opts: {
  caller: number;
  contractId: number;
  holderHex: string;
  payloadHash: string;
  windowEpochs: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "arm",
    args: [opts.holderHex, opts.payloadHash, opts.windowEpochs],
    epoch: opts.epoch,
  };
}

/** Holder-only: push the deadline forward by `refresh_window` epochs.
 *  Bumps the refresh_count for analytics. */
export function refreshPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("refresh", opts.caller, opts.contractId, opts.epoch);
}

/** Holder-only: fire the switch immediately with optional plaintext
 *  reveal. Pass empty string for hash-only release. */
export function triggerEarlyPayload(opts: {
  caller: number;
  contractId: number;
  plaintext: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "trigger_early",
    args: [opts.plaintext],
    epoch: opts.epoch,
  };
}

/** Anyone (after deadline): fire the switch. The plaintext arg is
 *  optional; in most uses the releaser is a designated executor
 *  who has the plaintext, but the contract is happy with empty if
 *  the releaser just wants to publish the "dead" fact + commitment. */
export function releaseDeadPayload(opts: {
  caller: number;
  contractId: number;
  plaintext: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "release_dead",
    args: [opts.plaintext],
    epoch: opts.epoch,
  };
}

/** Holder-only: hand off the watching duty. Does NOT reset the
 *  deadline; the new holder inherits whatever epochs are left. */
export function transferHolderPayload(opts: {
  caller: number;
  contractId: number;
  newHolderHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "transfer_holder",
    args: [opts.newHolderHex],
    epoch: opts.epoch,
  };
}

/** View: is the switch in the green-light state? (armed, not
 *  released, deadline not yet passed). */
export function isAlivePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_alive", opts.caller, opts.contractId, opts.epoch);
}

/** View: is the switch ready for anyone to release? (armed, not
 *  released, deadline lapsed). */
export function isReleasablePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_releasable", opts.caller, opts.contractId, opts.epoch);
}

/** View: has the secret been released? */
export function isReleasedPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_released", opts.caller, opts.contractId, opts.epoch);
}

/** View: epochs left before the switch becomes releasable.
 *  Returns 0 once the deadline has passed. */
export function epochsUntilDeadlinePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("epochs_until_deadline", opts.caller, opts.contractId, opts.epoch);
}

/** View: the committed secret hash. Always readable, even
 *  pre-release — that's the whole point of a public commitment. */
export function secretHashViewPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("secret_hash_view", opts.caller, opts.contractId, opts.epoch);
}

/** View: the revealed plaintext (only readable post-release;
 *  reverts otherwise). */
export function revealedSecretViewPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("revealed_secret_view", opts.caller, opts.contractId, opts.epoch);
}

/** View: which epoch the switch fired in (only readable post-release). */
export function releasedAtViewPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("released_at_view", opts.caller, opts.contractId, opts.epoch);
}

/** View: total refresh count (for analytics / holder activity tracking). */
export function refreshCountPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("refresh_count_view", opts.caller, opts.contractId, opts.epoch);
}

/** View: the epoch of the most recent refresh. */
export function lastRefreshPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("last_refresh_view", opts.caller, opts.contractId, opts.epoch);
}

/** View: returns the holder address. Reverts if not yet armed. */
export function holderViewPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("holder_view", opts.caller, opts.contractId, opts.epoch);
}

/** View: is `who` the current holder? Safe to call pre-arm
 *  (returns false rather than reverting). */
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

/** View: armed (configuration locked) or not. */
export function isArmedPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
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
export const refreshTx = (baseUrl: string, o: Parameters<typeof refreshPayload>[0]) =>
  post(baseUrl, CALL_PATH, refreshPayload(o));
export const triggerEarlyTx = (baseUrl: string, o: Parameters<typeof triggerEarlyPayload>[0]) =>
  post(baseUrl, CALL_PATH, triggerEarlyPayload(o));
export const releaseDeadTx = (baseUrl: string, o: Parameters<typeof releaseDeadPayload>[0]) =>
  post(baseUrl, CALL_PATH, releaseDeadPayload(o));
export const transferHolderTx = (baseUrl: string, o: Parameters<typeof transferHolderPayload>[0]) =>
  post(baseUrl, CALL_PATH, transferHolderPayload(o));

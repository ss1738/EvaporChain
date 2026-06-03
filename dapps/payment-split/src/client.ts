// PaymentSplit — chain client. Pull-payment revenue splitter with
// basis-point shares (sum must equal exactly 10_000 — 100.00% — no
// dust, no over-allocation).
//
// Three lifecycle phases:
//
//   1. **Pre-seal setup** (deployer/owner). add_recipient(target,
//      bps) for each recipient; bps must accumulate to 10_000.
//      seal() locks the recipient set; subsequent add_recipient
//      reverts.
//
//   2. **Operational** (anyone). deposit(amount) adds to the pool.
//      Recipients claim() their cumulative share minus already-claimed
//      via the SPLIT-1 division-first formula (avoids u64 overflow
//      at total_deposited > u64::MAX/bps). Re-claim with no new
//      deposit reverts.
//
//   3. **Terminal** (chain runtime). on_evaporate stamps
//      unclaimed_at_evaporate + flips forfeit_signaled so the
//      off-chain coordinator returns the residue to the deployer.
//
// The runtime is the closer. No off-chain recovery sweep needed —
// same chain-as-keeper doctrine as the Marketplace escrow quadruplet.

import { PAYMENT_SPLIT_SOURCE } from "./contract.ts";

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
    source_code: PAYMENT_SPLIT_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Owner-only, pre-seal: register a recipient with a bps share.
 *  `bps` must be positive; cumulative `total_bps` is capped at 10_000.
 *  Duplicates revert. Cannot be called after seal(). */
export function addRecipientPayload(opts: {
  caller: number;
  contractId: number;
  recipientHex: string;
  bps: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "add_recipient",
    args: [opts.recipientHex, opts.bps],
    epoch: opts.epoch,
  };
}

/** Owner-only: seal the recipient set. Requires `total_bps == 10000`
 *  exactly — no under-allocation, no over-allocation, no dust. After
 *  seal(), add_recipient reverts and deposit() becomes callable. */
export function sealPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("seal", opts.caller, opts.contractId, opts.epoch);
}

/** Anyone: deposit `amount` into the pool. Increments
 *  `total_deposited`; recipients pull from it via claim(). */
export function depositPayload(opts: {
  caller: number;
  contractId: number;
  amount: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "deposit",
    args: [opts.amount],
    epoch: opts.epoch,
  };
}

/** Recipient-only: claim cumulative share minus already-claimed.
 *  Returns the delta the off-chain coordinator should send. Reverts
 *  with "nothing to claim" if the entitlement hasn't grown since the
 *  last claim (no rounding refunds). */
export function claimPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("claim", opts.caller, opts.contractId, opts.epoch);
}

// ── Views ────────────────────────────────────────────────────────

/** View: gross cumulative entitlement for `who` (regardless of
 *  what they've already pulled). Non-recipients yield 0. */
export function entitlementOfPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "entitlement_of",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: vested-but-not-yet-claimed for `who` (what `claim()` would
 *  return). Non-recipients yield 0. */
export function pendingOfPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "pending_of",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: `who`'s bps share. Non-recipients yield 0. */
export function shareOfPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "share_of",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: total deposited into the pool to date. */
export function totalPoolPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("total_pool", opts.caller, opts.contractId, opts.epoch);
}

/** View: number of registered recipients. */
export function recipientsPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("recipients", opts.caller, opts.contractId, opts.epoch);
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
export const addRecipientTx = (baseUrl: string, o: Parameters<typeof addRecipientPayload>[0]) =>
  post(baseUrl, CALL_PATH, addRecipientPayload(o));
export const sealTx = (baseUrl: string, o: Parameters<typeof sealPayload>[0]) =>
  post(baseUrl, CALL_PATH, sealPayload(o));
export const depositTx = (baseUrl: string, o: Parameters<typeof depositPayload>[0]) =>
  post(baseUrl, CALL_PATH, depositPayload(o));
export const claimTx = (baseUrl: string, o: Parameters<typeof claimPayload>[0]) =>
  post(baseUrl, CALL_PATH, claimPayload(o));

// Subscription — chain client. Roles:
//
//   (a) Subscriber (deployer/owner) — calls set_terms(provider,
//       amount, period) once, then pay() each period. The act of
//       paying is itself the keep-alive: pay() refreshes the
//       contract's energy via the runtime hook; missing payments
//       lets the contract evaporate naturally; `on_evaporate`
//       flips lapsed=true. No off-chain reaper needed.
//
//   (b) Provider — receives payments, may unilaterally cancel.
//
//   (c) Either party — may cancel; cancellation is one-shot, blocks
//       future pay() calls, and is distinct from "lapsed by
//       evaporation" (cancelled subscriptions do NOT relapse).
//
// View functions are safe for any caller. The is_active() triple
// (sealed && !cancelled && !lapsed) is the natural status surface.

import { SUBSCRIPTION_SOURCE } from "./contract.ts";

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
    source_code: SUBSCRIPTION_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Subscriber-only, one-shot: lock terms (provider, amount, period). */
export function setTermsPayload(opts: {
  caller: number;
  contractId: number;
  providerHex: string;
  amount: number;
  period: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "set_terms",
    args: [opts.providerHex, opts.amount, opts.period],
    epoch: opts.epoch,
  };
}

/** Subscriber-only: pay one period. Returns the amount paid (so the
 *  off-chain coordinator knows how much to transfer). The act of
 *  calling this method IS the keep-alive — the contract's energy
 *  refreshes via the on-chain runtime hook on every pay(). */
export function payPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("pay", opts.caller, opts.contractId, opts.epoch);
}

/** Either party (subscriber OR provider) may cancel. One-shot.
 *  Blocks subsequent pay() calls but does NOT count as a lapse. */
export function cancelPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("cancel", opts.caller, opts.contractId, opts.epoch);
}

/** View: who's the provider? */
export function providerOfPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("provider_of", opts.caller, opts.contractId, opts.epoch);
}

/** View: who's the subscriber? */
export function subscriberOfPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("subscriber_of", opts.caller, opts.contractId, opts.epoch);
}

/** View: payment per period. */
export function amountPerPeriodPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("amount_per_period", opts.caller, opts.contractId, opts.epoch);
}

/** View: period length in epochs. */
export function periodLengthPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("period_length", opts.caller, opts.contractId, opts.epoch);
}

/** View: how many periods have been paid (for analytics). */
export function periodsPaidPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("periods_paid", opts.caller, opts.contractId, opts.epoch);
}

/** View: cumulative total paid (periods_paid * amount_per_period). */
export function totalPaidPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("total_paid", opts.caller, opts.contractId, opts.epoch);
}

/** View: epoch of the most recent payment. */
export function lastPaymentPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("last_payment", opts.caller, opts.contractId, opts.epoch);
}

/** View: is the subscription armed AND not cancelled AND not lapsed?
 *  The natural "should I bill?" gate for off-chain coordinators. */
export function isActivePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_active", opts.caller, opts.contractId, opts.epoch);
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
export const payTx = (baseUrl: string, o: Parameters<typeof payPayload>[0]) =>
  post(baseUrl, CALL_PATH, payPayload(o));
export const cancelTx = (baseUrl: string, o: Parameters<typeof cancelPayload>[0]) =>
  post(baseUrl, CALL_PATH, cancelPayload(o));

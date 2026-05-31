// MnemoChain — chain client. Two roles:
//
//   (a) Deployer / owner — deploys + arm()s with (holder, content,
//       initial_stability). The owner is the platform that created
//       the card; the holder is the learner.
//   (b) Holder — review()s at any cadence; transfer()s to a new
//       holder (the portable cognitive credential — the card
//       carries its review history with it).
//
// Rating convention (matches Anki):
//   1 = Again  (forgot — halve stability)
//   2 = Hard   (remembered with effort — stability unchanged)
//   3 = Good   (remembered — double stability)
//   4 = Easy   (effortless — triple stability)

import { MNEMOCHAIN_SOURCE } from "./contract.ts";

export const DEPLOY_PATH = "/api/tx/deploy-script";
export const CALL_PATH = "/api/tx/call-script";

export const RATING_AGAIN = 1;
export const RATING_HARD = 2;
export const RATING_GOOD = 3;
export const RATING_EASY = 4;

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
    source_code: MNEMOCHAIN_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Owner-only, one-shot: arm with holder + content hash + initial stability. */
export function armPayload(opts: {
  caller: number;
  contractId: number;
  holderHex: string;
  contentHash: string;
  initialStability: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "arm",
    args: [opts.holderHex, opts.contentHash, opts.initialStability],
    epoch: opts.epoch,
  };
}

/** Holder: review with a rating (1=Again, 2=Hard, 3=Good, 4=Easy).
 *  Use the RATING_* constants for readability. */
export function reviewPayload(opts: {
  caller: number;
  contractId: number;
  rating: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "review",
    args: [opts.rating],
    epoch: opts.epoch,
  };
}

/** Holder: transfer the card to a new holder. */
export function transferPayload(opts: {
  caller: number;
  contractId: number;
  toHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "transfer",
    args: [opts.toHex],
    epoch: opts.epoch,
  };
}

/** View: current retrievability in basis points (0-10000). */
export function retrievabilityBpPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("retrievability_bp", opts.caller, opts.contractId, opts.epoch);
}

/** View: is the card due for review (retrievability < 90%)? */
export function isDuePayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_due", opts.caller, opts.contractId, opts.epoch);
}

/** View: epochs until the card crosses into "due" state. */
export function epochsUntilDuePayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("epochs_until_due", opts.caller, opts.contractId, opts.epoch);
}

/** View: current stability (FSRS-lite). */
export function stabilityViewPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("stability_view", opts.caller, opts.contractId, opts.epoch);
}

/** View: total reviews. */
export function reviewCountPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("review_count_view", opts.caller, opts.contractId, opts.epoch);
}

/** View: is `who` the current holder? */
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

/** View: card content hash. Reverts pre-arm. */
export function cardContentPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("card_content_view", opts.caller, opts.contractId, opts.epoch);
}

export function isArmedPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_armed", opts.caller, opts.contractId, opts.epoch);
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
export const armTx = (baseUrl: string, o: Parameters<typeof armPayload>[0]) =>
  post(baseUrl, CALL_PATH, armPayload(o));
export const reviewTx = (baseUrl: string, o: Parameters<typeof reviewPayload>[0]) =>
  post(baseUrl, CALL_PATH, reviewPayload(o));
export const transferTx = (baseUrl: string, o: Parameters<typeof transferPayload>[0]) =>
  post(baseUrl, CALL_PATH, transferPayload(o));

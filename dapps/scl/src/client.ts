// SCL — chain client. Three roles:
//
//   (a) Lessor (owner) — deploys + arms with (lessee, verb,
//       object_hex, duration_epochs). May call revoke() to
//       terminate early.
//   (b) Lessee — calls exercise() while is_active() is true.
//   (c) Anyone — queries is_active(), is_lessee(who),
//       epochs_remaining() as the gate before honouring the
//       associated off-chain action.
//
// The doctrine point: even without revoke(), the capability
// disappears when the contract evaporates — no global ACL to
// update, no caller to forget. Decay IS the revocation primitive.

import { SCL_SOURCE } from "./contract.ts";

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
    source_code: SCL_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Lessor-only, one-shot: arm with lessee + verb + object hex + duration. */
export function armPayload(opts: {
  caller: number;
  contractId: number;
  lesseeHex: string;
  verb: string;
  objectHex: string;
  durationEpochs: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "arm",
    args: [opts.lesseeHex, opts.verb, opts.objectHex, opts.durationEpochs],
    epoch: opts.epoch,
  };
}

/** Lessee-only: exercise the capability. Reverts post-revoke or
 *  past the soft expiry. */
export function exercisePayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("exercise", opts.caller, opts.contractId, opts.epoch);
}

/** Lessor-only: revoke. Terminal — second revoke reverts. */
export function revokePayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("revoke", opts.caller, opts.contractId, opts.epoch);
}

/** View: composite gate. True iff armed AND not revoked AND
 *  epoch < granted_at + duration. The function downstream
 *  contracts and dApps consult before honouring the action. */
export function isActivePayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_active", opts.caller, opts.contractId, opts.epoch);
}

/** View: epochs left before soft expiry; 0 if revoked or expired. */
export function epochsRemainingPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("epochs_remaining", opts.caller, opts.contractId, opts.epoch);
}

/** View: is `who` the lessee? */
export function isLesseePayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "is_lessee",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: the leased verb (string). Reverts pre-arm. */
export function verbViewPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("verb_view", opts.caller, opts.contractId, opts.epoch);
}

/** View: the leased object hex (string). Reverts pre-arm. */
export function objectViewPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("object_view", opts.caller, opts.contractId, opts.epoch);
}

export function exercisesTotalPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("exercises_total", opts.caller, opts.contractId, opts.epoch);
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
export const exerciseTx = (baseUrl: string, o: Parameters<typeof exercisePayload>[0]) =>
  post(baseUrl, CALL_PATH, exercisePayload(o));
export const revokeTx = (baseUrl: string, o: Parameters<typeof revokePayload>[0]) =>
  post(baseUrl, CALL_PATH, revokePayload(o));

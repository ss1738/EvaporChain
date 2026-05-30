// Refresh-Market — chain client.
//
// Three player types:
//
//   (a) **Operator** — deploys + arms with capacity / base_rent /
//       eviction_window. After arming the curve is immutable.
//
//   (b) **Holders** — claim_slot (while capacity remains), refresh_slot
//       (reset their eviction clock once per rent period), release_slot
//       (voluntary). Only one slot per caller per contract.
//
//   (c) **Evictors** — anyone can call evict(who) once `who`'s
//       eviction window has elapsed. Reclaims capacity for the
//       market (and is the substrate's incentive for third parties
//       to police staleness).
//
// Pricing previews live in `./rate.ts` — pure BigInt port of the
// on-chain `current_rate` formula, so UIs render the same number the
// chain would compute without a round-trip.

import { REFRESH_MARKET_SOURCE } from "./contract.ts";

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

/** Deploy. `energy` = namespace lifespan, `half_life` = decay rate. */
export function deployPayload(opts: { deployer: number; energy: number; halfLife: number }): DeployPayload {
  return {
    deployer: opts.deployer,
    source_code: REFRESH_MARKET_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Operator-only, one-shot: configure capacity + base rent + eviction
 *  window. After arming these are immutable. */
export function armPayload(opts: {
  caller: number;
  contractId: number;
  capacity: number;
  baseRent: number;
  evictionWindow: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "arm",
    args: [opts.capacity, opts.baseRent, opts.evictionWindow],
    epoch: opts.epoch,
  };
}

/** Holder: claim a slot. Reverts if armed-namespace is at capacity
 *  OR if the caller already holds a slot. */
export function claimSlotPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("claim_slot", opts.caller, opts.contractId, opts.epoch);
}

/** Holder: reset eviction clock. */
export function refreshSlotPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("refresh_slot", opts.caller, opts.contractId, opts.epoch);
}

/** Holder: voluntary release. */
export function releaseSlotPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("release_slot", opts.caller, opts.contractId, opts.epoch);
}

/** Anyone: evict a stale holder. Reverts before window elapses. */
export function evictPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "evict",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: current per-epoch rate. */
export function currentRatePayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("current_rate", opts.caller, opts.contractId, opts.epoch);
}

/** View: what would the rate be at a hypothetical `used` level? */
export function rateAtUsedPayload(opts: {
  caller: number;
  contractId: number;
  usedHypothetical: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "rate_at_used",
    args: [opts.usedHypothetical],
    epoch: opts.epoch,
  };
}

/** View: does `who` hold a slot? */
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

/** View: is `who` past their eviction window (so evict() would succeed)? */
export function isEvictablePayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "is_evictable",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: slots_remaining. */
export function slotsRemainingPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("slots_remaining", opts.caller, opts.contractId, opts.epoch);
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
export const claimSlotTx = (baseUrl: string, o: Parameters<typeof claimSlotPayload>[0]) =>
  post(baseUrl, CALL_PATH, claimSlotPayload(o));
export const refreshSlotTx = (baseUrl: string, o: Parameters<typeof refreshSlotPayload>[0]) =>
  post(baseUrl, CALL_PATH, refreshSlotPayload(o));
export const releaseSlotTx = (baseUrl: string, o: Parameters<typeof releaseSlotPayload>[0]) =>
  post(baseUrl, CALL_PATH, releaseSlotPayload(o));
export const evictTx = (baseUrl: string, o: Parameters<typeof evictPayload>[0]) =>
  post(baseUrl, CALL_PATH, evictPayload(o));

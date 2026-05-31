// MortalMessage — chain client. Two roles:
//
//   (a) Sender (deployer/owner) — deploys the contract with
//       (energy, half_life) picked from the dApp's preset list, then
//       calls `set_payload(body, recipient)` exactly once. After
//       sealing, the body + recipient are immutable for the
//       lifetime of the contract.
//
//   (b) Recipient — calls `read()` to retrieve the body. The contract
//       gates on caller ∈ {sender, recipient}; anyone else gets a
//       require() revert.
//
// The contract's OWN energy is the message lifespan. No application
// code drives the decay — it's the chain runtime that walks the
// contract through active → grace → ghost → tomb. `on_refresh` boosts
// the boost_count counter; `on_evaporate` emits the terminal event.
//
// This is the canonical EvaporScript pilot (per project CLAUDE.md
// §"Two unifying invariants" #2). Every other EvaporScript contract
// in the tree follows this shape.

import { MORTAL_MESSAGE_SOURCE } from "./contract.ts";

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
    source_code: MORTAL_MESSAGE_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Sender-only, one-shot: populate the message body + recipient.
 *  After this call the message is sealed; no further mutation. */
export function setPayloadPayload(opts: {
  caller: number;
  contractId: number;
  body: string;
  recipientHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "set_payload",
    args: [opts.body, opts.recipientHex],
    epoch: opts.epoch,
  };
}

/** Sender or recipient: retrieve the body. Other callers revert. */
export function readPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("read", opts.caller, opts.contractId, opts.epoch);
}

/** Manual boost-record. The chain's runtime hook also bumps
 *  boost_count on refresh; this is the explicit user-callable
 *  surface for off-chain coordinators that want to log a boost
 *  without going through the energy-refresh path. */
export function recordBoostPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("record_boost", opts.caller, opts.contractId, opts.epoch);
}

/** View: how many times the message has been boosted. Safe for
 *  any caller — no privacy surface (the body is the protected part). */
export function inspectPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("inspect", opts.caller, opts.contractId, opts.epoch);
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
export const setPayloadTx = (baseUrl: string, o: Parameters<typeof setPayloadPayload>[0]) =>
  post(baseUrl, CALL_PATH, setPayloadPayload(o));
export const readTx = (baseUrl: string, o: Parameters<typeof readPayload>[0]) =>
  post(baseUrl, CALL_PATH, readPayload(o));
export const recordBoostTx = (baseUrl: string, o: Parameters<typeof recordBoostPayload>[0]) =>
  post(baseUrl, CALL_PATH, recordBoostPayload(o));

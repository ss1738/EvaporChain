// MortalNft — chain client. Two roles:
//
//   (a) Minter (deployer/owner) — deploys with (energy, half_life)
//       and calls set_metadata(name, collection, metadata, recipient)
//       exactly once to mint. `recipient` is the INITIAL holder —
//       passing the buyer's address here implements the mint-to-buyer
//       flow in one shot (no separate mint + transfer round-trip).
//
//   (b) Holder — calls transfer(to) to hand off. Each transfer bumps
//       transfer_count + records the epoch for chain-of-custody
//       telemetry. Only the current holder can transfer (not the
//       minter; the minter loses authority after sealing).
//
// Two address concepts to NOT confuse:
//   - The EvaporScript builtin `owner` = the minter (immutable).
//   - The on-chain `self.holder` state = the current holder
//     (mutable via transfer). They diverge the moment a non-self
//     mint or any transfer happens.
//
// The contract's OWN energy is the NFT's lifespan. When energy hits
// zero the contract evaporates and the NFT becomes a chain-level
// Ghost; standard ghost-recovery flow can re-energize it.

import { MORTAL_NFT_SOURCE } from "./contract.ts";

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
    source_code: MORTAL_NFT_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Minter-only, one-shot: seal the NFT identity and assign the
 *  initial holder. After this call, the metadata is immutable and
 *  the minter has no further authority. */
export function setMetadataPayload(opts: {
  caller: number;
  contractId: number;
  name: string;
  collection: string;
  metadata: string;
  recipientHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "set_metadata",
    args: [opts.name, opts.collection, opts.metadata, opts.recipientHex],
    epoch: opts.epoch,
  };
}

/** Holder-only: transfer to `to`. Increments transfer_count and
 *  records last_transfer_epoch for the chain-of-custody log. */
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

/** View: who currently holds the NFT? */
export function currentOwnerPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("current_owner", opts.caller, opts.contractId, opts.epoch);
}

/** View: the metadata reference (IPFS hash, HTTP URL, or
 *  content-addressed blob hash; the dApp interprets the format). */
export function metadataUriPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("metadata_uri", opts.caller, opts.contractId, opts.epoch);
}

/** View: how many times the NFT has changed hands. */
export function transfersPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("transfers", opts.caller, opts.contractId, opts.epoch);
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
export const setMetadataTx = (baseUrl: string, o: Parameters<typeof setMetadataPayload>[0]) =>
  post(baseUrl, CALL_PATH, setMetadataPayload(o));
export const transferTx = (baseUrl: string, o: Parameters<typeof transferPayload>[0]) =>
  post(baseUrl, CALL_PATH, transferPayload(o));

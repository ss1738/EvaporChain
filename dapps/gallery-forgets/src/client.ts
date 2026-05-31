// Gallery-That-Forgets — chain client.
//
// Two roles:
//
//   (a) Curator (owner) — open() once with a name, then add_piece()
//       / remove_piece() while the contract is alive. close_early()
//       to end the exhibition before evaporation.
//
//   (b) Visitors — anyone — query is_open(), active_pieces(),
//       is_piece_active(id), piece_hash_view(id),
//       gallery_name_view() to render the exhibition off-chain.
//
// Piece IDs are monotonically allocated (1, 2, 3, …) and never
// recycle — removing piece 2 leaves slot 2 permanently inactive;
// the next add() goes to slot N+1.

import { GALLERY_FORGETS_SOURCE } from "./contract.ts";

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
    source_code: GALLERY_FORGETS_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Curator-only, one-shot: open the gallery with a name. */
export function openPayload(opts: {
  caller: number;
  contractId: number;
  name: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "open",
    args: [opts.name],
    epoch: opts.epoch,
  };
}

/** Curator-only: add a piece. The id assigned == next_piece_id at
 *  call-time (read it via nextIdPayload before the tx if you need
 *  to surface the id in your UI). */
export function addPiecePayload(opts: {
  caller: number;
  contractId: number;
  contentHash: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "add_piece",
    args: [opts.contentHash],
    epoch: opts.epoch,
  };
}

/** Curator-only: remove a piece by id. Slot stays reserved (no recycle). */
export function removePiecePayload(opts: {
  caller: number;
  contractId: number;
  pieceId: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "remove_piece",
    args: [opts.pieceId],
    epoch: opts.epoch,
  };
}

/** Curator-only: end the exhibition before natural evaporation. */
export function closeEarlyPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("close_early", opts.caller, opts.contractId, opts.epoch);
}

/** View: is the gallery accepting new pieces? */
export function isOpenPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_open", opts.caller, opts.contractId, opts.epoch);
}

/** View: is a specific piece currently exhibited? */
export function isPieceActivePayload(opts: {
  caller: number;
  contractId: number;
  pieceId: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "is_piece_active",
    args: [opts.pieceId],
    epoch: opts.epoch,
  };
}

/** View: a piece's content hash. Reverts if piece is inactive. */
export function pieceHashViewPayload(opts: {
  caller: number;
  contractId: number;
  pieceId: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "piece_hash_view",
    args: [opts.pieceId],
    epoch: opts.epoch,
  };
}

/** View: how many pieces are currently exhibited. */
export function activePiecesPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("active_pieces", opts.caller, opts.contractId, opts.epoch);
}

/** View: gallery name (reverts pre-open). */
export function galleryNamePayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("gallery_name_view", opts.caller, opts.contractId, opts.epoch);
}

/** View: epochs since open. */
export function ageSinceOpenPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("age_since_open", opts.caller, opts.contractId, opts.epoch);
}

/** View: next_piece_id — the id that the next add_piece() will use. */
export function nextIdPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("next_id", opts.caller, opts.contractId, opts.epoch);
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
export const openTx = (baseUrl: string, o: Parameters<typeof openPayload>[0]) =>
  post(baseUrl, CALL_PATH, openPayload(o));
export const addPieceTx = (baseUrl: string, o: Parameters<typeof addPiecePayload>[0]) =>
  post(baseUrl, CALL_PATH, addPiecePayload(o));
export const removePieceTx = (baseUrl: string, o: Parameters<typeof removePiecePayload>[0]) =>
  post(baseUrl, CALL_PATH, removePiecePayload(o));
export const closeEarlyTx = (baseUrl: string, o: Parameters<typeof closeEarlyPayload>[0]) =>
  post(baseUrl, CALL_PATH, closeEarlyPayload(o));

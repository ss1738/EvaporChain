// SealedBidAuction — chain client. Classic commit/reveal/settle with
// a doctrine twist: `effective` (decay-adjusted) bid strength is the
// comparator, not nominal. The off-chain coordinator decays nominal →
// effective by reveal-time distance from commit, so effective <=
// nominal always. Early reveal wins on equal effective strength.
//
// Phase machine (seller-driven, monotone forward only):
//   0 = COMMIT   — bidders submit hashes
//   1 = REVEAL   — bidders open commits with nominal + effective
//   2 = SETTLE   — seller picks winner
//   3 = CLOSED   — record_winner advances to this on success
//
// SBA-1 commit-reveal binding (audit 2026-05-17): commit_hash is
// `blake3(chain_id || auction_id || bidder || price || nonce)`
// produced by the Rust NX4 / decay-bound-auction substrate. The
// contract stores the hash at commit time + re-verifies at reveal;
// the substrate additionally verifies the pre-image. Both layers
// enforce binding so neither can be bypassed alone.
//
// Doctrine: the contract's energy budget is the entire auction
// window. on_evaporate without settlement = auction void; the
// coordinator refunds bidders off-chain. Same chain-as-keeper
// pattern as the Marketplace escrow family.

import { SEALED_BID_AUCTION_SOURCE } from "./contract.ts";

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
    source_code: SEALED_BID_AUCTION_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Seller-only, one-shot: set the auction metadata. After this call
 *  the auction is `sealed` and the phase machine starts in COMMIT (0). */
export function setMetadataPayload(opts: {
  caller: number;
  contractId: number;
  itemLabel: string;
  reserve: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "set_metadata",
    args: [opts.itemLabel, opts.reserve],
    epoch: opts.epoch,
  };
}

/** Seller-only: advance the phase machine. Strict monotone forward
 *  (rewinding reverts). Max phase 3. Use 1 to open REVEAL, 2 to open
 *  SETTLE, 3 only via record_winner (which auto-advances). */
export function setPhasePayload(opts: {
  caller: number;
  contractId: number;
  next: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "set_phase",
    args: [opts.next],
    epoch: opts.epoch,
  };
}

/** Open call (phase 0 = COMMIT): bidder submits a commit hash.
 *  One commit per address. The hash is the off-chain blake3 of
 *  `chain_id || auction_id || bidder || price || nonce` produced by
 *  the Rust NX4 substrate. Stored on-chain for SBA-1 binding. */
export function commitPayload(opts: {
  caller: number;
  contractId: number;
  commitHash: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "commit",
    args: [opts.commitHash],
    epoch: opts.epoch,
  };
}

/** Open call (phase 1 = REVEAL): bidder reveals nominal + effective
 *  + the original commit hash. The contract verifies the stored
 *  commit_hash matches; the substrate verifies the blake3 pre-image.
 *  Nominal must clear `reserve_price`; effective must be <= nominal
 *  (the coordinator decays nominal → effective by reveal lag). */
export function revealPayload(opts: {
  caller: number;
  contractId: number;
  nominalBid: number;
  effectiveBid: number;
  commitmentHash: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "reveal",
    args: [opts.nominalBid, opts.effectiveBid, opts.commitmentHash],
    epoch: opts.epoch,
  };
}

/** Seller-only (phase 2 = SETTLE): record the winning bidder. The
 *  contract verifies the winner actually revealed and that the
 *  seller's claimed effective matches the on-chain reveal. Advances
 *  the phase to 3 (CLOSED) atomically. One-shot. */
export function recordWinnerPayload(opts: {
  caller: number;
  contractId: number;
  winnerHex: string;
  winningEffectiveBid: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "record_winner",
    args: [opts.winnerHex, opts.winningEffectiveBid],
    epoch: opts.epoch,
  };
}

// ── Views ────────────────────────────────────────────────────────

/** View: current phase (0/1/2/3). */
export function currentPhasePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("current_phase", opts.caller, opts.contractId, opts.epoch);
}

/** View: nominal bid recorded for `who` at reveal. Returns 0 if
 *  the address hasn't revealed (EvaporScript map default). */
export function nominalBidOfPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "nominal_bid_of",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: effective (decay-adjusted) bid for `who`. Returns 0 if
 *  not revealed. */
export function effectiveBidOfPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "effective_bid_of",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: stored commit hash for `who` (for coordinator verification). */
export function committedHashOfPayload(opts: {
  caller: number;
  contractId: number;
  whoHex: string;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "committed_hash_of",
    args: [opts.whoHex],
    epoch: opts.epoch,
  };
}

/** View: settled winner address (zero address if not yet settled). */
export function winnerOfPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("winner_of", opts.caller, opts.contractId, opts.epoch);
}

/** View: has the seller settled? */
export function isSettledPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("is_settled", opts.caller, opts.contractId, opts.epoch);
}

/** View: commits received in phase 0. */
export function commitsReceivedPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("commits_received", opts.caller, opts.contractId, opts.epoch);
}

/** View: reveals received in phase 1. */
export function revealsReceivedPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("reveals_received", opts.caller, opts.contractId, opts.epoch);
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
export const setPhaseTx = (baseUrl: string, o: Parameters<typeof setPhasePayload>[0]) =>
  post(baseUrl, CALL_PATH, setPhasePayload(o));
export const commitTx = (baseUrl: string, o: Parameters<typeof commitPayload>[0]) =>
  post(baseUrl, CALL_PATH, commitPayload(o));
export const revealTx = (baseUrl: string, o: Parameters<typeof revealPayload>[0]) =>
  post(baseUrl, CALL_PATH, revealPayload(o));
export const recordWinnerTx = (baseUrl: string, o: Parameters<typeof recordWinnerPayload>[0]) =>
  post(baseUrl, CALL_PATH, recordWinnerPayload(o));

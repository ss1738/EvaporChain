// Bell-Oracle — chain client.
//
// Two roles:
//
//   (a) **Operator / relayer** — armed at deploy, then runs a loop
//       reading /api/bell/latest from a chain node and submitting each
//       new measurement via submit_reading(). The contract enforces
//       above-floor (S > 2.0 ≡ s_milli > 2000) AND strictly-increasing
//       height; everything else bumps a rejection counter.
//
//   (b) **Consumer dApp** — calls is_certified_now() to gate any
//       action that requires quantum-grade entropy. False until the
//       relayer has posted a fresh certifying reading.
//
// The `fetchLatestBellBeacon()` helper does the read-side (a) — wraps
// the /api/bell/latest endpoint and decides whether the value is
// submission-worthy. The relayer then signs + posts via submitReadingTx.

import { BELL_ORACLE_SOURCE } from "./contract.ts";

export const DEPLOY_PATH = "/api/tx/deploy-script";
export const CALL_PATH = "/api/tx/call-script";
/** The node endpoint that exposes the per-block CHSH S-value beacon. */
export const BELL_LATEST_PATH = "/api/bell/latest";
/** Local-realism floor: readings at or below this are NOT quantum-grade. */
export const LOCAL_REALISM_FLOOR_MILLI = 2000;

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

/** Deploy. `energy` = oracle's own lifespan, `half_life` = decay rate. */
export function deployPayload(opts: { deployer: number; energy: number; halfLife: number }): DeployPayload {
  return {
    deployer: opts.deployer,
    source_code: BELL_ORACLE_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Arm. Operator-only, one-shot. `maxAgeEpochs` is the freshness ceiling. */
export function armPayload(opts: { caller: number; contractId: number; maxAgeEpochs: number; epoch: number }): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "arm",
    args: [opts.maxAgeEpochs],
    epoch: opts.epoch,
  };
}

/** Operator-only: submit a per-block Bell reading.
 *  Contract rejects (counter-bump, no revert) if s_milli ≤ 2000 OR
 *  height ≤ latest_height. */
export function submitReadingPayload(opts: {
  caller: number;
  contractId: number;
  sMilli: number;
  height: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "submit_reading",
    args: [opts.sMilli, opts.height],
    epoch: opts.epoch,
  };
}

/** View: is the oracle currently holding a fresh above-floor reading? */
export function isCertifiedNowPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("is_certified_now", opts.caller, opts.contractId, opts.epoch);
}

/** View: latest accepted S in milli-units. Reverts if no reading yet. */
export function latestSMilliPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("latest_s_milli_view", opts.caller, opts.contractId, opts.epoch);
}

/** View: block height of the latest accepted measurement. */
export function lastHeightPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("last_height", opts.caller, opts.contractId, opts.epoch);
}

/** View: accepted-readings counter. */
export function acceptedTotalPayload(opts: { caller: number; contractId: number; epoch: number }): CallPayload {
  return noArgCall("accepted_total", opts.caller, opts.contractId, opts.epoch);
}

function noArgCall(method: string, caller: number, contractId: number, epoch: number): CallPayload {
  return { caller, contract_id: contractId, method, args: [], epoch };
}

/* ──────────────────────────────────────────────────────────────────────
 * Relayer-side: read the chain's /api/bell/latest endpoint.
 * ──────────────────────────────────────────────────────────────────── */

export interface BellLatestOk {
  status: "ok";
  s_value_milli: number;
  threshold_milli: number;
  bell_certified: boolean;
  /** Block height the reading was measured on. The endpoint name varies
   *  across nodes ('block_height' or 'height'); both shapes handled. */
  block_height?: number;
  height?: number;
}

export interface BellLatestEmpty {
  status: "no_data";
}

export type BellLatestResp = BellLatestOk | BellLatestEmpty;

/** Fetch /api/bell/latest. Returns null on network/parse error.
 *  Relayer typically polls this every block (~2s) and submits whenever
 *  it observes (status==ok && bell_certified && height > last_posted_height). */
export async function fetchLatestBellBeacon(baseUrl: string): Promise<BellLatestResp | null> {
  try {
    const res = await fetch(`${baseUrl}${BELL_LATEST_PATH}`);
    if (!res.ok) return null;
    return (await res.json()) as BellLatestResp;
  } catch {
    return null;
  }
}

/** Decide if a beacon reading is worth submitting on-chain.
 *  (Pre-filter: the contract would reject anyway, but skipping the
 *  POST saves gas.) */
export function isSubmissionWorthy(
  resp: BellLatestResp | null,
  lastPostedHeight: number,
): resp is BellLatestOk {
  if (resp == null) return false;
  if (resp.status !== "ok") return false;
  if (!resp.bell_certified) return false;
  if (resp.s_value_milli <= LOCAL_REALISM_FLOOR_MILLI) return false;
  const h = resp.block_height ?? resp.height;
  if (h == null) return false;
  return h > lastPostedHeight;
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
export const submitReadingTx = (baseUrl: string, o: Parameters<typeof submitReadingPayload>[0]) =>
  post(baseUrl, CALL_PATH, submitReadingPayload(o));

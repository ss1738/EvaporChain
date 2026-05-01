/**
 * Read-only client for EvaporChain validator analytics.
 *
 * Endpoints consumed (api.rs line numbers, HEAD ce4be0c):
 *   GET /api/validators                       — line 7142, registry + totals
 *   GET /api/validator/:id/delegations        — line 8197, delegator list
 *   GET /api/network/peers                    — line 6973, libp2p peer view
 *   GET /api/network/health                   — line 7213, oncall snapshot
 *   GET /api/finality/gap                     — line 13406, unfinalised + recent gaps
 *   GET /metrics                              — line 11693, per-validator histograms
 *
 * Notes on coverage gaps (documented; nothing is fabricated):
 *   - There is NO standalone slash-history endpoint. The chain exposes
 *     `total_slashed` per validator on /api/validators and a slash *action*
 *     POST /api/validators/sanov_slash, but no event timeline. SlashTimeline
 *     renders an empty placeholder with the cumulative figure when it has
 *     a non-zero total_slashed.
 *   - Bell-Beacon S-value is computed via POST /api/bell_beacon for an ad-hoc
 *     query, not a per-validator gauge. We surface "—" on the detail page.
 *   - Hot/cold stake split lives behind POST endpoints
 *     (/api/hot_cold_stake/*) that need explicit inputs; we show the
 *     effective_stake total only.
 *   - Subnet info comes from the peer view (PeerInfo.subnet) and is keyed
 *     by peer_id, not validator_id, so the per-validator "subnet" cell is a
 *     best-effort lookup that may be blank in dev mode.
 */

import {
  parsePrometheus,
  blockProductionByValidator,
  peerScores,
  rejectionCounters,
  gauge,
  type ValidatorBlockHist,
} from "./promParse";

// The wallet-sdk's `Validator` type is a coarse shape that doesn't match the
// node's actual /api/validators response (id, effective_stake, blocks_produced,
// total_slashed, etc.), so we hit the REST surface directly with plain fetch
// and define types here. The SDK is still listed as a permitted dep for
// future write-side flows.

const baseUrl = (): string =>
  typeof window !== "undefined" ? window.location.origin : "http://127.0.0.1:8080";

async function getJSON<T>(path: string): Promise<T> {
  const res = await fetch(`${baseUrl()}${path}`);
  if (!res.ok) throw new Error(`${path} → ${res.status}`);
  return (await res.json()) as T;
}
async function getText(path: string): Promise<string> {
  const res = await fetch(`${baseUrl()}${path}`);
  if (!res.ok) throw new Error(`${path} → ${res.status}`);
  return res.text();
}

// ── Types mirrored from api.rs (snake_case as the chain emits) ──

export interface ValidatorRow {
  id: number;
  address: string;
  stake: number;
  effective_stake: number;
  jailed: boolean;
  bls_registered: boolean;
  health_score: number;
  blocks_produced: number;
  total_slashed: number;
}
export interface ValidatorsResponse {
  count: number;
  total_stake: number;
  total_effective_stake: number;
  jailed_count: number;
  bls_registered_count: number;
  validators: ValidatorRow[];
}

export interface DelegationRow {
  delegator: string;
  validator_id: number;
  amount: number;
  delegated_at_epoch: number;
  unbonding_amount: number;
  unbonding_epoch: number | null;
}
export interface ValidatorDelegationsResponse {
  validator_id: number;
  delegation_count: number;
  total_delegated: number;
  delegations: DelegationRow[];
}

export interface PeerInfo {
  peer_id: string;
  ip: string | null;
  subnet: string | null;
  since_ms: number;
  score: number;
  age_seconds: number;
}
export interface PeersResponse {
  count: number;
  peers: PeerInfo[];
}

export interface NetworkHealth {
  block_height: number;
  epoch: number;
  state_root: string;
  last_block_age_secs: number | null;
  peer_count: number;
  mempool_size: number;
  validator_count: number;
  jailed_count: number;
  finalised_height: number;
  finality_lag_blocks: number;
  total_objects: number;
  ghost_count: number;
  uptime_seconds: number;
  status: "healthy" | "syncing" | "stalled" | "isolated";
}

export interface FinalityGap {
  unfinalised: Array<{ height: number; age_seconds: number }>;
  worst_gap_seconds: number;
  recent_gaps: Array<{ height: number; gap_seconds: number }>;
}

// ── Public dashboard API ──

export const analyticsApi = {
  validators: () => getJSON<ValidatorsResponse>("/api/validators"),
  validatorDelegations: (id: number) =>
    getJSON<ValidatorDelegationsResponse>(`/api/validator/${id}/delegations`),
  networkPeers: () => getJSON<PeersResponse>("/api/network/peers"),
  networkHealth: () => getJSON<NetworkHealth>("/api/network/health"),
  finalityGap: () => getJSON<FinalityGap>("/api/finality/gap"),
  /** Raw Prometheus exposition. /metrics requires admin auth in the node when
   *  EVAPORCHAIN_ADMIN_KEY is set; in dev/testnet it's open. We swallow
   *  failures so the dashboard degrades gracefully. */
  async metrics(): Promise<{
    perValidator: ValidatorBlockHist[];
    peers: Array<{ peerId: string; score: number }>;
    rejections: Record<string, number>;
    blockHeight: number;
    finalisedHeight: number;
    activeValidators: number;
    setSize: number;
    activeBans: number;
    unfinalisedHeights: number;
    worstUnfinalisedSecs: number;
  }> {
    let text = "";
    try {
      text = await getText("/metrics");
    } catch {
      // Admin-gated or unavailable — return empty shells.
    }
    const samples = parsePrometheus(text);
    return {
      perValidator: blockProductionByValidator(samples),
      peers: peerScores(samples),
      rejections: rejectionCounters(samples),
      blockHeight: gauge(samples, "evap_block_height"),
      finalisedHeight: gauge(samples, "evap_finalized_height"),
      activeValidators: gauge(samples, "evap_active_validators"),
      setSize: gauge(samples, "evap_validator_set_size"),
      activeBans: gauge(samples, "evap_active_bans"),
      unfinalisedHeights: gauge(samples, "evap_unfinalised_height_count"),
      worstUnfinalisedSecs: gauge(samples, "evap_worst_unfinalised_gap_seconds"),
    };
  },
};

/** Polling hook helper — fires `fn` immediately, then every `intervalMs`. */
export function pollWithInterval<T>(
  fn: () => Promise<T>,
  onData: (v: T) => void,
  onError: (e: Error) => void,
  intervalMs = 15_000,
): () => void {
  let cancelled = false;
  const run = async () => {
    try {
      const v = await fn();
      if (!cancelled) onData(v);
    } catch (e) {
      if (!cancelled) onError(e instanceof Error ? e : new Error(String(e)));
    }
  };
  run();
  const id = setInterval(run, intervalMs);
  return () => {
    cancelled = true;
    clearInterval(id);
  };
}

/** Fraction blocks_produced contributes vs the validator-set leader.
 *  Crude proxy for "uptime" — chain doesn't expose missed-slot counts. */
export function uptimeProxy(rows: ValidatorRow[]): Map<number, number> {
  const max = rows.reduce((m, v) => Math.max(m, v.blocks_produced), 0);
  const out = new Map<number, number>();
  for (const v of rows) out.set(v.id, max > 0 ? v.blocks_produced / max : 0);
  return out;
}

export function shortAddr(a: string): string {
  if (a.length <= 14) return a;
  return `${a.slice(0, 8)}…${a.slice(-6)}`;
}

export function fmtEvap(n: number): string {
  return n.toLocaleString();
}

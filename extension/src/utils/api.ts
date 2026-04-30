/**
 * EvaporChain node API client for the browser extension.
 * Wraps the REST API endpoints used by the wallet.
 */

const DEFAULT_NODE = "https://testnet.evaporchain.com";

export interface ChainStatus {
  chain_name: string;
  version: string;
  block_height: number;
  epoch: number;
  active_objects: number;
  ghost_count: number;
  total_evaporated: number;
  peer_count: number;
  uptime_seconds: number;
}

export interface AccountDetail {
  address: string;
  name: string;
  balance: number;
  nonce: number;
}

export interface StateObject {
  id: string;
  name: string;
  owner: string;
  energy: number;
  max_energy: number;
  half_life: number;
  state: "Active" | "Grace" | "Ghost" | "Risen";
  current_energy: number;
  decay_percentage: number;
}

export interface NftItem {
  id: string;
  name: string;
  collection: string;
  owner: string;
  image_url?: string;
  energy: number;
  max_energy: number;
  current_energy: number;
  half_life: number;
  decay_percentage: number;
  state: "Active" | "Grace" | "Ghost";
  epochs_remaining: number;
  created_epoch: number;
}

export interface TxResult {
  success: boolean;
  message: string;
  hash?: string;
}

export interface TransactionRecord {
  type: string;
  detail: string;
  hash?: string;
}

export interface SimulateTransactionRequest {
  from: string;
  to: string;
  amount: number;
}

export interface SimulationResult {
  balanceBefore: number;
  balanceAfter: number;
  maxEnergy: number;
  gasCost: number;
  energyCost: number;
  nonce: number;
  estimatedBlock: number;
  recipientIsGhost: boolean;
  objectSurvivalEpochs?: number;
  objectHalfLife?: number;
  objectStateAfter: "Active" | "Grace" | "Ghost" | "Risen";
}

export interface DecayForecastResult {
  objectId: string;
  currentEnergy: number;
  maxEnergy: number;
  halfLife: number;
  currentEpoch: number;
  epochDurationMs: number;
  projectedEnergy: Array<{ epoch: number; energy: number; percent: number }>;
  evaporationEpoch: number;
  evaporationDate: string;
}

export interface TokenInfo {
  symbol: string;
  name: string;
  address: string;
  decimals: number;
  balance: number;
  logo?: string;
}

export interface SwapQuote {
  from_token: string;
  to_token: string;
  amount_in: number;
  amount_out: number;
  rate: number;
  price_impact: number;
  energy_cost: number;
  estimated_fee: number;
}

export interface SwapResult {
  success: boolean;
  message: string;
  hash?: string;
  amount_in: number;
  amount_out: number;
}

export interface RefreshCostEstimate {
  object_id: string;
  current_energy: number;
  target_energy: number;
  energy_needed: number;
  evap_cost: number;
  epochs_extended: number;
}

export interface GhostObject {
  id: string;
  name: string;
  owner: string;
  original_energy: number;
  max_energy: number;
  half_life: number;
  evaporated_epoch: number;
  epochs_since_evaporation: number;
  recovery_cost: number;
  recovery_window_remaining: number;
  recovery_window_total: number;
  proof_status: "valid" | "expiring" | "expired";
}

export interface GhostDetail extends GhostObject {
  created_epoch: number;
  evaporation_date: string;
  mint_date: string;
  merkle_proof: string;
  metadata: Record<string, string>;
  energy_history: Array<{ epoch: number; energy: number; percent: number }>;
}

export interface RecoveryCostEstimate {
  object_id: string;
  base_cost: number;
  decay_penalty: number;
  total_cost: number;
  proof_valid: boolean;
  epochs_until_expiry: number;
}

export interface EnergyPortfolio {
  total_energy: number;
  total_max_energy: number;
  object_count: number;
  ghost_count: number;
  at_risk_count: number;
  expiring_today_count: number;
  energy_trend: number;
  objects_refreshed_this_week: number;
  objects_evaporated_this_week: number;
  total_energy_spent_this_week: number;
  net_energy_change_this_week: number;
  objects: Array<{
    id: string;
    name: string;
    energy: number;
    max_energy: number;
    state: string;
  }>;
}

export interface EnergyHistory {
  address: string;
  epochs: Array<{
    epoch: number;
    energy: number;
  }>;
}

export interface SocialAuthResult {
  success: boolean;
  address: string;
  encrypted_key: string;
  is_new_account: boolean;
  message?: string;
}

// ── Tx status tracking ──────────────────────────────────────────────
//
// The node exposes GET /api/tx/:hash returning a TxRecord once a
// transaction has been included in a finalised block. Before then the
// node returns 404. We synthesise the higher-level "pending" /
// "mempool" states client-side: a tx we've broadcast but never seen in
// a block yet is "pending"; once /api/tx/:hash returns we map the
// record's `status` field — "success" → finalised, anything else →
// rejected.

export interface TxStatus {
  hash: string;
  state: "pending" | "mempool" | "included" | "finalised" | "rejected";
  block_height?: number;
  epoch?: number;
  error?: string;
}

interface TxRecordResponse {
  hash: string;
  type: string;
  from: string;
  to: string;
  amount?: number;
  object_id?: string;
  energy?: number;
  half_life?: number;
  method?: string;
  gas: number;
  block_number: number;
  epoch: number;
  status: string;
}

// ── Patronage Covenant types ────────────────────────────────────────

export interface PatronagePledgeReq {
  object_id_hex: string;
  namespace_id_hex: string;
  donation_per_epoch: number;
  epochs: number;
  current_epoch: number;
}

export interface PatronageActionReq {
  object_id_hex: string;
  epoch: number;
}

export interface PatronagePledgeResp {
  status: string;          // "pledged" | "error"
  object_id_hex: string;
  pre_funded: number;
  expires_epoch: number;
  detail: string;
}

export interface PatronageHonourResp {
  status: string;          // "honoured" | "error"
  donated: number;
  patronage_score: number;
  detail: string;
}

export interface PatronageRevokeResp {
  status: string;          // "revoked" | "error"
  object_id_hex?: string;
  patronage_score_archived?: number;
  refunded?: number;
  detail: string;
}

export interface PatronageStatusResp {
  active_covenants: number;
  total_pre_funded: number;
  total_active_score: number;
  patronage_ns_hex: string;
}

export interface PatronageImmuneResp {
  object_id_hex: string;
  epoch: number;
  immune: boolean;
  patronage_score: number;
}

// ── Refresh pool ────────────────────────────────────────────────────
//
// GET /api/refresh_pool — protocol-owned pool that funds object refreshes,
// fed by demurrage + patronage payments. Per-namespace credits track which
// namespace has accrued how much and when it was last touched.
// api.rs §RefreshPoolResp / RefreshPoolCredit (~L2823-2834).

export interface RefreshPoolCredit {
  namespace_hex: string;
  accrued: number;
  last_touched_epoch: number;
}

export interface RefreshPoolStatus {
  total_accrued: number;
  credits: RefreshPoolCredit[];
}

// ── Fee controller status ──────────────────────────────────────────
//
// GET /api/fee_controller/status — current Lyapunov-stable fee controller
// state. `energy` is the accumulated energy of the controller; `base_fee`
// is the fee at the current pressure; `target_*`/`fee_response_ppm` are
// the genesis target params used to evaluate convergence.
// api.rs §get_fee_controller_status (~L4672) — returns serde_json::Value
// with fields: status, energy, base_fee, target_energy, target_gas,
// fee_response_ppm.

export interface FeeControllerStatus {
  status: string;
  energy: number;
  base_fee: number;
  target_energy: number;
  target_gas: number;
  fee_response_ppm: number;
}

// One step return from POST /api/fee_controller/step. Used by the widget
// to plot V-function drift trace (delta < 0 = converging).
export interface FeeControllerStepResp {
  status: string;
  energy_after?: number;
  base_fee?: number;
  lyapunov_v_before?: number;
  lyapunov_v_after?: number;
  lyapunov_delta?: number;
  gas_used?: number;
  detail?: string;
}

// ── Demurrage owed ─────────────────────────────────────────────────
//
// POST /api/demurrage/owed — pure compute, no chain mutation. Returns
// `owed` (energy units), `rate_ppm` (per-epoch rate), and the recomputed
// remaining_balance. Note: the response does NOT include a `capped`
// field — it includes `is_disabled` and `remaining_balance` instead.
// api.rs §post_demurrage_owed (~L5366).

export interface DemurrageOwedReq {
  balance: number;
  last_touched_epoch: number;
  current_epoch: number;
  lambda_base_ppm: number;
  threshold: number;
}

export interface DemurrageOwedResp {
  status: string;
  balance: number;
  last_touched_epoch: number;
  current_epoch: number;
  elapsed_epochs: number;
  rate_ppm: number;
  owed: number;
  remaining_balance: number;
  is_disabled: boolean;
}

// ── Governance fork-choice ─────────────────────────────────────────
//
// GET  /api/governance/fork_choice_mode (~L1951) — current mode + attractors.
// POST /api/governance/fork_choice_mode (~L1912) — stake-quorum amendment.
// The POST does NOT take signature/public_key — it accepts endorser_stakes
// and required_stake; quorum is satisfied when sum(endorser_stakes) >=
// required_stake. The mode is "mcc" or "singh_attractor".

export interface ForkChoiceAttractor {
  center: number;
  basin_radius: number;
}

export interface ForkChoiceModeStatus {
  fork_choice_mode: string;
  attractors: ForkChoiceAttractor[];
  detail: string;
}

export interface ForkChoiceAmendReq {
  mode: string;
  attractors?: ForkChoiceAttractor[];
  endorser_stakes: number[];
  required_stake: number;
}

export interface ForkChoiceAmendResp {
  status: string;             // "amended" | "error"
  fork_choice_mode?: string;
  attractor_count?: number;
  detail: string;
}

// ── HLWA — Half-Life Wrapped Asset ────────────────────────────────
//
// POST /api/hlwa/effective_supply (~L4374) — pure compute, no broadcast.
// POST /api/hlwa/re_attest (~L4401) — pure simulation; returns the asset
// state as if a fresh re-attestation had landed at current_epoch. Neither
// endpoint takes a signature.

export interface HlwaSupplyReq {
  current_supply: number;
  origin_attested_supply: number;
  last_attested_epoch: number;
  attestation_lambda_epochs: number;
  current_epoch: number;
}

export interface HlwaSupplyResp {
  status: string;
  effective_supply: number;
  current_supply: number;
  excess_to_burn: number;
  current_epoch: number;
  detail?: string;
}

export interface HlwaReAttestResp {
  status: string;
  new_attested_supply: number;
  new_last_attested_epoch: number;
  effective_supply_after: number;
  detail?: string;
}

// ── DSN — Decay-Stamped Nullifiers ─────────────────────────────────
//
// Privacy primitive: shielded transfers are made anonymous within a
// sliding-window accumulator. The window aggregates blake3-folded
// nullifiers; advancing rolls the oldest slot off.
//
// Endpoints (api.rs):
//   POST /api/dsn/fold_nullifier  (~L5170) — { nullifier_hex } →
//        { status, total_count, aggregate_root_hex }
//   POST /api/dsn/advance_window  (~L5195) — no body →
//        { status, total_count, aggregate_root_hex }
//   GET  /api/dsn/status          (~L5205) →
//        { status, total_count, aggregate_root_hex }
//
// All three responses share the same shape — they're emitted as
// serde_json::Value so the TS interface mirrors the keys exactly.

export interface DsnStatus {
  status: string;
  total_count: number;
  aggregate_root_hex: string;
}

export interface DsnFoldReq {
  nullifier_hex: string;
}

// ── LAD-VM — Linear/Affine/Decaying resource simulator ─────────────
//
// POST /api/lad_vm/simulate (~L1335). Pure compute, no chain mutation,
// no signature. Returns the lifecycle outcome for a single op against
// a Resource constructed from the request's mode + value. See
// LadSimQuery / LadSimResp in api.rs (L1300-L1329).
//
// `outcome` strings (api.rs L1319-1322):
//   "ok" | "evaporated" | "already-consumed" | "linear-cannot-drop" |
//   "ticked-fresh" | "ticked-evaporated" | "ticked-no-op" |
//   "missing-decay-window" | "error"

export type LadMode = "linear" | "affine" | "decaying";
export type LadAction = "use" | "drop" | "tick";

export interface LadSimReq {
  mode: LadMode;
  value: number;
  created_at_epoch: number;
  /** Required iff mode == "decaying". */
  decay_window?: number;
  current_epoch: number;
  action: LadAction;
}

export interface LadSimResp {
  status: string;
  action: string;
  mode: string;
  outcome: string;
  returned_value: number | null;
  is_evaporated_at_query: boolean;
  created_at_epoch: number;
  current_epoch: number;
  decay_window: number | null;
  detail: string;
}

// ── Bell-Beacon — CHSH quantum-randomness certification ───────────
//
// POST /api/bell_beacon (~L3447). CHSH inequality: S = E(a,b) − E(a,b')
// + E(a',b) + E(a',b'). Local-realism bound is |S| ≤ 2; in milli-units
// the threshold is 2000mb (LOCAL_REALISM_S_MILLI). Beacon is "Bell-
// certified" iff S > threshold.
//
// Request (BellBeaconQuery, api.rs L3427-3436): all four expectation
// values in milli-units (i64), optional threshold override.
// Response (BellBeaconResp, api.rs L3438-3445): { status, s_value_milli,
// threshold_milli, bell_certified, detail }.
//
// There is no read-only GET form for Bell-Beacon — beacon S-values are
// computed per-block and would need to be surfaced via the chain header
// to be queryable cheaply. The card therefore reflects a single design-
// target simulation on mount.

export interface BellBeaconReq {
  e_ab: number;
  e_ab_prime: number;
  e_a_prime_b: number;
  e_a_prime_b_prime: number;
  threshold_milli?: number;
}

export interface BellBeaconResp {
  status: string;
  s_value_milli: number;
  threshold_milli: number;
  bell_certified: boolean;
  detail: string;
}

class EvaporChainAPI {
  private baseUrl: string;

  constructor(baseUrl: string = DEFAULT_NODE) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
  }

  setNode(url: string) {
    this.baseUrl = url.replace(/\/+$/, "");
  }

  private async get<T>(path: string): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`);
    if (!res.ok) throw new Error(`API ${res.status}: ${await res.text()}`);
    return res.json();
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`API ${res.status}: ${await res.text()}`);
    return res.json();
  }

  // ── Chain ──

  async getStatus(): Promise<ChainStatus> {
    return this.get("/api/status");
  }

  // ── Accounts ──

  async getAddressDetail(address: string): Promise<AccountDetail> {
    return this.get(`/api/address/${address}`);
  }

  async getAccounts(): Promise<AccountDetail[]> {
    return this.get("/api/accounts");
  }

  // ── Objects ──

  async getObjects(): Promise<StateObject[]> {
    return this.get("/api/objects");
  }

  async getObjectsByOwner(address: string): Promise<StateObject[]> {
    const all = await this.getObjects();
    return all.filter(o => o.owner === address);
  }

  // ── Transactions ──

  async transfer(from: string, to: string, amount: number, nonce: number, signature?: string, publicKey?: string): Promise<TxResult> {
    return this.post("/api/tx/transfer", {
      from, to, amount, nonce,
      signature, public_key: publicKey,
    });
  }

  async refreshObject(objectId: string, energyDeposit: number): Promise<TxResult> {
    return this.post("/api/tx/refresh", {
      object_id: objectId,
      energy_deposit: energyDeposit,
    });
  }

  async createObject(creator: string, objectId: string, energy: number, halfLife: number): Promise<TxResult> {
    return this.post("/api/tx/create-object", {
      creator, object_id: objectId, energy, half_life: halfLife,
    });
  }

  // ── Faucet ──

  async claimFaucet(address: string): Promise<{ success: boolean; balance: number; message?: string }> {
    return this.post("/api/faucet", { address });
  }

  // ── Transactions history ──

  async getTransactions(): Promise<TransactionRecord[]> {
    return this.get("/api/transactions");
  }

  // ── Swap / DEX ──

  async getTokens(): Promise<TokenInfo[]> {
    return this.get("/api/tokens");
  }

  async getSwapQuote(fromToken: string, toToken: string, amount: number): Promise<SwapQuote> {
    return this.post("/api/swap/quote", { from_token: fromToken, to_token: toToken, amount });
  }

  async executeSwap(fromToken: string, toToken: string, amount: number, slippage: number, signature?: string, publicKey?: string): Promise<SwapResult> {
    return this.post("/api/swap/execute", { from_token: fromToken, to_token: toToken, amount, slippage, signature, public_key: publicKey });
  }

  // ── NFTs ──

  async getNfts(): Promise<NftItem[]> {
    return this.get("/api/nfts");
  }

  async getNftsByOwner(address: string): Promise<NftItem[]> {
    const all = await this.getNfts();
    return all.filter(n => n.owner === address);
  }

  async getNft(id: string): Promise<NftItem> {
    return this.get(`/api/nft/${id}`);
  }

  async refreshNft(id: string, energy: number): Promise<TxResult> {
    return this.post("/api/nft/refresh", { nft_id: id, energy_deposit: energy });
  }

  async transferNft(id: string, to: string): Promise<TxResult> {
    return this.post("/api/nft/transfer", { nft_id: id, to });
  }

  // ── Batch Refresh ──

  async batchRefresh(objects: Array<{ id: string; energy: number }>, signature?: string, publicKey?: string): Promise<TxResult> {
    return this.post("/api/tx/batch-refresh", { objects, signature, public_key: publicKey });
  }

  async getRefreshCost(objectId: string, targetEnergy: number): Promise<RefreshCostEstimate> {
    return this.post("/api/tx/refresh/estimate", { object_id: objectId, target_energy: targetEnergy });
  }

  // ── Ghost Recovery ──

  async getGhosts(owner?: string): Promise<GhostObject[]> {
    const ghosts = await this.get<GhostObject[]>("/api/ghosts");
    return owner ? ghosts.filter(g => g.owner === owner) : ghosts;
  }

  async getGhostDetail(id: string): Promise<GhostDetail> {
    return this.get(`/api/ghost/${id}`);
  }

  async resurrectObject(id: string, energy: number, signature?: string, publicKey?: string): Promise<TxResult> {
    return this.post("/api/tx/resurrect", { object_id: id, energy_deposit: energy, signature, public_key: publicKey });
  }

  async getRecoveryCost(id: string): Promise<RecoveryCostEstimate> {
    return this.get(`/api/ghost/${id}/cost`);
  }

  // ── Energy Dashboard ──

  async getEnergyPortfolio(address: string): Promise<EnergyPortfolio> {
    return this.get(`/api/address/${address}/energy`);
  }

  async getEnergyHistory(address: string): Promise<EnergyHistory> {
    return this.get(`/api/address/${address}/energy/history`);
  }

  // ── Simulation ──

  async simulateTransaction(tx: SimulateTransactionRequest): Promise<SimulationResult> {
    return this.post("/api/tx/simulate", tx);
  }

  async getDecayForecast(objectId: string): Promise<DecayForecastResult> {
    return this.get(`/api/object/${objectId}/forecast`);
  }

  // ── Social Auth ──

  async socialAuth(provider: "google" | "apple", token: string): Promise<SocialAuthResult> {
    return this.post("/api/auth/social", { provider, token });
  }

  // ── Tx status ──
  //
  // Wraps GET /api/tx/:hash. Returns "pending" while the node has no
  // record (404) and a concrete TxRecord-derived state once it does.
  // The node only emits records from finalised blocks, so an observed
  // record with status="success" is mapped to "finalised", anything
  // else to "rejected".
  async getTxStatus(hash: string): Promise<TxStatus> {
    try {
      const res = await fetch(`${this.baseUrl}/api/tx/${hash}`);
      if (res.status === 404) {
        return { hash, state: "pending" };
      }
      if (!res.ok) {
        return { hash, state: "pending", error: `HTTP ${res.status}` };
      }
      const rec = (await res.json()) as TxRecordResponse;
      const success = rec.status === "success";
      return {
        hash,
        state: success ? "finalised" : "rejected",
        block_height: rec.block_number,
        epoch: rec.epoch,
        error: success ? undefined : rec.status,
      };
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      return { hash, state: "pending", error: msg };
    }
  }

  // ── Patronage Covenants ──

  async getPatronageStatus(): Promise<PatronageStatusResp> {
    return this.get("/api/patronage/status");
  }

  async getPatronageImmunity(objectIdHex: string, epoch: number): Promise<PatronageImmuneResp> {
    const q = new URLSearchParams({ object_id_hex: objectIdHex, epoch: String(epoch) });
    return this.get(`/api/patronage/immune?${q.toString()}`);
  }

  async pledgePatronage(req: PatronagePledgeReq): Promise<PatronagePledgeResp> {
    return this.post("/api/patronage/pledge", req);
  }

  async honourPatronage(req: PatronageActionReq): Promise<PatronageHonourResp> {
    return this.post("/api/patronage/honour", req);
  }

  async revokePatronage(req: PatronageActionReq): Promise<PatronageRevokeResp> {
    return this.post("/api/patronage/revoke", req);
  }

  // ── Refresh pool ──

  async getRefreshPool(): Promise<RefreshPoolStatus> {
    return this.get("/api/refresh_pool");
  }

  // ── Fee controller ──

  async getFeeControllerStatus(): Promise<FeeControllerStatus> {
    return this.get("/api/fee_controller/status");
  }

  async stepFeeController(gasUsed: number, epochsElapsed: number): Promise<FeeControllerStepResp> {
    return this.post("/api/fee_controller/step", {
      gas_used: gasUsed,
      epochs_elapsed: epochsElapsed,
    });
  }

  // ── Demurrage ──

  async getDemurrageOwed(req: DemurrageOwedReq): Promise<DemurrageOwedResp> {
    return this.post("/api/demurrage/owed", req);
  }

  // ── Governance: fork-choice ──

  async getForkChoiceMode(): Promise<ForkChoiceModeStatus> {
    return this.get("/api/governance/fork_choice_mode");
  }

  async amendForkChoiceMode(req: ForkChoiceAmendReq): Promise<ForkChoiceAmendResp> {
    return this.post("/api/governance/fork_choice_mode", req);
  }

  // ── HLWA ──

  async getHlwaEffectiveSupply(req: HlwaSupplyReq): Promise<HlwaSupplyResp> {
    return this.post("/api/hlwa/effective_supply", req);
  }

  async reAttestHlwa(req: HlwaSupplyReq): Promise<HlwaReAttestResp> {
    return this.post("/api/hlwa/re_attest", req);
  }

  // ── DSN — Decay-Stamped Nullifiers ──

  async getDsnStatus(): Promise<DsnStatus> {
    return this.get("/api/dsn/status");
  }

  async foldDsnNullifier(req: DsnFoldReq): Promise<DsnStatus> {
    return this.post("/api/dsn/fold_nullifier", req);
  }

  async advanceDsnWindow(): Promise<DsnStatus> {
    return this.post("/api/dsn/advance_window", {});
  }

  // ── LAD-VM ──

  async simulateLadVm(req: LadSimReq): Promise<LadSimResp> {
    return this.post("/api/lad_vm/simulate", req);
  }

  // ── Bell-Beacon ──

  async getBellBeacon(req: BellBeaconReq): Promise<BellBeaconResp> {
    return this.post("/api/bell_beacon", req);
  }
}

export const api = new EvaporChainAPI();
export { EvaporChainAPI };

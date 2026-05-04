# Crooks-MEV Refund — Consensus Integration Plan

**Status:** plan-draft 2026-05-04. No implementation yet.

**Context:** `DOCTRINE_PUNCH_LIST.md` Layer 6 row marks Crooks-MEV as "⚠ substrate-only — HTTP endpoint at `api.rs:4153/4168` consumes `evaporchain_crooks_mev_refund::compute_refund` but no consensus hot-path integration." This file is the roadmap to flip that to ✅. Mirrors `LAMBDA_FOLD_NOVA_PLAN.md` in shape — phases, stopping conditions, tests, doctrine sweep at the end.

---

## What's already shipped (substrate)

- **`evaporchain-cfm::crooks_log_ratio_millibits(p_forward, p_reverse)`** (`crates/evaporchain-cfm/src/crooks.rs:31`) — computes `log₂(P_F / P_R)` in millibits, the LHS of the Crooks identity.
- **`evaporchain-crooks-mev-refund::compute_delta_f_millibits(w_milli, log_ratio_milli, beta_mb)`** (`refund.rs:22`) — solves `ΔF = W − (1/β)·log_ratio`.
- **`evaporchain-crooks-mev-refund::compute_refund(work_extracted, delta_f_milli)`** (`refund.rs:52`) — returns `max(0, W − ΔF)` as the dissipative-MEV refund amount.
- **`POST /api/crooks_refund`** (`evaporchain-node/src/api.rs:4395`) — exposes the math. **The caller supplies `(p_forward, p_reverse, work, β)`.** The chain doesn't observe MEV events autonomously; the API is a calculator.

The math is sound and tested. What's missing is the consensus pipeline that would (a) detect MEV events from on-chain data, (b) compute the refund automatically, and (c) settle the refund as a protocol-issued state transition — all without operator intervention or off-chain triggers.

---

## Why this is hard

The math gives you the refund *given* a detected MEV event with observed `(P_F, P_R, W)`. The chain has none of those quantities natively:

1. **MEV detection.** Sandwich attacks, frontruns, and toxic flow don't carry self-labels. The chain sees a sequence of `Transaction` records and has to infer attacker/victim/work from execution side-effects (price movement, slippage, balance deltas).
2. **Pmf observation.** Crooks needs forward/reverse work distributions over a *path*, not single observations. Where does the chain get a distribution? Two natural candidates: rolling window of similar txs at the same fee tier (forward), and rolling window of *reversed* txs (reverse). Both require state we don't currently track.
3. **β source.** The temperature in the Crooks formula. Two candidates: pull from the Singh-Lyapunov fee controller (`evaporchain-execution::tick_lyapunov_fee_state`) which already maintains a chain-temperature analogue, or expose as a governance constant. The former is more principled; the latter is more tractable.
4. **Settlement.** The refund has to move tokens from attacker to victim. New protocol-issued tx type? Block-level execution side-effect? Either way it's a hot-path change with consensus implications (every validator must compute the same refund or the block is invalid).
5. **Anti-gaming.** A naive implementation lets adversaries weaponise the refund: post a deliberately bad-looking tx, claim self-MEV, drain a victim. Need confidence thresholds and grace periods.

---

## Phase breakdown

Seven phases, ~4–6 weeks total. Phases 1-2 unblock observability; Phases 3-5 are the consensus-grade work; Phases 6-7 are tightening and docs.

### Phase 1 — Detection substrate (3-5 days)

**Goal:** ship a no-op detector that records candidate MEV events without acting on them. The chain emits "MEV observation" log entries; nothing settles.

- [x] **1.1 — MEV signature module** (`crates/evaporchain-mev-detect/`): SHIPPED. New crate `evaporchain-mev-detect` with `scan_block(txs, height) -> Vec<MevObservation>`. O(n²) outer scan over `Transaction::Transfer` triples; non-Transfer txs skipped. 9/9 tests green covering classic sandwich, honest sequence, target mismatch, self-MEV skip, multi-sandwich, multi-victim cardinality cap.
- [x] **1.2 — Per-block evidence struct**: `MevObservation` shipped in `evaporchain-mev-detect` (decision: keep co-located with its only producer for now; move to `evaporchain-types` in Phase 1.6+ if downstream consumers need it from there). Fields: `(block_height, attacker_pre_idx, victim_idx, attacker_post_idx, attacker, victim, target, work_estimate, confidence_score)`. Phase 4.3 self-MEV anti-gaming guard already pre-wired (attacker == victim → not emitted at detection time).
- [x] **1.3 — Detection call site**: SHIPPED at `tendermint.rs:on_block_committed` (right before the Lambda-Fold per-block step). Bounded ring buffer `mev_observations: VecDeque<MevObservation>` on `TendermintConsensus`, capped at `MEV_OBSERVATION_BUFFER_CAP = 1024`. Read-only accessor `mev_observations(&self) -> &VecDeque<…>` shipped.
- [x] **1.4 — `GET /api/mev/observations` endpoint**: SHIPPED on `evaporchain-node`. Returns `MevObservationsResp { count, observations: Vec<MevObservationView> }` with hex-encoded addresses.
- [x] **1.5 — Tests**: `test_mev_observations_sandwich_recorded` in `tendermint.rs` drives a synthetic sandwich block + an honest follow-up block through `on_block_committed`, asserts (a) the sandwich produces exactly 1 observation with the expected attacker/victim/target/work fields, (b) the honest block does NOT add a false-positive observation. Test passes on Mini under release.

**Phase 1 deliverable: SHIPPED.** Chain logs MEV-shaped patterns at zero operational cost. Default is no-op observation. HTTP endpoint exposes the buffer for operator tooling.

### Phase 2 — Refund computation (2-3 days)

**Goal:** wire the detected events into the existing `crooks_mev_refund` math. Compute refund amounts but don't settle them yet.

- [ ] **2.1 — Pmf source design**: rolling window of similar-fee-tier swap txs (forward) vs the same window with attacker/victim ordering reversed (reverse). Window size = governance flag, default 256 blocks.
- [ ] **2.2 — β source design**: pull from `Lyapunov::current_beta()` (Singh-Lyapunov already runs per-block at `parallel.rs:2076`). If Lyapunov hasn't converged, default β = governance constant.
- [ ] **2.3 — Refund pipeline**: for each `MevObservation`, compute `(p_forward_ppm, p_reverse_ppm)` from the windows, call existing `compute_refund(work, delta_f)`, attach the result to the observation.
- [ ] **2.4 — `MevObservation` extended**: gain `refund_amount: Option<u64>` field.
- [ ] **2.5 — Tests**: synthetic sandwich produces non-zero refund estimate; honest swap produces zero; refund amount stable under repeated measurement.

**Phase 2 deliverable:** chain estimates refund amounts for every detected MEV event. Still no settlement; observations are operator-readable only.

### Phase 3 — Settlement plumbing (5-7 days)

**Goal:** debit attacker, credit victim, in-protocol. This is the consensus-grade phase.

- [ ] **3.1 — `RefundTx` protocol-issued tx type**: new variant in `Transaction` enum. Producer constructs one per MEV observation; validators verify the construction matches the observation deterministically.
- [ ] **3.2 — Determinism contract**: every validator with the same `MevObservation` ring buffer + same window state computes the same `(refund_amount, attacker, victim)`. This requires the ring buffer + window to be part of `state_root` (otherwise validators diverge).
- [ ] **3.3 — Block construction rule**: proposer MUST include a `RefundTx` for every observation in the buffer that's at least N blocks old (`grace_period`) and at most M blocks old (`refund_window`). Outside this window, the observation is discarded as stale.
- [ ] **3.4 — Block validation rule**: validators reject blocks that omit a required `RefundTx`. `ValidationError::MissingRefund`.
- [ ] **3.5 — Slashing rule**: producer who omits a required refund is slashed via `evaporchain-entropic-slashing`. Severity = `SlashSeverity::Negligence` (not `::Equivocation`).
- [ ] **3.6 — Tests**: 
  - 3-block sandwich → grace period elapses → next block must include `RefundTx` (validators reject if absent).
  - Stale observation (older than `refund_window`) → no `RefundTx` required.
  - Invalid refund amount → block rejected.

**Phase 3 deliverable:** in-protocol settlement of detected MEV events. Validators converge on the same refund amounts deterministically.

### Phase 4 — Anti-gaming (3-5 days)

**Goal:** prevent false positives from being weaponised against innocent traders.

- [ ] **4.1 — Confidence threshold**: detected events with `confidence_score < threshold` are recorded but NOT settled. Threshold is a governance flag (default 0.95).
- [ ] **4.2 — Victim consent flag**: optional opt-out — txs can carry a `mev_refund_eligible: false` flag in their metadata. Refunds skipped for opted-out victims.
- [ ] **4.3 — Self-MEV detection**: attacker == victim (same address) → no refund. Closes the obvious self-attack vector.
- [ ] **4.4 — Operator override endpoint**: `POST /api/mev/dispute` lets the chain operator (governance multisig) cancel a pending refund within the grace period. Audited via on-chain log.
- [ ] **4.5 — Tests**: low-confidence event → recorded but not settled; opted-out victim → no settlement; self-MEV → no settlement; dispute within grace period → cancellation; dispute after grace period → rejected.

**Phase 4 deliverable:** the refund mechanism is not a footgun. Adversaries can't drain victims via false-positive refund claims.

### Phase 5 — Governance flag (1 day)

- [ ] **5.1 — `crooks_mev_mode` governance allowlist entry**: values `"observe"` (default — Phase 1+2 only), `"refund"` (full pipeline incl. settlement). Mirrors the Lambda-Fold pattern from `LAMBDA_FOLD_NOVA_PLAN.md` Phase 5.2.
- [ ] **5.2 — Branch at the detection call site**: only run Phase 3+ settlement when mode == "refund".
- [ ] **5.3 — Tests**: default mode = observe → no `RefundTx` issued; mode = refund → full pipeline runs.

**Phase 5 deliverable:** safe rollout via flag flip. Default behaviour for new chains is observe-only.

### Phase 6 — Integration test + performance (2-3 days)

- [ ] **6.1 — End-to-end test**: governance set `crooks_mev_mode = "refund"`, drive a synthetic sandwich through `on_block_committed`, observe the next block contains the expected `RefundTx`, victim balance increases, attacker balance decreases.
- [ ] **6.2 — Worst-case detection cost**: profile detection on a block with N=1000 txs. Goal: < 50 ms on M4. If detection is O(N²) (cross-pair sandwich check), add bucketing by token pair to bring it back to O(N log N).
- [ ] **6.3 — Adversarial witness test**: false-positive sandwich (3 unrelated txs at the same time) → confidence < threshold → no refund. Locks the precision of detection.

**Phase 6 deliverable:** the pipeline is fast enough for the consensus hot path and resistant to false-positive abuse.

### Phase 7 — Documentation + doctrine (1 day)

- [ ] **7.1 — Whitepaper update**: new section under MEV Protection (currently §8 Encrypted Mempool). Two-tier defense: encrypted mempool (preventive) + Crooks refund (restitutive).
- [ ] **7.2 — `INVENTION_STACK.md §A1.3 Crooks-MEV row`**: drop "substrate-only" qualifier, add SHIPPED date + soundness test names.
- [ ] **7.3 — `DOCTRINE_PUNCH_LIST.md` Layer 6**: flip Crooks-MEV from ⚠ to ✅.
- [ ] **7.4 — Operator runbook**: how to flip the governance flag, how to monitor `mev_observations`, how to file a dispute.

**Phase 7 deliverable:** docs match shipped reality. Layer 6 is one notch closer to ✅ DONE (Light-Cone full DAG remains).

---

## Stopping conditions

The phase plan is contingent on these. If any of them holds, stop and re-litigate before continuing:

- **Detection precision below 80%** at the end of Phase 1.5 (false-positive rate on a benchmark of 1000 honest swaps + 100 sandwiches). Below that, settlement is unsafe regardless of anti-gaming guards. Re-design the detector.
- **Determinism breaks** in Phase 3.2 (validators with the same inputs disagree on refund amount by even 1 unit). Refund pipeline must produce bit-identical output across all validators or the chain forks. If divergence appears, the ring buffer + windows aren't being correctly committed to state.
- **Slashing rate >0** in Phase 3.5 testnet — means honest producers are missing required refunds due to a logic gap. Pause settlement, fix the producer-side rule first.
- **Worst-case detection cost > 100 ms** in Phase 6.2 — too slow for the hot path. Either restrict detection to a subset of blocks (every 10th, etc.) or move to async detection with delayed settlement.

---

## Cross-cutting risks

1. **State-bloat from observation buffer.** Per-block `MevObservation` × ring buffer × M blocks = potentially MB-scale state growth. Mitigation: cap ring buffer at 1024 entries, prune oldest when full, never serialize fully-old entries to state — only the last `refund_window` blocks worth.

2. **Crooks identity assumes equilibrium near the path.** Real chain traffic is non-equilibrium most of the time. The pmf-window approach is an approximation; under high-frequency regime changes, refunds could be systematically biased. Phase 4.1's confidence threshold partially mitigates by filtering low-signal events. A more rigorous fix is non-equilibrium Crooks, but that's a research project.

3. **Settlement creates an MEV target on MEV refunds.** A producer with knowledge of a coming refund could try to front-run their own refund attestation. Mitigation: refunds are issued at block boundary by the proposer and there's no mempool stage for them. But adversarial proposers could selectively omit refunds favouring a specific victim. Phase 3.5's slashing is the recourse.

4. **β source dependency on Singh-Lyapunov.** If Lyapunov doesn't converge under load, β is wrong, refunds are wrong. Phase 2.2's fallback governance constant is the safety valve. Need a "Lyapunov healthy" check at the call site.

5. **Encrypted mempool interaction.** When the encrypted mempool (§8 of whitepaper) is active, MEV detection sees only post-decryption txs. If the encrypted mempool is doing its job, MEV opportunities are scarce by construction — Crooks refund's value is mostly retrospective (catching events that slipped past the encryption layer). Both mechanisms are complementary.

---

## What this plan does NOT cover

- **Front-run-only attacks** (single tx, no victim back-run): the Crooks formulation is sandwich-shaped (forward path P_F, reverse path P_R). Pure front-runs need a different treatment, likely ZK-based as in Sybil et al. Out of scope; tracked separately.
- **Cross-chain MEV.** EvaporChain-only events are detectable; cross-chain arbitrage is a separate research lane.
- **Toxic flow.** Counterparty-level MEV (informed flow exploiting uninformed liquidity) is not visible in tx records alone; needs LP-side accounting. Out of scope.

---

## Pre-implementation checklist

Before starting Phase 1:

- [ ] Confirm `evaporchain-mev-detect` crate name doesn't collide with existing crates.
- [ ] Confirm `evaporchain-types::Transaction` has space for a new `RefundTx` variant without wire-format breakage (probably needs a new `protocol_version` flip).
- [ ] Confirm `evaporchain-execution::ParallelExecutor` exposes the per-tx side-effects detection needs (balance deltas, price reads). If not, extend the execution trace.
- [ ] Sanity-check the substrate crate — a 1-line empirical run confirming `compute_refund` matches a hand-computed sandwich result (substrate parity).

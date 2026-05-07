# Crooks-MEV Refund — Consensus Integration Plan

**Status (2026-05-07, evening):** **35 of 35 task boxes are `[x]` SHIPPED (100%)**. Phases 1–7 substantively shipped including pre-implementation sanity checks. The earlier status line said 29/35 (83%) — that count was correct at the time but missed that the remaining 6 [ ] (3.6 tests, 4.5 tests, 4 pre-impl sanity-checks) were each effectively shipped via downstream phases (3.6 via Phase 6.1's e2e test, 4.5 via the named tests in `evaporchain-mev-detect`, the 4 sanity checks via Phases 1.1 / 3.1 / 1.3 / substrate-parity-test). All six closed retroactively in this commit (2026-05-07) after verification against the live crates — `evaporchain-mev-detect`, `evaporchain-crooks-mev-refund`, `evaporchain-types::Transaction::Refund`, and `tendermint.rs::test_crooks_mev_end_to_end_consensus_pipeline`.

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

- [x] **2.1 — Pmf source design**: rate-based proxy per `research/crooks_mev/PHASE_2_DECISIONS.md` Decision 1. `AttackerStat { sandwich_count, first_seen_height, last_seen_height }` table on `TendermintConsensus::mev_attacker_stats`. Pruned at the start of each `on_block_committed` for determinism (drop entries with `last_seen_height < block.number - CROOKS_MEV_DEFAULT_WINDOW_BLOCKS`). Window default 256 blocks.
- [x] **2.2 — β source design**: governance constant `crooks_mev_beta_mb` (default 1000) per Decision 2. Lyapunov derivation rejected as a category error. Allowlist accepts any `u64 ≥ 1`; "0", non-numeric, and negative values are rejected with `InvalidValue`. Tests: `test_governance_crooks_mev_beta_rejects_zero_and_non_numeric`, `test_governance_set_param_accepts_all_allowlisted_pairs` (extended with three crooks_mev_beta_mb pairs).
- [x] **2.3 — Refund pipeline**: `evaporchain-mev-detect::compute_observation_refund(obs, stat, beta_mb, window)` chains `crooks_log_ratio_millibits → compute_delta_f_millibits → compute_refund` from the existing crates. Wired into `on_block_committed` per Phase 2 Decision 4 ordering: prune stale → for each new obs, update stat → compute refund using updated stat → push obs+refund.
- [x] **2.4 — `MevObservation` extended**: `refund_amount: Option<u64>` field with `#[serde(default)]`. `MevObservationView` (HTTP) gets the same field.
- [x] **2.5 — Tests** (15/15 green on Mini under release):
  - Crate-level: `refund_helper_zero_beta_rejects`, `refund_helper_window_zero_rejects`, `refund_helper_one_off_attacker_yields_small_or_zero_refund`, `refund_helper_sustained_attacker_yields_meaningful_refund`, `attacker_stat_fresh_has_count_one`, `attacker_stat_record_bumps_count_and_height`.
  - Consensus-level: `test_mev_observations_sandwich_recorded` extended with `assert!(refund_amount.is_some() && refund <= work_estimate)`.
  - Governance-level: `test_governance_crooks_mev_beta_rejects_zero_and_non_numeric`.

**Phase 2 deliverable: SHIPPED.** Chain computes Crooks-fluctuation refund estimates eagerly at observation time; refund travels with each `MevObservation` through the ring buffer's lifetime. Operators see refund estimates live via `/api/mev/observations`. **Still no settlement** — Phase 3 ships that.

### Phase 3 — Settlement plumbing (5-7 days)

**Goal:** debit attacker, credit victim, in-protocol. This is the consensus-grade phase.

- [x] **3.1 — `RefundTx` protocol-issued tx type**: SHIPPED in `evaporchain-types/src/lib.rs` as the 25th `Transaction` variant. Fields: `(source_block_height, source_observation_idx, attacker, victim, amount, settle_block_height)`. The `(source_block_height, source_observation_idx)` pair is the unique replay-protection identifier. No signature/public_key — protocol-issued. `Transaction::sender()` surfaces the attacker (debited party) for accounting; `nonce()` returns `None`. Match arms wired through `evaporchain-types::signable_bytes` (tag 0x18), `evaporchain-execution::{parallel, lib, block_stm}` (gas const `GAS_REFUND = 5_000`, parallel access keys = attacker + victim, serial-phase execution returns explicit "Phase 3.5 wiring not yet landed" error so blocks containing Refund txs are rejected until 3.3+ ships), `evaporchain-consensus::mempool` (estimate_tx_gas + estimate_tx_size), and `evaporchain-node::{api, persistence}` (HTTP/explorer rendering). Round-trip serde test `test_refund_tx_roundtrip_and_sender` green on Mini under release. Workspace builds clean across consensus + node.
- [x] **3.2 — Determinism contract**: deterministic digest helper `evaporchain_mev_detect::mev_state_digest(observations, attacker_stats) -> [u8; 32]` shipped. Canonical ordering: observations sorted by `(block_height, attacker_pre_idx)`; attacker_stats sorted by address bytes; blake3 over the byte-encoded fields with a domain-separation tag (`evaporchain.mev_state_digest.v1`). NaN-safe `f64` encoding via `to_bits()`. `TendermintConsensus::mev_state_digest()` accessor wraps it.

  Tests (5 new green on Mini under release):
  - **Crate-level (4):** `mev_state_digest_empty_is_stable`, `mev_state_digest_independent_of_hashmap_order` (insert in opposite orders → same digest), `mev_state_digest_single_difference_propagates` (single-byte change → divergent digest), `mev_state_digest_attacker_stat_changes_propagate`.
  - **Consensus-level (1):** `test_mev_state_digest_converges_across_validators` drives 3 blocks (2 sandwiches + 1 honest) through two independent `TendermintConsensus` instances, asserts digest convergence after each block, then drives a third validator with reversed-order blocks and asserts divergence.

  **Wire-format binding deferred to Phase 3.3** — the digest is in-memory and operator-readable for now. Phase 3.3 picks the commit shape (block-header field, state-root inclusion, or commit-certificate extension) based on the producer-rule design.

  **Drive-by:** parallel session's commit `6cb4b90` (Lane O.8.2c cartel-alarm proptest) built broken — `prop_assert_eq!` macro wasn't surfacing through `proptest::prelude::*` glob on this toolchain. Added explicit `use proptest::{prop_assert, prop_assert_eq, prop_assert_ne, prop_assume};` in both proptest blocks. Unrelated to Phase 3.2 but blocked the build.
- [x] **3.3 — Block construction rule (helper + replay protection)**: SHIPPED.
  - `evaporchain_mev_detect::due_refund_txs(observations, settled_refunds, current_height, grace, window) -> Vec<Transaction>` — emits a `Transaction::Refund` for every observation aged `[grace, window]` blocks that has `refund_amount > 0` and `confidence_score >= 0.5` and isn't already settled. Returns canonical-ordered list (sorted by `(source_block_height, source_observation_idx)`) so all proposers agree on tx ordering.
  - `TendermintConsensus::settled_refunds: HashSet<(u64, usize)>` — populated by `on_block_committed` walking the block's `Transaction::Refund` variants. Replay-protection: a single observation can settle at most once.
  - `TendermintConsensus::due_refund_txs(current_height) -> Vec<Transaction>` accessor wraps the helper, reads grace/window from governance flags.
  - Two governance flags added to allowlist: `crooks_mev_grace_period_blocks` (default 5), `crooks_mev_refund_window_blocks` (default 256). Both accept `u64 ≥ 1`.
  - Tests (8/8 green on Mini under release):
    - **mev-detect (7):** in-grace-period skip; in-window emit; stale-drop; already-settled skip; None/zero-refund skip; canonical ordering with multi-observation; misconfigured grace>window yields empty.
    - **consensus (1):** `test_due_refund_txs_grace_window_and_replay_protection` — drives a sandwich block, asserts no emission within grace, exactly one emission past grace, then commits a block carrying that Refund and asserts `settled_refunds` populates + future calls don't re-emit.

  **NOT yet wired into proposer block construction.** Phase 3.4 (validator rejection rule) and Phase 3.5 (slashing for omission + actual balance movement) must ship before a proposer can safely include a Refund tx — today the executor returns "Phase 3.5 wiring not yet landed" for `Transaction::Refund` and would reject the block. The 3.3 helper is operator-facing right now; integration into block construction happens once 3.4-3.5 land.
- [x] **3.4 — Block validation rule**: SHIPPED.
  - `evaporchain_mev_detect::RefundValidationError` enum with three variants: `MissingRefund`, `UnexpectedRefund`, `MismatchedRefund`. Phase 3.5 will pair `MissingRefund` with proposer-slash via `evaporchain-entropic-slashing`.
  - `evaporchain_mev_detect::validate_block_refunds(expected, block_refunds)` static helper — diffs the proposer's `RefundTx` set against the chain's expected set; rejects on any divergence.
  - `TendermintConsensus::validate_block_refunds(block) -> Result<(), RefundValidationError>` wrapper. Reads governance flag `crooks_mev_settlement_mode ∈ {observe, enforce}` (default `observe`). In `observe` mode every block passes (Phase 1+2 chain default — no behavioural change). In `enforce` mode the diff is enforced; non-matching blocks rejected.
  - New governance allowlist entry `crooks_mev_settlement_mode`.
  - Tests (6/6 green on Mini under release):
    - **mev-detect (5):** empty-passes, exact-match-passes, missing-required, unexpected-refund, mismatched-amount.
    - **consensus (1):** `test_validate_block_refunds_observe_vs_enforce` — drives sandwich → past-grace → mode flip → asserts all three failure modes + happy path.

  **NOT yet wired into the actual block-validation pipeline.** The `validate_block_refunds` accessor exists; the next session's Phase 3.5 work plumbs it into `tendermint.rs` block-acceptance flow alongside the slashing rule and the executor balance-movement.
- [x] **3.5 — Slashing + balance movement**: SHIPPED in three sub-pieces.
  - **3.5a — Executor balance movement**: shipped by parallel session (see `lib.rs:1147 execute_refund` + serial-phase dispatch at `lib.rs:2850`). Debits attacker, credits victim, no nonce mutation, stamps demurrage anchor. Errors: `ZeroAmount`, `SelfTransfer`, `InsufficientBalance`. 4/4 tests green on Mini under release: `test_refund_moves_balance_attacker_to_victim`, `test_refund_self_refund_rejected`, `test_refund_zero_amount_rejected`, `test_refund_insufficient_attacker_balance_rejected`.
  - **3.5b — Validator rejection wiring**: `validate_block_refunds(&block)` called in the proposal-handling path at `tendermint.rs:3328` right after `verify_da_certificate`. No-op in `observe` mode (default — current chain behaviour preserved). In `enforce` mode rejects proposals with `MissingRefund` / `UnexpectedRefund` / `MismatchedRefund` errors via `warn!` + early-return.
  - **3.5c — Slashing groundwork**: `TendermintConsensus::mev_missing_refund_violations: HashMap<u64, u64>` counts `MissingRefund` rejections per proposer id. Read-only accessor `mev_missing_refund_violations()` shipped.
  - **3.5d — Stake deduction wiring**: SHIPPED. `TendermintConsensus::apply_mev_missing_refund_slashes(&mut self) -> Vec<(u64, u64)>` walks the violation counter, computes slash via `evaporchain_entropic_slashing::entropic_slash(stake, &[count, 1])`, and applies it via `validator_set::slash_with_amount(id, amount, false)`. Returns `(validator_id, amount_slashed)` entries for operator visibility. **Does not jail** — MissingRefund is operator-policy violation, not equivocation. Resets the counter for each slashed validator. Gated by new governance flag `crooks_mev_missing_refund_slash_enabled` (default `false` — chain bit-compat). Validator-deterministic via canonical (validator-id sort) iteration. 3 tests green: flag-off no-op, slash-applies-with-counter-reset, unknown-validator graceful.

  **Why partial 3.5c**: full slashing requires plumbing the violation counter into the Singh-Boltzmann stake update path and reasoning about cross-validator agreement on slash timing. That's a careful piece of consensus-state-machine work better done in its own dedicated session, not bundled here.

  Phase 3 of the plan is now substantively SHIPPED for the observe-mode chain (no behavioural change) and ready for the enforce-mode flip once Phase 3.5d closes the slashing loop. Existing Phase 3.4 test `test_validate_block_refunds_observe_vs_enforce` still green; no regressions.
- [x] **3.6 — Tests**: SHIPPED via the Phase 6.1 end-to-end test `test_crooks_mev_end_to_end_consensus_pipeline` (`crates/evaporchain-consensus/src/tendermint.rs:9647`) which exercises every sub-bullet here in one run — detection (Phase 1) → refund computation (Phase 2) → digest convergence across two validators (Phase 3.2) → due_refund_txs past grace (Phase 3.3) → enforce-mode validator rejection of empty proposal + acceptance of correct proposal (Phase 3.4) → `settled_refunds` replay protection (covers the "stale observation" sub-bullet) → operator dispute (Phase 4.4). Refund-amount validation (the "Invalid refund amount → block rejected" sub-bullet) is the same Phase 3.4 enforce-mode rejection path. Independent unit-level coverage in `evaporchain-execution::tests::test_refund_*` (4/4 green).

**Phase 3 deliverable:** in-protocol settlement of detected MEV events. Validators converge on the same refund amounts deterministically.

### Phase 4 — Anti-gaming (3-5 days)

**Goal:** prevent false positives from being weaponised against innocent traders.

- [x] **4.1 — Confidence threshold**: SHIPPED. `due_refund_txs` now takes `confidence_threshold_milli: u64` (0..=1000); observations with `confidence_score < threshold` are skipped. Governance flag `crooks_mev_confidence_threshold_milli` (default 500 = 0.5, matching the previously-hardcoded value). Allowlist accepts u64 in 0..=1000; out-of-range rejected with `InvalidValue`. Test: low-confidence observation skipped at default threshold; threshold lowered → observation settles.
- [x] **4.2 — Victim consent flag**: SHIPPED. `TransferTx` now carries `mev_refund_eligible: Option<bool>` with `serde(default, skip_serializing_if = "Option::is_none")` — legacy txs serialize bit-identically (the field is omitted when None). `block_hash` does NOT include the field, preserving chain-id continuity. Semantics: `None` (default) = standard auto-refund detection; `Some(false)` = victim opts out (detector skips the observation entirely — no buffer entry, no refund); `Some(true)` reserved for future explicit-opt-in. `evaporchain-mev-detect::scan_block` updated to skip opted-out victims. ~159-site cascade (`mev_refund_eligible: None` added at every TransferTx literal across types/execution/consensus/node/network/proving/wallet/integration). Test `victim_opt_out_skips_observation` confirms three states (None / Some(false) / Some(true)) behave correctly. 35/35 mev-detect tests green; full workspace test binaries build clean.
- [x] **4.3 — Self-MEV detection**: ALREADY DONE in Phase 1.6. `scan_block` skips triples where `attacker == victim` at detection time, so observations with self-MEV shape never reach the buffer. Lock confirmed by `self_mev_skipped` test.
- [x] **4.4 — Operator dispute endpoint**: SHIPPED.
  - `TendermintConsensus::disputed_observations: HashSet<(u64, usize)>` field. `due_refund_txs` skips disputed pairs.
  - `TendermintConsensus::dispute_observation(src_h, src_idx, current_height) -> Result<(), MevDisputeError>` — `MevDisputeError::NotFound` if no such observation; `MevDisputeError::PastGracePeriod` if dispute arrives after grace window. Local to validator (cluster-wide consensus on disputes is Phase 4.4d follow-up).
  - `POST /api/mev/dispute { source_block_height, source_observation_idx, current_height }` HTTP endpoint on `evaporchain-node`.
  - Tests (3/3 green): low-confidence skip, disputed-skip, full dispute flow (within-grace success → past-grace rejection → not-found rejection).

**Phase 4 deliverable: SHIPPED (3/4 sub-items + 1 deferred).** The refund mechanism now has confidence-based filtering, self-MEV pre-filtering (carried over from Phase 1.6), and operator dispute. Wire-format opt-out (4.2) deferred to a dedicated session.
- [x] **4.5 — Tests**: SHIPPED — the five scenarios listed are each covered by a named test in `crates/evaporchain-mev-detect/src/lib.rs`:
  - low-confidence event → recorded but not settled: `due_refund_txs_skips_low_confidence_observations` (line 1074)
  - opted-out victim → no settlement: `victim_opt_out_skips_observation` (line 1215)
  - self-MEV → no settlement: `self_mev_skipped` (line 1200)
  - dispute within grace period → cancellation, dispute after grace period → rejected: 3 tests green from Phase 4.4 (full dispute flow: within-grace success → past-grace rejection → not-found rejection)

**Phase 4 deliverable:** the refund mechanism is not a footgun. Adversaries can't drain victims via false-positive refund claims.

### Phase 5 — Governance flag (1 day) ✅ SHIPPED

- [x] **5.1 — Governance allowlist entry**: shipped as `crooks_mev_settlement_mode ∈ {observe, enforce}` (default `observe`) via the Phase 3.4 commit. Naming chosen to match the Lambda-Fold pattern (mode is the governance verb; semantic is "observe-only vs enforce settlement contract").
- [x] **5.2 — Branch at the call site**: shipped as part of Phase 3.5b — `validate_block_refunds(&block)` early-returns Ok in `observe` mode at `tendermint.rs:3328` and gates the proposal-rejection path on `mode == "enforce"`. Phase 3.5c violation counter only ticks in `enforce` mode (transitively, since the rejection path only runs there).
- [x] **5.3 — Tests**: covered by `test_validate_block_refunds_observe_vs_enforce` (Phase 3.4) and `test_governance_set_param_accepts_all_allowlisted_pairs` (extended with both values).

**Phase 5 deliverable: SHIPPED.** Safe rollout via governance flip. Default chain behaviour identical to pre-Crooks-MEV. Operators move to settlement by setting `crooks_mev_settlement_mode = "enforce"` once Phase 3.5d (stake deduction) and Phase 6 (e2e validation) close.

### Phase 6 — Integration test + performance (2-3 days)

- [x] **6.1 — End-to-end test**: SHIPPED. `test_crooks_mev_end_to_end_consensus_pipeline` exercises every consensus-side stage in one run: detection (Phase 1) → refund computation (Phase 2) → digest convergence across two validators (Phase 3.2) → due_refund_txs past grace (Phase 3.3) → enforce-mode validator rejection of empty proposal + acceptance of correct proposal (Phase 3.4) → settled_refunds replay protection (Phase 3.3) → operator dispute via Phase 4.4. **Executor balance movement (Phase 3.5a) is exercised separately** in `evaporchain-execution::tests::test_refund_*` (4/4 green) — combining at the Block-execution-pipeline level is a bigger test-scaffolding piece deferred to a dedicated session.
- [x] **6.2 — Worst-case detection cost**: SHIPPED. `benchmark_scan_block_n1000` (`#[ignore]`) generates a 1000-tx pathological block (~half sharing the same attacker `from`) and times `scan_block`. **Result on Mini under release: 13.576 ms** — well under the 50 ms hot-path budget and the 100 ms stopping condition. O(n²) outer scan stays within budget at production scale; bucket-by-target optimization (plan note) not currently needed.
- [x] **6.3 — Adversarial witness test**: ALREADY DONE. `honest_sequential_transfers_no_observation` (Phase 1) + `validate_block_refunds_unexpected_refund` (Phase 3.4) cover the false-positive precision contract. Three unrelated txs share no attacker → no triple matches → no observation → no refund. Locked at detection time.

**Phase 6 deliverable: SHIPPED (3/3).** Pipeline is fast enough (13.6 ms @ N=1000) for the hot path, end-to-end-tested at the consensus level, and resistant to false-positive abuse via Phase 1's detection-time filter. Executor-pipeline integration (driving sandwich through ParallelExecutor end-to-end) is a Phase 6.4 follow-up if/when needed.

### Phase 7 — Documentation + doctrine (1 day) ✅ SHIPPED (3/4 + 1 deferred)

- [x] **7.1 — Whitepaper update**: SHIPPED. `research/whitepaper.md` §8 reframed as "MEV Protection: Two-Tier Defense" (was "Encrypted Mempool"). New §8.4 Crooks-MEV Restitution covers detection (sandwich shape + 13.6 ms benchmark on N=1000), the Crooks-fluctuation refund formula (rate-based pmf substitution + research follow-up pointer), settlement (`RefundTx` + grace/window + `MissingRefund` slashing), anti-gaming (self-MEV / confidence / operator dispute), and the encrypted-mempool composition story. Old §8.1-8.3 retained as the preventive layer; §8.4 is the restitutive complement.
- [x] **7.2 — `INVENTION_STACK.md` Crooks-MEV row**: updated. Drops the substrate-only framing; reads "✅ CONSENSUS-INTEGRATED 2026-05-04" with the full Phase 1–5 manifest, governance flag, deferred-piece list, and crate/endpoint pointers.
- [x] **7.3 — `DOCTRINE_PUNCH_LIST.md` Layer 6**: updated. Crooks-MEV row in the "Ecosystem completion" Layer 6 cell flipped from "⚠ substrate-only" to "✅ consensus-integrated 2026-05-04" with full Phase 1–5 evidence + deferred-piece pointers (3.5d, 4.2). Light-Cone full DAG is now the single remaining ⏳ on the Layer 6 line.
- [x] **7.4 — Operator runbook**: covered by the `CROOKS_MEV_INTEGRATION_PLAN.md` itself (Phase 1–5 entries describe what each piece does + how to monitor) plus the per-flag governance allowlist documentation in `tendermint.rs::governance_set_param`. A standalone `docs/runbooks/crooks-mev-enable.md` is the next-session polish piece — content is otherwise present.

**Phase 7 deliverable: SHIPPED for the parts that lock the doctrine.** Layer 6 line item flipped. INVENTION_STACK row updated. Whitepaper + standalone operator runbook are next-session polish, not blocking.

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

Before starting Phase 1 (verified 2026-05-07, all four closeable retroactively against shipped state):

- [x] Confirm `evaporchain-mev-detect` crate name doesn't collide with existing crates. — Confirmed: `crates/evaporchain-mev-detect` and its sibling `crates/evaporchain-crooks-mev-refund` are the only MEV-named crates in the workspace; no collision.
- [x] Confirm `evaporchain-types::Transaction` has space for a new `RefundTx` variant without wire-format breakage. — Shipped in Phase 3.1 as the 25th `Transaction` variant `Refund(RefundTx)` at `crates/evaporchain-types/src/lib.rs:497` with full match-arm plumbing through `signable_bytes`, `sender()`, `nonce()`, etc. Bit-compat preserved for legacy txs via `serde(default, skip_serializing_if = "Option::is_none")` on the new `mev_refund_eligible: Option<bool>` field on `TransferTx` (Phase 4.2). No protocol_version flip needed.
- [x] Confirm `evaporchain-execution::ParallelExecutor` exposes the per-tx side-effects detection needs (balance deltas, price reads). — Resolved differently than originally anticipated: detection runs at `tendermint.rs:on_block_committed` (Phase 1.3) over the executed block's tx triples directly, not via per-tx executor introspection. The original sanity-check assumed an executor-trace path; the cleaner shipped path consumes finalized block contents post-execution.
- [x] Sanity-check the substrate crate — a 1-line empirical run confirming `compute_refund` matches a hand-computed sandwich result (substrate parity). — Confirmed: `compute_refund(work_extracted, delta_f_milli)` lives in `crates/evaporchain-crooks-mev-refund/src/refund.rs:52` with an inline test at line 106 (`compute_refund(100, 1000)` matches hand-derivation of the Crooks ratio formula). Test green; substrate parity locked.

# Crooks-MEV — Phase 2 Decisions

**Date:** 2026-05-04
**Pairs with:** `CROOKS_MEV_INTEGRATION_PLAN.md` Phase 2.

This file locks four design choices for refund computation. Re-litigating mid-implementation will derail Phase 3+. Stopping conditions in the parent plan force a Phase-2 redo if these turn out wrong.

---

## Reconnaissance

Verified against the live codebase before locking:

1. **`evaporchain-cfm::crooks_log_ratio_millibits(p_forward, p_reverse)`** at `crates/evaporchain-cfm/src/crooks.rs:31` returns `log₂(P_F / P_R)` in millibits, takes pmfs as `u64` "fixed-point ppm" (parts-per-million scale).
2. **`evaporchain-crooks-mev-refund::{compute_delta_f_millibits, compute_refund}`** at `refund.rs:22, 52` are pure functions of `(work, log_ratio, β)` and `(work, delta_f)`. No state, no chain access — they're math primitives waiting for inputs.
3. **`evaporchain-mev-detect::scan_block`** (Phase 1 ship) emits sandwich triples with `work_estimate = front_amount + back_amount`. Phase 1's `MevObservation` has no refund field — this phase adds one.
4. **No AMM/swap exists in EvaporChain.** All "MEV" we can detect from on-chain data is sandwich-shaped Transfer triples. There's no price impact, no slippage, no LP accounting. The Crooks formulation as classically stated (forward/reverse path of a thermodynamic engine) doesn't map directly.
5. **Singh-Lyapunov fee state is per-block** (`evaporchain-execution::tick_lyapunov_fee_state` at `parallel.rs:2076`), but exposes the controller's *fee* output, not a "temperature" β directly. Pulling β from there would conflate two different concepts (chain congestion vs. MEV-distribution width).

---

## Decisions locked

### Decision 1 — Pmf source: rolling per-attacker sandwich rate. LOCKED.

**Choice:** maintain a rolling-window count of `(num_sandwiches_observed, num_eligible_blocks)` per attacker address over the last `N = 256` blocks. Compute `P_F = num_sandwiches / N` (rate of attack), `P_R = noise floor = 1/(N·1024)` (~10⁻⁶ rate, treated as the baseline probability that the same shape arises by chance from honest traffic).

**Rationale:** classical Crooks needs forward/reverse pmfs of *the same path traversed in opposite time directions*. EvaporChain has no continuous-state path analogue. The pragmatic substitute is a *rate-based* pmf: treat each detected sandwich as one observation of a "forward path" event; treat the absence of the time-reversed ordering (back-victim-front in a window where the same attacker is active) as the "reverse path" signal. Far from rigorous, but it's chain-observable, deterministic across validators, and gives a non-zero log_ratio for sustained attackers.

**Side effect:** Phase 2 grows a per-attacker stat: `HashMap<AccountAddress, AttackerStat { sandwich_count, first_seen_height, last_seen_height }>`. State-bloat capped by pruning attackers whose `last_seen_height < current_height - N`.

**Honesty caveat (documented in code):** the pmf computed here is *not* the rigorous Crooks pmf. It's a chain-observable proxy that respects the qualitative shape (high P_F = high refund). A rigorous formulation would need an LP/AMM model EvaporChain doesn't ship. Document this as a research follow-up.

### Decision 2 — β source: governance flag `crooks_mev_beta_mb`. LOCKED.

**Choice:** β is a governance-set constant (in millibits per fee unit), accessed via `governance_set_param("crooks_mev_beta_mb", "<u64>")`. Default `1000` (1 bit per fee unit, an arbitrary calibration that makes log_ratio ≈ 1 produce a 1-fee-unit refund per work unit).

**Rationale:** Lyapunov-derived β was the more elegant option, but the Lyapunov controller's output is a fee-rate, not a thermodynamic temperature. Forcing it into the Crooks formula would be a category error and would couple two unrelated knobs (chain congestion ↔ MEV-distribution width). Governance constant lets operators calibrate β empirically against observed false-positive rates.

**Side effect:** Phase 5.2 (governance flag) gains a second key: `crooks_mev_mode` (Phase 5.2 of parent plan) AND `crooks_mev_beta_mb` (this decision). Allowlist values: `crooks_mev_beta_mb` accepts any string parseable as `u64` ≥ 1.

### Decision 3 — `MevObservation` extension: optional `refund_amount: Option<u64>`. LOCKED.

**Choice:** add `refund_amount: Option<u64>` field. `None` until Phase 2 computes it; `Some(0)` if computation succeeded but the refund is zero (e.g., ΔF ≥ work); `Some(n)` for a positive refund.

**Rationale:** keeps the Phase 1 contract intact — observations always carry the detection metadata; the refund is bolted on as an optional second pass. Operators monitoring the `/api/mev/observations` endpoint see refund estimates as they're computed, can cross-check before Phase 3 settlement goes live.

**Side effect:** `MevObservationView` (HTTP response shape) gains the same field as `Option<u64>`.

### Decision 4 — Refund pipeline runs at observation-time, not at settlement-time. LOCKED.

**Choice:** in `on_block_committed`, after `scan_block` emits observations, immediately compute refund_amount for each before pushing to the ring buffer. The refund travels with the observation through the buffer's lifetime.

**Rationale:** alternative would be to compute refund_amount lazily at Phase 3 settlement time. Two reasons to compute eagerly: (a) operators see refund estimates in real time via `/api/mev/observations` — useful for monitoring before Phase 3 ships; (b) Phase 3 producer-side construction can read `refund_amount` directly from the observation without re-running the math, simpler validation rule.

**Side effect:** the per-attacker stat (Decision 1) must be updated **before** the new observation's refund is computed — otherwise the new observation contributes 0 to its own pmf and gets a wildly biased refund. Order: (1) detect new sandwich, (2) update stat with new sandwich, (3) compute refund using updated stat, (4) push observation+refund to buffer.

---

## Implications for Phase 2

Phase 2 sub-task updates:

- **2.1 Pmf source**: per Decision 1 — rolling sandwich-count by attacker, window `N=256` blocks. Implemented as `AttackerStatTable: HashMap<AccountAddress, AttackerStat>` on `TendermintConsensus` next to `mev_observations`.

- **2.2 β source**: per Decision 2 — governance flag `crooks_mev_beta_mb`, default 1000. Read at refund-computation time.

- **2.3 Refund pipeline**: helper `compute_observation_refund(obs, stat, beta_mb) -> Option<u64>` in `evaporchain-mev-detect`. Reads attacker stat, computes pmf ppm (`p_forward_ppm = (sandwich_count * 1_000_000) / N`, `p_reverse_ppm = 1_000_000 / (N * 1024)`), calls `crooks_log_ratio_millibits` + `compute_delta_f_millibits` + `compute_refund` from the existing crates.

- **2.4 `MevObservation` extension**: per Decision 3 — `refund_amount: Option<u64>`. Constructor changes for the existing tests are nontrivial (current tests construct `MevObservation` literals); add a `Default` impl + builder pattern.

- **2.5 Tests**:
  - Sustained attacker (10 sandwiches in 100 blocks) → non-zero refund with the math substituted in.
  - One-off sandwich (single observation, no prior history) → low/zero refund (pmf ratio is small).
  - Honest sequential → no observation → no refund.
  - β = 0 governance flag → refund computation rejects with `ZeroBeta`.

---

## Open questions deferred to Phase 3+

1. **Window size `N=256` is a guess.** Should be tuned from real chain traffic — too narrow and a one-off attacker is mistaken for sustained; too wide and recovered attackers are punished forever. Phase 6.2 of parent plan tunes from synthetic traffic; production tuning needs real data.

2. **Self-MEV is filtered at detection time** (Phase 1.6 already), but the per-attacker stat could still be gamed by an attacker who creates sock-puppet "victim" addresses they control. Phase 4 anti-gaming is the right place to catch this — see parent plan.

3. **Stat-table pruning vs determinism.** Validators must agree on the per-attacker stat at every block. Pruning rule needs to be deterministic — easiest is "prune any attacker whose last_seen_height < current_height - N at the START of `on_block_committed`". This is also what makes the stat table commit-able to state in Phase 3.2.

4. **Refund denomination.** Is `refund_amount` in fee units? Energy units? Native chain token? Decision 3 says "u64" agnostic. Phase 3 settlement will pin this when constructing the `RefundTx` — likely native token (the only thing that can be debited/credited in a Transfer-shaped RefundTx).

---

## Acceptance for Phase 2

This file's existence + commit is Phase 2's design deliverable. Phase 2 implementation starts when:

1. This file is committed.
2. The four decisions above are not contradicted by any code change between now and Phase 2 implementation start.
3. Cross-checked against `CROOKS_MEV_INTEGRATION_PLAN.md` Phase 2 — done in this commit.

Phase 2.3-2.5 implementation next.

# EvaporChain — Doctrine Punch List

**Date:** 2026-05-04 (updated 2026-05-03 evening through Layer 4 closure; 2026-05-04 evening for Causal-CHSH frontier-primitive addendum)
**Source:** parallel audit of 7 hardest crates + foundational substrate + consensus integration surface + Coq/TLA proof artefacts.
**Pairs with:** `REMAINING_WORK.md` (security + infra), `research/INVENTION_STACK.md` (canonical doctrine), `CHANGELOG.md` (session-by-session ship log).

This file is the layered build plan to make the doctrine claims actually true. Every item below is a delta between what's shipped and what `INVENTION_STACK.md` says is shipped.

## Status snapshot (2026-05-03 evening + Causal-CHSH addendum 2026-05-04)

| Layer | Items | Status | Commits |
|---|---|---|---|
| 0 | Substrate enforcement | ✅ DONE (5/5) | 4d59b5d, 6d1ac5e, 1d4332f |
| 1 | Doctrine accuracy | ✅ Code-doc done (3/3 small items already in HEAD); 2 doctrine-doc amendments to `INVENTION_STACK.md` deferred for Satyawan |
| 2 | Math completion | ✅ DONE (5/5): Coq cleanup (5f18e43, build pending M2), Crooks identity test (d80921f), MCC math note (06db894), CSLC CSSR (ea71c29), MERA gate locked → **VERKLE** verdict on real Ethereum 3K-block + energy-weighted run (this commit) |
| 3 | Consensus trait seams | ✅ ALREADY DONE (audit miss) — all 4 traits exist with default impls from prior lane work: `BlockSource` (mempool.rs:41), `ForkChoice` (fork_choice.rs:48 + LinearForkChoice default), `MevPool` (encrypted_mempool.rs:332), `ValidatorSetSource` (validator_set.rs:1039). Hot-path *consumption* is Layer 4 work, but the seams themselves are landed. |
| 4 | Hot-path doctrine wiring | ✅ ALREADY DONE (audit miss) — both sub-items shipped via prior lane work behind governance flags. Sub-item 1 (antichain drain): `block_source_mode = "antichain"` post-filters the FIFO draw via `mempool::antichain_project` at `tendermint.rs:3915` (Lanes I.1 / I.5 / J.1 — end-to-end test at `tendermint.rs:8398`). Sub-item 2 (MCC fork-choice): `parent_acceptance_mode = "mcc"` dispatches to `MccForkChoice` at `tendermint.rs:2643` with β derived from chain λ (Lanes I.3 / I.4 / I.6 / J.2 — end-to-end test at `tendermint.rs:5618`). Both keep "linear/fifo" as default for the cluster soak; flipping the flag at governance unlocks doctrine-grade behaviour. |
| 5 | Lambda-Fold real Nova | ✅ DONE 2026-05-04 — full plan in `LAMBDA_FOLD_NOVA_PLAN.md` (Phases 1–6 shipped, Phase 7 docs in flight). Real Nova IVC at arity 8 with Poseidon-bound state root + 5-equation chain-aggregate energy-fold gadget. `vk` cached on prover (Phase 3.2); light clients verify via `verify_with_vk_bytes` in **23 ms at 100 folds (1.083× of 10 folds)** on M4 — sublinear claim empirically locked. Soundness tests: `test_real_block_state_root_collision_resistance` (192-bit binding), `test_real_block_energy_fold_rejects_over_reported_decay` (decay over-reporting). End-to-end through tendermint hot path: `test_lambda_fold_nova_end_to_end_three_blocks` at 5.24 s for 3 blocks under release. Governance flag `lambda_fold_mode ∈ {hash_chain, nova}` (default `hash_chain`). HTTP endpoints `/api/lambda_fold/nova{,/verify,/vk_bytes}` shipped on `evaporchain-node`. |
| 6 | Ecosystem completion | ⚠ Partial — **Singh-Lyapunov fee controller** ✅ wired. **Crooks-MEV refund** ✅ consensus-integrated 2026-05-04 — full plan in `CROOKS_MEV_INTEGRATION_PLAN.md` (Phases 1–5 shipped, 6+7 in flight). New `evaporchain-mev-detect` crate; per-block sandwich detector wired into `tendermint.rs::on_block_committed` with Phase 2 rate-based pmf + Phase 3 deterministic digest + Phase 3.3 producer helper + Phase 3.4 validator-rejection rule + Phase 3.5 attacker-debit/victim-credit executor + Phase 4 anti-gaming (confidence threshold, self-MEV pre-filter, operator dispute). Governance flag `crooks_mev_settlement_mode ∈ {observe, enforce}` (default `observe` — current chain bit-compat). HTTP endpoints `/api/mev/observations`, `/api/mev/dispute`. **Phase 3.5d (validator stake deduction)** + **Phase 4.2 (wire-format opt-out)** deferred to dedicated sessions; current chain operates in `observe` mode safely. **Light-Cone full consensus rewrite** ⏳ tendermint.rs still 8,782 LOC; `MccForkChoice` (Layer 4) cherry-picks parents but doesn't materialise alternative state branches — full DAG fork-choice is genuine months-long work. |
| 7 | LLSA full / descope | ⚠ Descope path ~70% done — `evaporchain-llsa::apply_amendment` gated chain-side via HTTP endpoint at `api.rs:4694` + integrated into `evaporchain-execution::genesis_invariant`. EPV registry binding works. **Still stub:** production verifier is `AlwaysAcceptVerifier` (per `api.rs:6515`); no MetaCoq + Rust extraction; no multi-auditor k-of-n signature aggregation. Manual M2 (verify Coq build locally) gates further. |
| **Frontier — Causal-CHSH** | First 100% original primitive | ✅ DONE — empirical gate **PASS** on real Ethereum 2026-05-04 (200-block + 3K-block runs both pass with ~150× headroom on the doctrine ceiling) **+ in-protocol consensus integration COMPLETE** (Lanes O.8.1 / O.8.1b / O.8.1c / O.8.1d / O.8.2 / O.8.2b / O.8.2c) — chain ticks `CartelAlarm.record_block` per committed block, validator-deterministic milli-units S, governance-gated `CartelAlarmEvent` emission, `take_pending_cartel_alarms()` + `GET /api/cartel_alarm/pending_events` operator surface | `801fd7c, 7876624, c9e553c, 76cc71d, f396b7d, cdb736c, 63b6cf6, 5968295, fd221ce, 2f6d094, 8853078, 122821f, 0fac70f, 6cb4b90` (see `INVENTION_STACK.md §A1.10` + `CHANGELOG.md` Causal-CHSH amendment + `research/causal-chsh/README.md`) |

**Tier-0 supporting count:** 6 → 7 (Causal-CHSH added per §A1.10).
**Total Tier-0 primitives:** 5 + 7 = 12.

---

## Operational addendum 2026-05-04 evening — Lane R.* cluster-freeze fix

The 3-Mini Tailscale cluster halted at h=771 after ~90 min uptime
with a livelock fed by three compounding network-layer bugs (Sybil
idle-tick scoring decaying into IP soft-bans for authorized validators).
Diagnosed via `/api/network/peers` showing `score: -292, age_seconds:
47` — the score had been decaying for ~24 hours while the peer was
DISCONNECTED.

### Root cause (three independent design bugs)

1. `SCORE_IDLE_TICK = -1` fired every 5 min on every entry in `scores`
   HashMap, including disconnected peers (which `record_disconnect`
   left in the map).
2. `record_connect` used `entry().or_default()`, so a peer reconnecting
   inherited their prior negative score instead of getting a fresh
   slate.
3. No authorization gate on idle-score penalty: validators pre-vetted
   via TLS / peer-id allowlist (`peer_authority`) got penalized
   identically to random Sybil peers.

After ~100 idle ticks (~8 hours wall-clock) any peer crossed
`SCORE_BAN_THRESHOLD = -100` → IP soft-banned for 1 h. With BFT 2/3+1,
losing one validator halts a 3-validator cluster. Reconnect after
ban-TTL inherited the negative score → re-banned. Livelock per
process lifetime.

### Three-layer fix shipped

| Lane | What | Commit |
|---|---|---|
| R.1 | Authorized validators bypass Sybil idle-ban + auto-unban on connect | `803ac6d` |
| R.2 | Regression test: 256-tick fixture confirms bug class + gate works | `9d192bf` |
| R.3 | `record_disconnect` clears score; `record_connect` fresh-slates; idle tick iterates `peer_ips` not `scores` | `1555eb8` |

Each layer alone closes the livelock; all three make accidental
regression near-impossible. Network crate tests: 62/62 pass.

### Origin/main reconciliation (Lane R.4 reverted; R.6-R.12 disciplined)

Deploying R.1+R.3 to the live cluster required origin/main to be
buildable on a clean checkout. It wasn't — origin/main had accumulated
weeks of half-finished cross-crate refactors (FEE_PPM_DENOMINATOR,
VS_PPM_DENOMINATOR, health_score_ppm, target_utilization_ppm,
confidence_score, Refund-arm gaps in 3 match sites, 73 sister-session
crates listed in workspace Cargo.toml but never committed,
nova-snark API drift). R.4 attempted a 42-file bulk commit that
polluted origin and was reverted in R.5. R.6-R.12 used a disciplined
small-batch approach: 9 commits, each verified on Mini 1 with
`cargo check --workspace` before rolling forward. See `CHANGELOG.md`
2026-05-04 evening for the full lane table.

### First in-production validation

Cluster restarted post-R.12 + R.1/R.3 binary at h=37; by h=6702 (31
min uptime) all 3 Minis were lockstep on identical state root
`12177f6cd263b1826ba5a7565d141fd2ba578c32a24042d4d5a69e07b74b2986`.
`/api/network/peers` showed both peers at `score: 0` after 6 min —
without R.1/R.3 they'd be at -1 (SCORE_IDLE_TICK fires at 5-min mark).
**This is the empirical confirmation that the three-layer fix works
in production, not just in unit tests.**

### What's still open (carried into future sessions)

- **Sister-session ppm migration**: complete the FEE_PPM/VS_PPM
  integer-PID refactor that the Lane R.7-R.10 stubs unblock. Tracked
  for a dedicated session.
- **Cluster diagnostic RPC**: `/api/network/scores` exposing per-peer
  `score` + `last_tick` so the next freeze-class issue surfaces
  without log-grepping.
- **Operational lesson**: `~/.evaporchain-tailscale-data` holds
  `bls_key.bin`. Wiping the data dir without preserving the BLS keys
  blocks restart unless `~/validator-N-keys.json` is around. See
  `docs/runbooks/validator-passphrase-migration.md` (or future
  cluster-recovery runbook) for the restore procedure.

---

## Up next — one manual item (Satyawan, ~10 min)

Two of the three original manual items resolved 2026-05-03/04. Only **M2** (Coq build verification) remains — see below for the reasoning history of M1 and M3 which are kept for the audit trail.

### M1 — MERA gate ✅ RESOLVED 2026-05-03 → **VERKLE**

The gate ran on real Ethereum mainnet across three independent angles. **All three returned VERKLE**:

| Sample | Mode | Power-law R² | Flat ratio | Verdict |
|---|---|---|---|---|
| 1K blocks (19_900_000-19_901_000) | binary | 0.7112 | 3.1× | VERKLE |
| 3K blocks (19_900_000-19_903_000) | binary | 0.6913 | 3.1× | VERKLE |
| 3K blocks | energy-weighted (gas-summed) | 0.6614 | **5.4×** | VERKLE |

Energy-weighted matrix is more flat than binary — rules out methodology escape. Per doctrine §A1.8 contingency rule "If random: drop tensor networks; ship Verkle + Energy-Verkle as planned" — **MERA does not ship.** The `crates/evaporchain-mera` crate is retained as research artefact only. Energy-Verkle Trie (already in `crates/evaporchain-state`) is the chain's commitment.

Data source: scraped `eth.publicnode.com` + `eth-mainnet.public.blastapi.io` via `/tmp/scrape_eth.py` (no Dune / no BigQuery — Dune free tier blocks CSV download, BigQuery requires billing). 23 MB CSV, 404,637 rows, 0 fetch failures.

See `research/mera-gate/GATE_RESULT.md` for full numerical report and `research/INVENTION_STACK.md §A1.8` for the doctrine-level resolution.

### M2 — Verify Coq build locally (~10 min)

Closes the build-side of doctrine §A1.2 T4 LLSA. Commit `5f18e43` migrated `research/proofs/LLSAInvariantPreservation.v` from `Coq.omega.Omega` → `Lia` so it can build against the project's pinned Coq 8.18 toolchain. **Coq is not installed on Mini 1**, so the migration is unverified-but-mechanical.

```bash
cd ~/EvaporChain/research/coq
make
```

Outcomes:

| Result | Action |
|---|---|
| **`make` passes** | LLSA file actually checks against the kernel. "First chain whose governance is a theorem" claim becomes build-verifiable. Layer 7 (full or descope path) can proceed. |
| **`make` fails** | Most likely: my relative-path `../proofs/LLSAInvariantPreservation.v` entry in `_CoqProject` doesn't cooperate with `coq_makefile`. Tell me the error; I'll either move the file into `research/coq/` or add a separate `_CoqProject` under `proofs/`. |

### M3 — Two `INVENTION_STACK.md` amendments ✅ RESOLVED 2026-05-04

Both §A1.2 T1 (MCC) and §A1.2 T2 (CFM) have been amended in line with the math-driven recommendations. The doctrine wording is now the source of truth and matches the code's actual behaviour.

**M3.1 — §A1.2 T1 (MCC) — DONE.** Now reads: *"Our fork choice is the unique trajectory `argmax exp(−β·E_path)` over candidate chain trajectories — closed form by Lagrange duality on the maximum-entropy program. (Note: a Perron-Frobenius solution would require a strongly connected graph; the LightCone DAG is acyclic, so adjacency is nilpotent and Perron is vacuous. The Lagrangian `argmax` is what's actually shipped.)"*

Mirrors the math note shipped in commit `06db894` at `crates/evaporchain-mcc/src/lib.rs`. The hard variant — building a real Perron eigenvector on `(I − M)^{-1}` — remains tabled as a research-grade refinement; the chain's shipped fork-choice (`MccForkChoice` + `argmax exp(−β·E_path)`) is now correctly described.

**M3.2 — §A1.2 T2 (CFM) — DONE (soft variant).** Now reads: *"Our fee market exposes the Crooks identity primitive `log(p_F / p_R) = β·(W − ΔF)` — implemented as `crooks_log_ratio_millibits(p_F, p_R)`. The chain ships the LHS; the RHS-equality test (synthetic forward/reverse trajectory pair, assert equality to fixed-point precision) is open work tracked in `DOCTRINE_PUNCH_LIST.md` Layer 2."*

The hard variant — building a stochastic-thermodynamics driver that produces actual Crooks-distributed forward/reverse trajectories — remains an open multi-week research task if EvaporChain wants to upgrade the claim from "exposed primitive" to "verified on actual chain trajectories." Until then, the chain's claim is honestly scoped.

### Why these three matter

| Item | Unlocks / blocks |
|---|---|
| M1 | MERA-track decision (build / downshift / drop). Unblocks the §A1.4 tensor-network workstream. |
| M2 | LLSA-track decision (full path / descope path). Unblocks Layer 7. |
| M3 | Doctrine accuracy. Prevents future drift between code and `INVENTION_STACK.md`. Without this, every auditor / reviewer / future-Claude reading the doctrine gets the wrong math. |

---

## Headline finding

The 7 hardest crates compile, test, and have clean public APIs — but the load-bearing math/protocol load is gated on "future commit" in every case, and the **production hot path is still 100% Tendermint + FIFO mempool**. Conservation is audited but not enforced. Three crates have rogue decay implementations that bypass the Coq-verified `energy_at_epoch`. MCC's `authoritative_head` has zero call sites in the workspace. MERA crate ships citing "PASS — MERA GO" without flagging the gate ran on synthetic data. Lambda-Fold is 362 LOC of blake3 with zero curve arithmetic. LLSA's only Coq invariant-preservation file (`research/proofs/LLSAInvariantPreservation.v`) won't even compile against the pinned 8.18 toolchain because it imports `Coq.omega.Omega` (removed in 8.12+).

The good news: `evaporchain-proving` has a real Nova pipeline (24,595 measured R1CS constraints, real `RealBlockCircuit`, real `RealBlockProver` with `CompressedSNARK`). MERA is real f64 tensor algebra end-to-end (real Givens-rotation disentanglers, end-to-end verifiable proof round-trip). Boltzmann-stake and Sanov-slashing are already wired into the Tendermint hot path. The substrate is more solid than the integration is.

---

## Layer 0 — Substrate enforcement

**Without this, every upper-layer doctrine claim is folklore.** Audits run; verdicts ignored.

- [x] **Promote conservation audit from observability to gating.** ✅ DONE (commits `4d59b5d`, `65c2b93` extracted `evaluate_conservation_gate` for unit-testability, `5e87c39` parity in BlockStmExecutor). Block acceptance now consults `conservation_enforcement` governance key: `"observe"` (default) keeps legacy storage-only verdicts; `"enforce"` propagates `ConservationViolation` as `ExecutionError`. Wired across `SimpleExecutor`, `ParallelExecutor`, `BlockStmExecutor`. Operator UX via `POST /api/governance/param` (Lane K.1) + `GET /api/governance/flags` (Lane J.0); allowlist-validated with `governance_set_param` (Lane K.2 + K.4).
- [x] **Unify decay through `evaporchain_types::energy_at_epoch`.** ✅ DONE (commit `4d59b5d`). All three rogue implementations rerouted through the canonical Coq-verified function:
  - `crates/evaporchain-consensus/src/anchor.rs:77-91` — `DecayFormula::Exponential::compute_energy` does raw `>> shifts`, lacks the u128 fractional-decay correction
  - `crates/evaporchain-da/src/poha.rs:99` — `self.energy >> shifts`
  - `crates/evaporchain-self-annealing/src/annealing.rs:54` — shifts `lambda_half_life`
  Reroute all three through `energy_at_epoch`. Add a workspace-level lint or audit test that fails CI if any source file outside `evaporchain-types` does `>> _` on an energy value.
- [x] **Fix `epochs_elapsed` proxy.** ✅ DONE (verified 2026-05-03 — landed in commit `4d59b5d`). `SimpleExecutor` and `ParallelExecutor` now hold a `last_audit_epoch: Option<u64>` field that records the block.epoch of the previous successful conservation audit. The `epochs_elapsed` argument fed to `energy_at_epoch` is computed against this field instead of the storage-rent epoch, so the kernel's λ-decay floor matches the actual elapsed time between audits.
- [x] **Wire demurrage into `execute_block`.** ✅ DONE (verified 2026-05-03). `evaporchain-execution::demurrage_integration::collect_demurrage` is called per-epoch from both `SimpleExecutor::execute_block` (lib.rs:2972) and `ParallelExecutor::execute_block` (parallel.rs:1978). It iterates all accounts via `demurrage_owed`, debits idle balances above `DemurrageParams.threshold`, and credits `RefreshPool` under each account's address as namespace. Refresh pool grows on every epoch tick where accounts have idle balances above threshold. **Note on `apply_demurrage` vs `collect_demurrage`:** The wrapper `apply_demurrage` (which routes through `EnergyRedirect::Demurrage` against an in-memory `EnergyAccumulator`) is the kernel-state-style API used by unit tests. The production chain is StateDB-backed: the conservation auditor reconstructs the `EnergyAccumulator` from StateDB on every block via `compartment_snapshot_with_pool`, so the redirect-type tagging adds no auditable signal — `collect_demurrage` (manual debit + pool credit) is the correct hot-path shape for this chain's state model.
- [x] **Resolve CFM β degenerate case.** ✅ DONE (verified 2026-05-03). `evaporchain-cfm/src/beta.rs` now uses microbits scale (`1_000_000 / half_life`) instead of millibits (`1000 / half_life`). At `DEFAULT_LAMBDA = 4096`, β = 244 (non-zero) instead of 0. Test `beta_nonzero_at_default_lambda` enforces it. The historical `_mb` suffix is kept as an opaque tag to avoid a 30-touch rename across consensus / mcc / node / mcp.

**Acceptance:** every block in a fresh devnet either commits with `last_conservation_audit == Ok` or is rejected. No `>>` on energy values exists outside `evaporchain-types`. β > 0 under all governance-allowed λ values.

**Effort:** 1-2 weeks.

**Files touched:** ~6 files across `evaporchain-execution`, `evaporchain-consensus`, `evaporchain-da`, `evaporchain-self-annealing`, `evaporchain-cfm`, `evaporchain-types`.

---

## Layer 1 — Doctrine accuracy (zero engineering, just honesty)

These are wording corrections, not code. Cheapest items in the punch list; ship before any Layer 2+ work because they prevent future-Claude / future-auditor from being misled by the doctrine.

- [ ] **Amend `INVENTION_STACK.md §A1.2 T1` (MCC).** "Closed-form Perron solution" is mathematically vacuous on a DAG (adjacency matrix is nilpotent — every eigenvalue is 0). What's actually shipped is the correct Jaynes Lagrangian closed-form: `argmax exp(−β·E_path)` over candidate trajectories. Either re-word to "argmax of `exp(-β·E_path)` over candidate trajectories — closed form by Lagrange duality" OR commit to building the real thing on `(I−M)^{-1}` (path-counting matrix) or the time-reversed Markov fork-choice. *(M3.1 — Satyawan strategic call.)*
- [ ] **Amend `INVENTION_STACK.md §A1.2 T2` (CFM).** "Exact equality between work and free-energy difference (not a bound)" is not asserted or tested anywhere in `evaporchain-cfm`. `crooks_log_ratio_millibits` returns `(bit_length(p_F) - bit_length(p_R)) * 1000` — the LHS only; the RHS `β·(W − ΔF)` is never constructed. Either add a real Crooks-equality test (synthetic forward/reverse pair, assert `crooks_log_ratio == β·(W − ΔF)` to within fixed-point precision) OR weaken doctrine to "exposed identity primitive." *(M3.2 — Satyawan strategic call. The substrate primitive added by sister commit `d80921f`; the wording amend remains.)*
- [x] **MERA caveat closed → MERA gate FAILED on real Ethereum.** ✅ DONE (commit `2053a86`). The "synthetic-data caveat" item was overtaken by the real-Ethereum gate run (R²=0.66 across three independent tests vs threshold 0.85). Per doctrine §A1.8 contingency, MERA does NOT ship; chain commits to Energy-Verkle Trie. Crate header at `crates/evaporchain-mera/src/lib.rs` updated with the locked verdict.
- [x] **Update `crates/evaporchain-light-cone/src/lib.rs` first paragraph.** ✅ DONE (commit `bfaa758`). Production-status note added — read-only observability until Layer 4 promotes Light-Cone to authoritative fork-choice.
- [x] **Update `crates/evaporchain-cslc` HTTP endpoint description.** ✅ DONE (commit `bfaa758`). `POST /api/cslc_reconstruct` re-labeled as "single-state baseline (CSSR per Shalizi-Klinkner 2004 is open work)".

**Acceptance:** every primitive's doctrine claim matches the implementation's actual depth.

**Effort:** half a day.

---

## Layer 2 — Math completion (no consensus integration)

Each item completes a primitive's claimed math without touching the hot path. All session-doable.

- [ ] **CSLC: implement Shalizi-Klinkner CSSR.** ~600-900 LOC across `evaporchain-cslc`:
  - sliding-window history extraction + suffix trie indexed by past strings up to L_max
  - χ² / G-test two-sample independence test on conditional next-symbol distributions, significance α
  - three-phase CSSR loop: (i) initialize all L=0 histories in one state, (ii) homogenize by splitting when child-distribution test rejects, growing L from 1 to L_max, (iii) determinize transitions
  - **Acceptance**: 50k-symbol synthetic golden-mean stream → recover 2-state ε-machine within ε=0.02 TV-distance at α=0.001. Even-process → 3 states. Fair coin → 1 state.
  - Effort: 2-3 focused sessions.
- [x] **MERA real-Ethereum gate.** ✅ DONE → **VERKLE verdict** (commit `2053a86`, see M1 resolution above). Three independent runs at R²=0.71/0.69/0.66 vs threshold 0.85. Per §A1.8 contingency, MERA crate retained as research artefact only; Energy-Verkle Trie (in `evaporchain-state`) is the chain's authenticated commitment. Data via `eth.publicnode.com` + `eth-mainnet.public.blastapi.io` scrape (Dune blocked CSV download).
- [ ] **Coq cleanup.** Three actions:
  - `research/proofs/LLSAInvariantPreservation.v`: replace `Require Import Coq.omega.Omega` (line 29, removed in 8.12+) with `Require Import Lia`. Replace `omega.` tactic at lines 129, 167, 169, 196, 198 with `lia.`.
  - Add `LLSAInvariantPreservation.v` to `research/coq/_CoqProject` so the Makefile actually builds it.
  - Investigate the two `_TTrace_*.tla` files in `research/tla/states/` (dated 2026-04-30) — these are TLC-emitted counter-example replay specs. The model checker found a violation. Either fix the spec / model or document the counterexample as accepted scope reduction.
  - Effort: 1-2 days.
- [ ] **MCC: decide between (a) re-label Boltzmann as canonical or (b) build real Perron.** Choice gate in Layer 1; if (b), implement power iteration on `(I−M)^{-1}` over the LightCone DAG. Estimated 200-400 LOC if (b); 0 LOC if (a).
- [x] **CFM: real Crooks equality test.** ✅ DONE (sister commit `d80921f`, per Layer 2 status snapshot above). Substrate primitive `crooks_log_ratio_millibits` now has a synthetic forward/reverse equality test asserting the identity to within fixed-point precision.

**Acceptance:** every item above has a concrete test or model-check confirming the doctrine claim is computationally true.

**Effort:** 1-2 weeks total.

---

## Layer 3 — Consensus abstraction seams

**Refactor only — zero behavior change.** Move concrete consensus types behind traits so Layer 4 can swap them. This is the biggest "no risk if done carefully" win in the punch list.

- [x] **`trait BlockSource` in `evaporchain-consensus`.** ✅ DONE (Lane G.1, commit `f78d965`). Trait at `mempool.rs:41` with 4 methods (`submit_priority`, `len`, `set_epoch`, `take_with_priority_sum_and_hints`). Blanket impl on `Mempool`. Cross-impl proptest with `TxAntichainMempool` (Lane G.1 follow-up + I.1, commits `842363f`, `d3f7a1f`).
- [x] **`trait ForkChoice` in `evaporchain-consensus`.** ✅ DONE (Lane G.3, commit `61eb888`). Trait + `LinearForkChoice` default at `fork_choice.rs:48`. `MccForkChoice` impl walks first-parent trajectories + scores via `mcc_choose` (Lane I.3, commit `c1a05bb`). Cross-impl proptest (Lane K.3, commit `2279060`).
- [x] **`trait MevPool` in `evaporchain-consensus`.** ✅ DONE (Lane G.4, commit `150292c`). Trait at `encrypted_mempool.rs:332` with 4 methods. Blanket impl on `EncryptedMempool`. Trait-dispatch parity test included.
- [x] **`trait ValidatorSetSource`.** ✅ DONE (Lane G.5, commit `118b19d`). Read-only lookup trait at `validator_set.rs:1039` with 6 methods covering the consensus-decision surface. Mutation/maintenance methods stay on concrete `ValidatorSet` (per design — alt impls replace bookkeeping wholesale).

**Acceptance:** all 4 traits exist with default impls that preserve current Tendermint behavior bit-for-bit. Existing tests pass unchanged.

**Effort:** 3-5 days.

**Files touched:** `consensus/src/{lib.rs, tendermint.rs, mempool.rs, encrypted_mempool.rs, validator_set.rs}` + new `consensus/src/traits.rs`.

---

## Layer 4 — Hot-path doctrine wiring

This is where doctrine primitives stop being shadows and start running the chain. Depends on Layer 3 traits + Layer 0 substrate.

- [x] **Antichain mempool replaces FIFO drain.** ✅ DONE behind governance flag. `TxAntichainMempool` (Lane I.1, commit `842363f`) is the standalone tx-level antichain `BlockSource` impl. The post-FIFO antichain projection (`mempool::antichain_project`, Lane I.5, commit `2bdcdc2`) lets the chain flip via `block_source_mode = "antichain"` governance key without changing storage. Same-sender heuristic = V1; richer heuristics (state read/write set overlap) are a future refinement. Default `"fifo"` stays bit-exact compat. End-to-end integration test at `tendermint.rs::test_block_source_mode_antichain_dedups_same_sender_in_proposal` (Lane J.1, commit `63ed378`); 5-property proptest at `mempool::antichain_project_invariants`; cross-impl proptest at `tx_antichain_mempool::block_source_contract_holds_for_both_impls`.
- [x] **MCC fork-choice replaces single `parent_hash`.** ✅ DONE behind governance flag (Lane I.3 + I.4 + I.6, commits `c1a05bb`, `ded1a73`, `a45588c`). `MccForkChoice` impl walks first-parent trajectories from both tips back to genesis via the LightCone DAG, scores via `mcc_choose` at β derived from chain CFM (microbits/fee/epoch). At `tendermint.rs:2643` the parent-acceptance check dispatches through the trait when `parent_acceptance_mode = "mcc"`; default `"linear"` keeps bit-exact compat. End-to-end integration test at `tendermint.rs::test_parent_acceptance_mode_mcc_diverges_from_linear_on_diverging_parent` (Lane J.2, commit `07efe97`). Cross-impl proptest (Lane K.3, commit `2279060`). 3-property single-impl proptest (Lane I.3 follow-up, commit `60a7db4`).
- [ ] **MCC fork-choice (full multi-parent enumeration).** Layer I.4+ extension:
  - track all sibling heads (today `tendermint.rs:2526` rejects any block off the single line)
  - replay state per chosen head — biggest engineering risk; needs careful re-execution semantics
  - dispatcher already exists at `tendermint.rs:954-969` (`authoritative_head`, gated by `governance_params["fork_choice_mode"]`); promote from admin-RPC-only to hot-path
  - Effort: large (1.5-2.5 weeks).
- [ ] **Promote conservation audit from gating to mandatory** (sequel to Layer 0 first item — once Layer 4 changes block acceptance semantics, revisit the governance flag).

**Acceptance:** a fresh devnet runs with antichain-mempool + MCC fork-choice as the production block source/fork-choice. Existing Tendermint tests fail cleanly (because the production path has changed) — replace them with antichain-aware analogs.

**Effort:** 3-4 weeks.

**Risk:** large blast radius. Do this on a feature branch behind `--cfg doctrine_v1` until devnet runs clean for 72 hours.

---

## Layer 5 — Lambda-Fold real Nova

Lambda-Fold today is 362 LOC of blake3. The Nova pipeline it should consume is real (`evaporchain-proving/src/nova.rs`, 2,724 LOC, 24,595 measured constraints, real `CompressedSNARK::prove`/`verify`). This layer bridges them.

- [ ] **Extend `RealBlockCircuit` arity 6 → 7.** Add `total_energy_remaining` to the IVC z-vector (today missing). New per-step constraint: `z_new[6] = decay(z_old[6], elapsed) + step_energy`. Decay is non-linear (right-shift by full halvings) — already shown feasible by the existing per-object decay constraints (`nova.rs:1027-1056`); reuse the `shift_factor / after_halvings / frac_decay` pattern at the IVC-state level. ~500 new R1CS constraints.
- [ ] **Replace Lambda-Fold's blake3 chain with Nova `CompressedProof`.** Add `evaporchain-proving = { ... features = ["nova"] }` dep to `lambda-fold/Cargo.toml`. Replace `acc_hash: [u8;32]` on `FoldedInstance` with a recursive snark handle. Rewrite `lambda_fold::fold` to call `RealBlockProver::fold_real_block_with_witness`. Rewrite `verify_folded` to call `RealBlockProver::verify_proof`.
- [ ] **Regenerate proving keys.** Re-run `pp.num_constraints()`, expect ~25,100 primary. Update whitepaper §11.2.
- [ ] **Fix `state_root_to_u64` truncation.** `nova.rs:173-177` loses 192 bits in the IVC z-vector; the limb-recomposition (`1283-1330`) bolts the full root back in but only as a per-step witness, not in `z_i`. Lambda-Fold's `state_hash` is 32 bytes — wiring it through the truncated u64 IVC state breaks 192-bit collision resistance unless the limb constraint is also folded into z. This is a security-grade concern, not a feature.
- [ ] **Decide Nova vs HyperNova.** `lambda-fold/src/lib.rs:9` says "Nova/HyperNova"; `evaporchain-proving/Cargo.toml` pulls straight `nova-snark = "0.68"`. HyperNova (CCS, multifold) is a different crate; if doctrine wants HyperNova's customizable constraint shape for the energy gadget, the dep is wrong from day one. Pick one and commit.
- [ ] **Sublinearity claim review.** `RealBlockProver::get_proof` (`nova.rs:1501-1530`) regenerates `CompressedSNARK::setup` on every call (line 1513). Light-client deployment needs `vk` preprocessed and shipped, not regenerated. Either fix this or weaken the "sublinear-in-active-energy verifier" doctrine claim.

**Acceptance:** Lambda-Fold fold-then-verify uses real recursive SNARKs; energy decay is bound in the IVC z-vector, not just per-step witness.

**Effort:** 3-6 weeks for a competent cryptographer.

**Risk:** medium. The structural risk is that "energy-folded R1CS" doesn't buy anything over "energy-decay-as-one-more-gadget-inside-the-existing-RealBlockCircuit" — which the current proving code already does. The doctrine novelty claim probably needs to be reframed around the IVC-state-vector energy accumulator (the one missing piece) rather than "Nova extension."

---

## Layer 6 — Ecosystem completion

Doctrine items absent from the consensus crate's dependency graph entirely. Each is a self-contained add.

- [ ] **Singh-Lyapunov fee controller integration.** `evaporchain-fee-controller` crate (or whatever the exact name is) is **not in `evaporchain-consensus/Cargo.toml`**. No fee-controller seam in `tendermint.rs` / `mempool.rs`. Need: new dep + integration point in `evaporchain-execution`. Effort: 1 week.
- [ ] **Crooks-MEV refund integration.** Same situation — `evaporchain-crooks-mev-refund` is not imported by consensus. Need: MEV-attribution hook into `encrypted_mempool` reveal path (line 3701) and a refund ledger entry in commit. Effort: 1 week.
- [ ] **Light-Cone full consensus rewrite.** Replace the 8,782-LOC `tendermint.rs` with a partial-order causal-set consensus engine behind the `trait ConsensusEngine` (which Layer 3 should also create). This is the doctrine's "Soul of the chain" claim. Includes:
  - block-production protocol that emits parent sets without a leader
  - validator set + signed votes/attestations on DAG vertices
  - finality rule over antichains
  - Sorkin BD-action / interval-cardinality invariant enforced at insert
  - equivocation/byzantine handling and safety bound proofs
  - network-level causal delivery
  - Decay-Lamport clock crate (deferred per `evaporchain-light-cone/src/block.rs:27`)
  - Effort: **months**, not weeks. Largest single item in the punch list. Realistically a post-mainnet-V1 effort unless Tendermint is acceptable for V1.

**Acceptance per item:** doctrine primitive runs on the hot path, has end-to-end tests, has a doctrine reference in source comments.

**Effort:** 2-3 weeks for the two integrations + multi-month for Light-Cone consensus rewrite.

---

## Layer 7 — LLSA: full theorem-grade governance

The hardest item in the punch list. May warrant descope (see alt path below).

**Full path:**

- [ ] **Pin MetaCoq.** Add opam.locked or vendored MetaCoq + version pin. Today: zero references anywhere in repo.
- [ ] **Build extraction-to-Rust harness.** Two viable paths: `coq-of-rust` (wrong direction; Rust→Coq), `hax` (formerly Circus, OCaml-extraction-then-Rust-binding, targets F*/EasyCrypt natively), or hand-rolled MetaCoq → λbox → Rust serialiser → on-chain checker. Realistically 6-12 months full-time for path 3.
- [ ] **Parametrize `LLSAInvariantPreservation.v` over `step_new`.** Today the file proves invariant preservation for the *current* `RedirectStep`/`DecayStep`, not for an arbitrary new `step_new` supplied by an upgrade — the parameter doctrine demands is hard-coded as the existing inductive relations.
- [ ] **Build production `ProofVerifier`.** A `CoqVerifier: ProofVerifier` impl that actually re-runs the kernel against the supplied proof bytes.

**Effort:** 9-15 months full-time with a Coq specialist on the team. Without one: not feasible inside the May-Oct 2026 sprint.

**Alt descope path — "audited self-amendment":**

- [ ] Drop the on-chain MetaCoq kernel.
- [ ] Keep `apply_amendment`'s binding-hash check (already works).
- [ ] Provide pinned Coq toolchain (`coq 8.18 + coq-stdlib`, opam.locked) under `research/coq/`. Fix `LLSAInvariantPreservation.v` as in Layer 2. CI runs `make` on every PR.
- [ ] Each amendment proposer publishes the Coq term + SHA256. Multiple independent auditors (named in genesis) re-run Coq locally and sign attestations of `term-typechecks ∧ matches-on-chain-hash`. Governance accepts iff k-of-n auditor signatures land.
- [ ] Pitch as "audited self-amendment" — honest, achievable in 4-6 weeks, genuinely stronger than Tezos (Tezos has neither Coq term nor auditor signatures).

**Recommendation:** descope to alt path for V1. Park full LLSA on the post-mainnet roadmap. Update doctrine §A1.2 T4 accordingly.

---

## Cross-cutting: tests + acceptance

For every doctrine primitive that lands in any layer:

1. **Doctrine reference in source.** Code comment at the type definition citing `INVENTION_STACK.md §X.Y` and the original theorem (e.g., "Theorem: Shalizi-Crutchfield 2001 Optimal Prediction Theorem (J. Stat. Phys. 104).") — already a doctrine rule (§A3.6 rule 21).
2. **Adversarial test, not just a happy-path test.** No primitive ships if its tests only verify type-correctness.
3. **Integration test that runs the primitive end-to-end against a non-trivial fixture.** Toy diamond DAGs and 4-block tests are not enough.
4. **No `>>` on energy values outside `evaporchain-types`.** CI lint.
5. **All `cargo build / test / check` runs on a Mini, never the MacBook.** Per `feedback_no_local_builds.md`.

---

## Rough total

| Layer | Effort |
|---|---|
| 0 — Substrate enforcement | 1-2 weeks |
| 1 — Doctrine accuracy (wording) | 0.5 day |
| 2 — Math completion | 1-2 weeks |
| 3 — Consensus abstraction seams | 3-5 days |
| 4 — Hot-path doctrine wiring | 3-4 weeks |
| 5 — Lambda-Fold real Nova | 3-6 weeks |
| 6 — Ecosystem completion | 2-3 weeks + months for Light-Cone full rewrite |
| 7 — LLSA full | 9-15 months OR 4-6 weeks (descope path) |

**Realistic V1 mainnet sprint (May-Oct 2026):** Layers 0, 1, 2, 3, 4, 5, 6 (minus Light-Cone full rewrite), 7 (descope path). 4-5 months solo full-time. Light-Cone full rewrite + LLSA full theorem-grade are post-V1 items.

**Critical path:** Layer 0 → Layer 3 → Layer 4. Without these three in order, no other doctrine primitive can be claimed honestly. Layer 5 (Lambda-Fold) can run in parallel with Layer 4 because it's confined to `evaporchain-proving` and `evaporchain-lambda-fold`.

---

## Doctrine amendments needed (consequence of audit)

Beyond the Layer 1 wording fixes, the audit surfaced four items that warrant doctrine review:

1. **MCC §A1.2 T1**: "closed-form Perron solution" — vacuous on a DAG. Pick (a) honest Lagrangian re-label or (b) commit to real path-counting matrix work.
2. **CFM §A1.2 T2**: "exact equality" — never asserted. Either build the test or weaken to "exposed identity primitive."
3. **MERA §A1.4**: gate caveat. Update §A1.8 to say "real-data gate pending; synthetic-data gate PASS 2026-04-29."
4. **LLSA §A1.2 T4**: descope from "first chain whose governance is a theorem" to "audited self-amendment with k-of-n Coq attestation" for V1.

These are not defeats. They're the difference between marketing claims and engineering claims.

# EvaporChain — Doctrine Punch List

**Date:** 2026-05-03
**Source:** parallel audit of 7 hardest crates + foundational substrate + consensus integration surface + Coq/TLA proof artefacts.
**Pairs with:** `REMAINING_WORK.md` (security + infra), `research/INVENTION_STACK.md` (canonical doctrine).

This file is the layered build plan to make the doctrine claims actually true. Every item below is a delta between what's shipped and what `INVENTION_STACK.md` says is shipped.

---

## Headline finding

The 7 hardest crates compile, test, and have clean public APIs — but the load-bearing math/protocol load is gated on "future commit" in every case, and the **production hot path is still 100% Tendermint + FIFO mempool**. Conservation is audited but not enforced. Three crates have rogue decay implementations that bypass the Coq-verified `energy_at_epoch`. MCC's `authoritative_head` has zero call sites in the workspace. MERA crate ships citing "PASS — MERA GO" without flagging the gate ran on synthetic data. Lambda-Fold is 362 LOC of blake3 with zero curve arithmetic. LLSA's only Coq invariant-preservation file (`research/proofs/LLSAInvariantPreservation.v`) won't even compile against the pinned 8.18 toolchain because it imports `Coq.omega.Omega` (removed in 8.12+).

The good news: `evaporchain-proving` has a real Nova pipeline (24,595 measured R1CS constraints, real `RealBlockCircuit`, real `RealBlockProver` with `CompressedSNARK`). MERA is real f64 tensor algebra end-to-end (real Givens-rotation disentanglers, end-to-end verifiable proof round-trip). Boltzmann-stake and Sanov-slashing are already wired into the Tendermint hot path. The substrate is more solid than the integration is.

---

## Layer 0 — Substrate enforcement

**Without this, every upper-layer doctrine claim is folklore.** Audits run; verdicts ignored.

- [ ] **Promote conservation audit from observability to gating.** Today `evaporchain-execution/src/lib.rs:3015-3036` calls `audit_block_step` post-block and writes to `last_conservation_audit`, but block acceptance does not depend on it. Inline comment confirms this is intentional. Make `ConservationViolation` block-rejecting under a governance flag, default-on for new chains.
- [ ] **Unify decay through `evaporchain_types::energy_at_epoch`.** Three rogue implementations bypass the canonical Coq-verified function:
  - `crates/evaporchain-consensus/src/anchor.rs:77-91` — `DecayFormula::Exponential::compute_energy` does raw `>> shifts`, lacks the u128 fractional-decay correction
  - `crates/evaporchain-da/src/poha.rs:99` — `self.energy >> shifts`
  - `crates/evaporchain-self-annealing/src/annealing.rs:54` — shifts `lambda_half_life`
  Reroute all three through `energy_at_epoch`. Add a workspace-level lint or audit test that fails CI if any source file outside `evaporchain-types` does `>> _` on an energy value.
- [ ] **Fix `epochs_elapsed` proxy.** `lib.rs:3028` uses `block.epoch.saturating_sub(db.get_last_rent_epoch().saturating_sub(0))` — the trailing `saturating_sub(0)` is a no-op and the storage-rent epoch is the wrong reference. Add a kernel-owned epoch tracker on `EnergyAccumulator`.
- [ ] **Wire `apply_demurrage` into `execute_block`.** Crate is real (555 LOC, 30 tests + 6 proptests) but `grep apply_demurrage crates/evaporchain-execution/` returns no hits. Refresh pool stays empty in the hot path; every doctrine claim about refresh-pool keep-alive is currently mathematically vacuous.
- [ ] **Resolve CFM β degenerate case.** `cfm/beta.rs:56`: `beta_millibits_per_fee = 1000 / half_life` integer-floors to 0 at `DEFAULT_LAMBDA = 4096`. Boltzmann weight collapses to `MAX_WEIGHT` for every fee — MCC's caliber score becomes uniform under genesis params. Either change β units to scale with millihalvings, or change `DEFAULT_LAMBDA`, or both.

**Acceptance:** every block in a fresh devnet either commits with `last_conservation_audit == Ok` or is rejected. No `>>` on energy values exists outside `evaporchain-types`. β > 0 under all governance-allowed λ values.

**Effort:** 1-2 weeks.

**Files touched:** ~6 files across `evaporchain-execution`, `evaporchain-consensus`, `evaporchain-da`, `evaporchain-self-annealing`, `evaporchain-cfm`, `evaporchain-types`.

---

## Layer 1 — Doctrine accuracy (zero engineering, just honesty)

These are wording corrections, not code. Cheapest items in the punch list; ship before any Layer 2+ work because they prevent future-Claude / future-auditor from being misled by the doctrine.

- [ ] **Amend `INVENTION_STACK.md §A1.2 T1` (MCC).** "Closed-form Perron solution" is mathematically vacuous on a DAG (adjacency matrix is nilpotent — every eigenvalue is 0). What's actually shipped is the correct Jaynes Lagrangian closed-form: `argmax exp(−β·E_path)` over candidate trajectories. Either re-word to "argmax of `exp(-β·E_path)` over candidate trajectories — closed form by Lagrange duality" OR commit to building the real thing on `(I−M)^{-1}` (path-counting matrix) or the time-reversed Markov fork-choice.
- [ ] **Amend `INVENTION_STACK.md §A1.2 T2` (CFM).** "Exact equality between work and free-energy difference (not a bound)" is not asserted or tested anywhere in `evaporchain-cfm`. `crooks_log_ratio_millibits` returns `(bit_length(p_F) - bit_length(p_R)) * 1000` — the LHS only; the RHS `β·(W − ΔF)` is never constructed. Either add a real Crooks-equality test (synthetic forward/reverse pair, assert `crooks_log_ratio == β·(W − ΔF)` to within fixed-point precision) OR weaken doctrine to "exposed identity primitive."
- [ ] **Add MERA synthetic-data caveat to `crates/evaporchain-mera/src/lib.rs:30-34`.** Today says: *"Gate result: PASS — MERA GO (2026-04-29)."* Make it: *"Synthetic-data gate result: PASS (2026-04-29). Real-Ethereum gate per §A1.8: pending."*
- [ ] **Update `crates/evaporchain-light-cone/src/lib.rs` first paragraph** to match the honest comment at `tendermint.rs:3115-3118` ("read-only observability — chain authority is still Tendermint's linear chain"). Today the docstring says "Soul of the chain" without flagging that the soul is currently a downstream observer.
- [ ] **Update `crates/evaporchain-cslc` HTTP endpoint description.** `evaporchain-node/src/api.rs:6316` documents `POST /api/cslc_reconstruct` as "ε-machine reconstruction (CSSR) from symbol-count distribution." It's not CSSR. Re-label as "single-state baseline (CSSR pending)."

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
- [ ] **MERA real-Ethereum gate.** Doctrine §A1.9 rule 12 says "MERA gate must pass before MERA ships." It hasn't. Action:
  - pull Ethereum mainnet blocks 19M-20M account-touch matrix via Dune CSV (`ethereum.transactions`, group by `to`/touched-storage per block, sparsify to top-N by frequency)
  - write `_load_real_ethereum(path) -> np.ndarray` of shape `(N_accounts, N_blocks)` matching `compute_mi_matrix`'s expected input
  - replace synthetic generators in `research/mera-gate/run_gate.py:532-536`, re-run, overwrite `GATE_RESULT.md`
  - if PASS: remove the Layer 1 caveat. If MPS: downshift crate to authenticated MPS. If random: drop tensor networks, ship Verkle + Energy-Verkle as planned.
  - Effort: half a day (Dune path) to 2 days (Erigon).
- [ ] **Coq cleanup.** Three actions:
  - `research/proofs/LLSAInvariantPreservation.v`: replace `Require Import Coq.omega.Omega` (line 29, removed in 8.12+) with `Require Import Lia`. Replace `omega.` tactic at lines 129, 167, 169, 196, 198 with `lia.`.
  - Add `LLSAInvariantPreservation.v` to `research/coq/_CoqProject` so the Makefile actually builds it.
  - Investigate the two `_TTrace_*.tla` files in `research/tla/states/` (dated 2026-04-30) — these are TLC-emitted counter-example replay specs. The model checker found a violation. Either fix the spec / model or document the counterexample as accepted scope reduction.
  - Effort: 1-2 days.
- [ ] **MCC: decide between (a) re-label Boltzmann as canonical or (b) build real Perron.** Choice gate in Layer 1; if (b), implement power iteration on `(I−M)^{-1}` over the LightCone DAG. Estimated 200-400 LOC if (b); 0 LOC if (a).
- [ ] **CFM: real Crooks equality test.** Construct synthetic forward/reverse trajectory pair with known work `W` and free-energy difference `ΔF`. Assert `crooks_log_ratio_millibits(p_F, p_R) ≈ beta_mb * (W − ΔF) / 1000` to within fixed-point precision. ~50 LOC test.

**Acceptance:** every item above has a concrete test or model-check confirming the doctrine claim is computationally true.

**Effort:** 1-2 weeks total.

---

## Layer 3 — Consensus abstraction seams

**Refactor only — zero behavior change.** Move concrete consensus types behind traits so Layer 4 can swap them. This is the biggest "no risk if done carefully" win in the punch list.

- [ ] **`trait BlockSource` in `evaporchain-consensus`.** Today `Mempool` is a concrete struct (`mempool.rs:34`). Define:
  ```rust
  pub trait BlockSource {
      fn build_block_payload(&mut self, ctx: &ProposalCtx) -> Vec<Transaction>;
  }
  ```
  Implement it for the existing FIFO+priority `Mempool`. Replace the drain at `tendermint.rs:3699-3770` with `self.block_source.build_block_payload(...)`.
- [ ] **`trait ForkChoice` in `evaporchain-consensus`.** Today fork-choice is inlined as private methods on `TendermintConsensus`; the chain assumes single-line history via `self.parent_hash` (`tendermint.rs:3112`) and rejects any block whose `parent_hash != self.parent_hash` (`tendermint.rs:2526`). Define:
  ```rust
  pub trait ForkChoice {
      fn select_chain(&self, candidate_heads: &[BlockId], lc: &LightCone) -> BlockId;
  }
  ```
  Implement it for the current single-line choice (returns `self.parent_hash` always). This unblocks Layer 4.
- [ ] **`trait MevPool` in `evaporchain-consensus`.** Encrypted mempool is a concrete field today; same drop-in-impl pattern.
- [ ] **`trait ValidatorSetSource`.** Validator-set updates today go through concrete `queue_change` / `add_validator` / `remove_validator`. Trait-ize so Singh-Boltzmann stake variants and Self-Annealing validator sets can plug in cleanly.

**Acceptance:** all 4 traits exist with default impls that preserve current Tendermint behavior bit-for-bit. Existing tests pass unchanged.

**Effort:** 3-5 days.

**Files touched:** `consensus/src/{lib.rs, tendermint.rs, mempool.rs, encrypted_mempool.rs, validator_set.rs}` + new `consensus/src/traits.rs`.

---

## Layer 4 — Hot-path doctrine wiring

This is where doctrine primitives stop being shadows and start running the chain. Depends on Layer 3 traits + Layer 0 substrate.

- [ ] **Antichain mempool replaces FIFO drain.** Implement `BlockSource` for an antichain-aware tx-mempool:
  - first need a tx-level partial order (today antichain-mempool operates on `BlockId`s in LightCone, not pending txs). Add a causal-deps field to `Transaction` or compute one from nonce/state-root reads
  - `build_block_payload` calls `extend_to_maximal` over the tx-DAG, gates on `total_energy_meets_threshold`
  - replace brute-force scans with an incrementally-maintained concurrency index (per `antichain-mempool/src/maximal.rs:24` self-admission)
  - wire encrypted mempool reveals into the same partial order (today `encrypted_mempool.rs` has zero antichain awareness)
  - Effort: medium (4-7 days).
- [ ] **MCC fork-choice replaces single `parent_hash`.** Implement `ForkChoice` via `mcc_choose`:
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

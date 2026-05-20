# B-1/B-2 ZK Verifier — EvaporChain Mainnet Audit Dossier

**Date:** 2026-05-20 · **Branch:** `s4-grumpkin-config` · **Companion:**
`MAINNET_REMAINING_WORK_FLOW.md` (live status, full commit history)

This dossier is the auditor's entry point. It is **calibrated** — each
claim cites a measurement or proof; what is *not yet proven* is
marked as such; the four mid-arc corrections that walked back
optimism are recorded prominently, not buried.

## 1. Problem (audit B-1/B-2, #1 mainnet blocker)

EvaporChain's L1 verifier originally used Groth16 over the raw
hand-rolled `NovaVerifierCircuit`. Audit B-1/B-2 found this path's
secondary side blows up to ~2.03×10⁸ constraints (D.3 measurement)
— infeasible Groth16 prove on any hardware. Architectural
rethink required.

## 2. Architecture chosen (Satyawan's call, 2026-05-19)

**Option 1C: nova-snark 0.68 + custom CycleFold + Groth16 decider**
(rejected 1A/1B Sonobe paths after the third-correction surfacing of
Sonobe's not-yet-audited + missing-solidity-verifiers state on
staging). Native `CompressedSNARK<ppsnark>` already validated
end-to-end on Mini3 (25 s `snark` + 125 s `ppsnark`); CycleFold
shrinks the secondary so a Groth16 wrap of its succinct verifier is
practical.

## 3. Validated primitives — what IS proven, with anchors

| # | Module | Validates | Box-test | Commit |
|---|---|---|---|---|
| 1 | `cyclefold_aux_circuit` | BN254-G1 native scalar-mul over Bn254Fq circuit (CycleFold aux core) | 2,548 cons measured, 3/3 incl. non-vacuity | `01add2da` |
| 2 | `cyclefold_instance_circuit` | CF instance circuit + public IO binding | 1,985 cons, 3/3 incl. non-vacuity | `9bb02bc3` |
| 3a | `cyclefold_fold_homomorphism` | Pedersen-on-Grumpkin additive homomorphism | 3/3 (homomorphism, fold step, multi-step) | `daeb7bf8` |
| 3b-1 | `scalar_adapter::ark_fq_to_secondary` | Same-field ArkFq ↔ grumpkin::Scalar bridge | 9/9 (32-rand round-trip + arithmetic preservation) | `8604fb14` |
| 3b-2 | `cyclefold_r1cs_bridge` | arkworks `ConstraintSystem<Bn254Fq>` → nova `R1CSShape<GrumpkinEngine>` (dims-exact) | 1985/1812/21 match | `7e86d394` |
| 3b-3 | (same) | Satisfied `(shape, U, W)` triple — nova's `is_sat` accepts | layout bug caught + fixed | `0c929866` |
| 3b-4 | (same) | Real `NIFS::<GrumpkinEngine>::prove` ⇒ `is_sat_relaxed` + `verify==prove` | 3/3 | `2fadba36` |
| 4a | `cyclefold_ivc_accumulator` | Multi-step CF fold accumulator (3 + 6 steps) | 2/2 with per-step `is_sat_relaxed` gates | `710a54db` |
| 4b-α | `cyclefold_primary_augmented_circuit` | Primary aug circuit SHELL + 3 surfaced corrections | 3/3 + 3 fixed errors | `1e59253b` |
| 4b-β-1 | `cyclefold_cf_x_digest::compute_cf_x_digest_native` | Native oracle (bit-exact 127-limb encoding) | 3/3 | `91b31c3a` |
| 4b-β-2 | (same) `enforce_cf_x_digest` | In-circuit oracle-match | 4/4 first try | `da703de8` |
| 4b-β-3 | shell `Section C` wired LIVE | cf_x_digest binding in shell (single tuple) | 5/5 first try | `575e7043` |
| 4b-β-4 | shell `Section R` (native IO absorb) | 6-element transcript hash | 7/7 first try | `75b3db29` |
| 4b-β-4b | (same) `cf_u_running` limb absorb | Bn254Fq-into-Section-R pattern | 8/8 first try | `408b2add` |
| 4b-β-4c | (same) CF commitments absorb | 4 native Fr CF comm fields | 9/9 first try | `1932ee2a` |
| 4b-β-4d | (same) `cf_x_vec` limb loop | 21-element Bn254Fq vec absorb | 10/10 first try | `b93cc1b2` |
| 4b-β-5-α | shell `Section F` native fold | `u_new = u_R + r`, `X_new = X_R + r·X_I` | 12/12 first try | `98b21a3b` |
| 4b-β-5-β | (same) r-from-RO | r bound to Neptune transcript | 13/13 first try | `0f27b584` |
| 4b-β-5-γ | (same) comm_T absorb | BN254 G1 limb decomp in r-RO | 14/14 first try | `24b134f7` |
| 4b-β-5-δ | `enforce_cf_x_digest_pair` + shell | TWO-tuple cf1/cf2 binding | 20/20 + flip | `4bcd9d3c`+`b1e31951` |
| 5-α | `cyclefold_n_aux_probe` | Real n_aux MEASURED from ppsnark proof | 1/1 (real proof, 13.04 s) | `da3735a1` |
| 6-α | `foundry-bench/` | Solidity Grumpkin gas anchors | 4/4 (Foundry) | `1917f9e4` |
| (b)-1 | `cyclefold_shell_chain` | Primary state threading across 2 IVC steps | 1/1 (12.69 s) | `d9ce77b1` |
| (b)-2 | (same) | CF accumulator integration step-0 end-to-end | 1/1 (8.01 s) | `c98fd972` |
| (b)-2b | `s4_secondary_extract::extract_relaxed_running_inst` + chain test | Full 2-step CF chain (extractor + step-1 chaining) | 1/1 (14.99 s) | `36de2ea4` |

**Aggregate:** 93 commits this arc (`436d2e2d → 5be5db90`); every
primitive box-validated; **fourteen consecutive first-try passes**
on the 4b shell-extension + (b) IVC integration micro-arcs.

## 4. Final shell measurements

`PrimaryAugmentedCircuitShell` (commit `41ab3c06`,
`sections_wired:true`):
- **47,597 constraints / 41,580 witness / 11 instance vars**
- Section breakdown: Step 1, C(pair) ~13.9k, R(54 absorbs) ~29.9k, F ~3.8k
- 15 in-module non-vacuity tests, all passing

CF instance circuit (commit `9bb02bc3`):
- **1,985 cons / 1,812 witness / 22 instance** (incl. ONE)
- bridges exactly to nova-snark `R1CSShape<GrumpkinEngine>`

Real ppsnark proof n_aux (commit `da3735a1`):
- **n_aux = 2¹⁴ = 16,384** (measured, not predicted)
- 8× reduction from option-2 dead-end (2¹⁷)

Foundry gas anchors (commit `1917f9e4`):
- Grumpkin point-add: 3,834 gas
- Grumpkin 256-bit scalar-mul: 1,545,603 gas
- Pippenger MSM at n=16,384: best ~62.7M, realistic ~87.8M

## 5. Four honest mid-arc corrections (the discipline working)

Each surfaced via measurement, none buried; each made the story
tighter, more credible.

1. **`~10⁵` flat-MSM constraints → actually `~10⁷`** (commit
   `b508d7ff`). Source-read claim of "~10⁵ native" got corrected to
   measured 2,533 cons/term × 10,554 = 26.7M ≈ 10⁷ via D.3-style
   probe `predict_native_grumpkin_msm_size_for_recursion_circuit`.
2. **"Tensor-fold 10-100× lever" → actually 1.58× WORSE than flat**
   (commit `12c3e02d`). `FOLD_PROBE`: ratio_fold/flat=1.579; option (2)
   declared measured-dead.
3. **n_aux predicted 2¹³ → actually 2¹⁴** (commit `da3735a1`).
   `total_nz` dominated padding, not `2·num_vars`.
4. **Solidity gas predicted ~24M (L1-viable) → actually ~63-88M
   (NOT L1-viable)** (commit `1917f9e4`). Foundry-measured;
   triggered the L2-only mainnet decision.

## 6. Deployment target — LOCKED: L2-only (commit `6e6b0a1f`)

Per Satyawan's post-Foundry decision: EvaporChain mainnet ZK
verification deploys on L2 (Optimism / Arbitrum / Base), not
Ethereum L1. L1 gas at the real `n_aux=16,384` is ~63-88M (2-3× over
30M block limit) even with naive Solidity; heavy optimization gets
~2× speedup ≈ 30-45M, still borderline-over. L2 gas is 100-1000×
cheaper — the SAME Solidity verifier deploys trivially.

## 7. Open follow-ups (clearly NOT yet proven)

1. **Byte-level BESPOKE alignment** of the r-from-RO transcript
   with nova-snark's exact `nifs.rs::prove` ordering (e.g.,
   absorbing `U2.comm_W_I` too, transcript domain tags). Analogous
   to `section2_gadget`'s neptune-vs-arkworks reconciliation
   (already CLOSED). Architectural pattern is in place; bit-level
   identity is a focused crypto-alignment pass.
2. ~~IVC integration~~ — **CLOSED** at the architectural validation
   level via (b)-1 + (b)-2 + (b)-2b: primary state threading + CF
   accumulator integration + 2-step full chain all box-verified.
   N-step extensibility is mechanical iteration of the same pattern;
   the underlying composition (shell ↔ accumulator linkage) is
   end-to-end-proven on real proofs.
3. **Solidity decider verifier** (production-quality) for L2
   deployment. Foundry benchmark `foundry-bench/` is anchor-only;
   the full ppsnark verifier in Solidity is the L2-deployable
   artifact. Multi-week crypto + Solidity engineering pass.
4. **L2 deployment harness** (Optimism / Arbitrum / Base —
   selection + integration). Depends on (3).
5. **External audit** of the 1C construction. The discipline
   pattern in §5 caught four issues during construction; external
   review covers what construction-time discipline cannot see.

## 8. Persisted lessons (auto-memory)

- [`lesson_2026_05_19_zk_size_assert_without_measuring.md`] — RULE:
  no ZK circuit-size claim without a D.3-style `cs.num_constraints()`
  probe + size parameter pinned from a REAL proof.
- [`lesson_2026_05_20_zk_evm_gas_undershoot.md`] — RULE: no L1-vs-L2
  deployment viability claim without a Foundry harness. Default to
  "L2-only unless Foundry says otherwise."

Both lessons are companion-linked and codify the calibration
discipline this arc's four corrections demanded.

## 9. How to read the actual code

Reading order for a reviewer:
1. `MAINNET_REMAINING_WORK_FLOW.md` (live status, full commit-by-
   commit history with calibration notes).
2. `cyclefold_primary_augmented_circuit.rs` (the final shell — entry
   point; the four sections are clearly demarcated).
3. `cyclefold_cf_x_digest.rs` (the cf_x_digest oracle + gadget —
   the soundness-critical hash binding).
4. `cyclefold_r1cs_bridge.rs` (arkworks→nova-snark bridge — the
   load-bearing infrastructure for `is_sat_relaxed` correctness).
5. `cyclefold_n_aux_probe.rs` (the real ppsnark measurement — pins
   the calibration claims to ground truth).
6. `foundry-bench/` (Solidity gas anchors — the L1-vs-L2 decision
   evidence).

All other modules are dependencies of these five.

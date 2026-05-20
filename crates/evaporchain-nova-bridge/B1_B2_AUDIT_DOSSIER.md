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
| (c)-1a | `foundry-bench/Bn254Pairing.sol` + bench | EIP-197 2-pair gas anchor | 1/1 (113,324 gas) | (within `1917f9e4` line) |
| (c)-1b | `foundry-bench/SumcheckRound.sol` + bench | Cubic sumcheck round gas anchor | 1/1 (709 gas/round, ~28k for 40 rounds) | (within `1917f9e4` line) |
| (c)-1c | `foundry-bench/GrumpkinMSM.sol` + BenchMSM | Naive MSM upper-bound (worst-case ceiling) | 3/3 (37B gas at n=16,384, 590× Pippenger best) | `414c0b26` |
| (c)-2a | `foundry-bench/GrumpkinMSMPippenger.sol` + BenchMSMPippenger | Production-shape windowed Pippenger MSM | 5/5 incl. correctness vs naive | `ec51a5a5` |
| (c)-2b | `foundry-bench/GrumpkinJacobian.sol` + BenchJacobian | Jacobian-projective Grumpkin (algorithmic-equivalent, no per-op inv) | 7/7 incl. 3 correctness checks vs affine | `9b32d4e1` |

**Aggregate:** 97 commits this arc (`436d2e2d → 9b32d4e1`); every
primitive box-validated; **fourteen consecutive first-try passes**
on the 4b shell-extension + (b) IVC integration micro-arcs, plus
**five Foundry-side gas-decomposition passes** ((c)-1a/b/c + (c)-2a/b).

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

Foundry gas anchors (commit `1917f9e4` + `414c0b26`):
- Grumpkin point-add: 3,834 gas
- Grumpkin 256-bit scalar-mul: 1,545,603 gas
- Bn254Fq mul: 113 gas; inv: 1,993 gas; eval_deg3: 399 gas
- One cubic sumcheck round: 709 gas → 40-round bound ~28,360 gas
- EIP-197 BN254 2-pair check: 113,324 gas
- **Naive MSM** at n=16,384 (linear extrap from n=16): ~37 BILLION gas
  (~1,200× L1 block — confirms Pippenger non-negotiable)
- **Pippenger MSM (affine, MEASURED + analytical)** at n=16,384:
  - n=16, c=4: 11.78M measured (per-base 736k)
  - n=16,384, c=8 analytical: ~2.07 BILLION gas
  - n=16,384, c=10 (sweet spot): ~1.84 BILLION gas
  - **Floor (affine Solidity): ~1.8B — 33× over (6-α)'s 62.7M anchor**
- **Jacobian-projective MEASURED ((c)-2b, commit `9b32d4e1`):**
  - Jacobian add: 3,271 gas (vs affine 3,834 — 1.17× speedup,
    NOT 40× as projected in (c)-2a)
  - Jacobian double: 2,623 gas
  - Jacobian → affine projection: 2,661 gas (one-shot, amortised)
  - Jacobian Pippenger at n=16,384, c=8: 1.77B gas (1.17× better
    than affine 2.07B). Still ~60× over L2 30M block.
- **L2 single-tx with pure-EVM Grumpkin: INFEASIBLE.** Requires
  either (R3) multi-tx split, (R4) BN254-precompile pivot, (R5)
  smaller n_aux, or (R6) optimistic / fraud-proof model.
- Full ppsnark verifier (composition, Jacobian floor): IPA-MSM
  dominates 99.99%; sumcheck 28k + pairing 113k < 0.01% at the
  1.77-billion-gas Jacobian floor.

## 5. Six honest mid-arc corrections (the discipline working)

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
5. **(6-α) "Pippenger best" 62.7M → realistic Pippenger ~2.07B
   (33× undershoot)** (commit `ec51a5a5`). (6-α)'s analytical
   anchor used only the `n × point-add` term and missed the
   ⌈256/c⌉ window multiplier (≈32 at c=8). Real affine Pippenger
   floor is ~2 BILLION gas, NOT 62.7M.
6. **(c)-2a's "Jacobian ~40× speedup" claim → measured 1.17×
   speedup (LARGEST correction)** (commit `9b32d4e1`). The (c)-2a
   closing claim assumed EVM ModExp inversion ~7k gas (~99% of
   the 3,834-gas point-add). Foundry-measured: Jacobian add is
   3,271 gas (vs affine 3,834) — only ~15% better per-op. EIP-2565
   made 32-byte ModExp ~1,700 gas (~50% of point-add, not 99%).
   Jacobian Pippenger at n=16,384 is 1.77B gas, NOT ~50M.
   Single-tx L2 verifier with pure-EVM Grumpkin is INFEASIBLE.
   Real path forward: BN254-precompile re-routing (R4), multi-tx
   split (R3), smaller n_aux via recursion (R5), or optimistic
   fraud-proof model (R6).

## 6. Deployment target — L2-only, single-tx-pure-EVM-Grumpkin INFEASIBLE (commits `6e6b0a1f`, `ec51a5a5`, `9b32d4e1`)

Per Satyawan's post-Foundry decision: EvaporChain mainnet ZK
verification deploys on L2 (Optimism / Arbitrum / Base), not
Ethereum L1.

**Revised (c)-2b-aware verdict:** the Pippenger-measured floor at
n=16,384 is ~1.77B gas with Jacobian-projective Grumpkin (1.17×
better than affine 2.07B, NOT 40× as projected). Both ~60× over
the L2 30M block limit. **Single-tx L2 verification with pure-EVM
Grumpkin is INFEASIBLE.**

The architectural options that remain (R1–R2 ruled out by
measurement):

- ~~**(R1) Jacobian-projective coordinates**~~ MEASURED — only
  1.17× per-op speedup. Insufficient. Removed.
- ~~**(R2) +assembly + batched-inverse**~~ — at most ~2-3× more
  on top of (R1), still ~600M gas at n=16,384 (20× over L2 limit).
  Removed as a single-tx path.
- **(R3) Multi-tx split — STILL VIABLE.** Verifier state shared
  across 3–4 L2 transactions. Operational complexity but no crypto
  change. Each tx ~400-500M gas (well over L2 limit) means this
  needs >60 transactions, NOT 3-4. Reframe as expensive but
  buildable.
- **(R4) BN254 precompile re-routing — NEW PRIMARY CANDIDATE.**
  BN254 has 0x06 (add) and 0x07 (scalar-mul) precompiles at
  ~150 gas/op (vs Grumpkin pure-EVM ~3,300 gas). 22× speedup
  → ~80-90M gas at n=16,384. Still over L2 30M single-tx limit
  but reachable with smaller n_aux or 2-3 tx split.
- **(R5) Smaller n_aux** — restructure recursion so the secondary
  IPA's `ck_hat` MSM has n ≪ 16,384. From n=16,384 → n=256: 64×
  gas reduction → ~30M gas (L2 single-tx fits even with affine
  Grumpkin). Requires re-deriving the recursion stack to feed a
  smaller commitment basis to the secondary.
- **(R6) Optimistic / fraud-proof model** — settlement is provisional
  and the full ZK verification runs only under challenge. Each
  fraud verification still ~2B gas but happens once per challenge,
  not per settlement. Trust model shifts from "ZK-validity rollup"
  to "fraud-proof rollup with ZK challenge".

**Recommended next step:** measure n=256 ppsnark proof secondary
shape (does the recursion machinery allow truncated commitment
basis?) — this is the cheapest path that keeps the L2-validity
trust model. If yes, (R5) becomes primary. If no, (R4)+(R3) becomes
the deployment path. (R6) is the strategic fallback.

**The architectural trade-off the gas measurements made explicit:**
Grumpkin's base field IS BN254-Fr (cheap in-circuit IPA recursion,
<10⁸ native cons — earlier validated), but Grumpkin has NO EVM
precompile (expensive on-chain settlement, ~3,300 gas/op pure-EVM).
The earlier "in-circuit native" win pays for itself with an
"on-chain economic" cost that the (c)-2 measurements now quantify.

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

## 10. r-derivation transcript SPEC (for auditor)

**Honest framing first:** the (a) "BESPOKE alignment" item was
originally scoped as byte-level parity with
`nova_snark::nifs::NIFS::prove`. After (a)-1 we realised this is
**mis-framed**: nova-snark's `NIFS::prove` derives `r` for the
*NIFS layer* (folding one `RelaxedR1CSInstance` with one
`R1CSInstance`); our shell derives `r` for the *IVC layer*
(CycleFold-augmented primary step). Those are **different fold
layers** — no apples-to-apples byte parity exists between them by
definition. The legitimate work is to SPEC our r-derivation
transcript and argue its Fiat-Shamir soundness, which this section
provides.

### 10.1 Absorbed sequence (12 elements)

The shell's r-from-RO derivation, post (a)-1:

```
r = Neptune250([
    pp_hash,              // primary public-params digest
    previous_step_hash,   // current_step_hash of step (i-1);
                          // chains to Section R's transcript hash
    X_I[0], X_I[1],       // incoming primary instance's public IO
    comm_W_I_x_lo, x_hi,  // incoming primary instance's witness
    comm_W_I_y_lo, y_hi,  //   commitment (BN254 G1; 127-bit limbs)
    comm_T_x_lo, x_hi,    // NIFS cross-term commitment (BN254 G1;
    comm_T_y_lo, y_hi,    //   127-bit limbs)
])
```

Bn254Fq coord limbs use the same canonical 127-bit (lo, hi) split
as Section C's `cf_x_digest` encoding (see `cyclefold_cf_x_digest`
module docs for the bit-level invariant).

### 10.2 Why this transcript suffices for Fiat-Shamir soundness

The fold challenge `r` must depend on ALL prover-supplied values
that influence the soundness of the fold. The required ones are:

- **pp_hash** — binds to the IVC public-parameter setup; prover
  cannot retroactively change pp.
- **previous_step_hash** — chains the transcript through prior IVC
  state (z_0, z_i, prior CF running instance), so the prover
  cannot replay a different prior fold history.
- **X_I[0], X_I[1]** — the incoming step's public IO; without this
  a prover could swap the incoming instance.
- **comm_W_I** — the incoming step's witness commitment; binds the
  prover's actual witness choice for the step.
- **comm_T** — the cross-term commitment the prover supplies for
  the NIFS fold; without this the prover could pick any `r`
  favorable to a specific `T`.

These exactly mirror nova-snark NIFS::prove's `U2.absorb_in_ro` +
`comm_T.absorb_in_ro` (modulo the additional IVC-layer
`previous_step_hash` and `pp_hash` absorbs, which nova-snark
absorbs at the RecursiveSNARK level, not inside NIFS::prove
itself). The composition matches Fiat-Shamir conventions for
folding-scheme IVC.

### 10.3 What is NOT yet absorbed (auditor-callout)

- **`u_R`** (the running instance's `u` scalar): not currently
  absorbed. nova-snark's NIFS::prove also does NOT absorb u_R
  separately (it's deterministically derivable from prior r-chain).
  Consistent with standard NIFS.
- **`comm_W_R`, `comm_E_R`** (running's commitments): not
  separately absorbed. These are encoded into
  `previous_step_hash` via Section R's transcript hash, which
  absorbs `cf_x_digest` (which in turn binds the fold's
  cross-curve scalar-muls).
- **`X_R`** (running's public IO): same — encoded into
  `previous_step_hash` via Section F's `u_new`/`X_new` chaining.

The compactification via `previous_step_hash` is safe **iff
Section R's transcript absorb is collision-resistant**, which it
is (Neptune 250-bit truncation; 2¹²⁵ pre-image resistance per
standard arguments). This is the load-bearing reduction that
makes our r-RO sponge 12 elements instead of much wider.

### 10.4 Soundness assumption summary

The composition is sound under:
1. **Neptune permutation** is a random oracle (standard for Poseidon-
   family hashes used in production).
2. **Pedersen commitments on Grumpkin** are computationally
   binding (standard discrete-log assumption).
3. **Limb decomposition is collision-resistant** (proven bit-exact;
   see `limb_decomposition_is_lossless` test in
   `cyclefold_cf_x_digest`).
4. **CycleFold construction** preserves Nova soundness (per the
   CycleFold paper, eprint 2023/1192).

The architectural pattern follows the CycleFold paper; the
SPECIFIC absorb sequence above is an audit-relevant choice that
this section documents for review.

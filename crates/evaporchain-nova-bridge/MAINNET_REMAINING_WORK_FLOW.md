# EvaporChain — Remaining Work to Mainnet (Sequential Flow)

**Date:** 2026-05-19 · **Status:** NOT mainnet-ready · **Companion:**
`S4_DESIGN.md` (detailed specs), this file = the end-to-end ordered flow.

**Calibration rules (auditor-grade, apply to every step):**
- A step is "DONE" ONLY with explicit box evidence (`... ok` / `N passed`
  on a run that actually fit in memory). An OOM-died/timed-out run is
  **not** a result.
- "Confidence" tags: **[V]** box-verified this work · **[I]** implemented,
  not yet verified · **[S]** specced/pinned, not implemented ·
  **[X]** not started · **[~]** project-asserted, not re-verified.
- Hardware: the 4 GB Hetzner node-box CANNOT run full-`W` non-native
  synthesis (empirically OOM'd). Full-scale verification → M4 Mini
  cluster / `satyawan-1`. Never run heavy synth on the node-box.

---

## PHASE 0 — Baseline (already done; do not redo)

- **0.1 [V]** S2a — section-bearing trusted-setup shape; S6 determinism
  proof passed (real Nova fixture).
- **0.2 [V]** S2b — mandatory section bindings; full suite GREEN, 5 box
  cycles; section-less circuits provably unprovable.
- **0.3 [V]** S4 curve configs — `GrumpkinConfig` + `ark_bn254::g1::Config`
  pass byte-for-byte cross-library trust gates.
- **0.4 [V]** S4a primitives — `pedersen_msm_grumpkin` gadget +
  `secondary_to_ark_fq` exact converter box-proven.
- **0.5 [V]** S4 de-risk — no in-circuit pairing; non-native field & EC
  gadgets are arkworks-provided. Zero bespoke crypto primitives remain.

---

## PHASE A — Finish S4a secondary binding

- **A.1 [I→verify]** Box-verify the S4b *primitive* proof
  (`s4b_secondary_r1cs_gadget`, tiny 1-row instance) — non-ignored,
  memory-safe. Gate: `secondary_relaxed_r1cs_nn_sat_and_adversarial`
  prints `ok`.
- **A.2 [I→verify]** Box-verify the **bounded-`W`** secondary binding
  (`s4_secondary_extract::secondary_msm_binds_real_prefix`, N=12) on the
  node-box. Gate: in-circuit MSM == out-of-circuit ark MSM over the SAME
  real extracted prefix + adversarial. Depends on: A.1 compiling clean.
- **A.3 [X]** Full-`W` secondary binding (`comm_W == Σ Wᵢ·ckᵢ + r_W·h`,
  unbounded) on a **bigger machine** (M4 Mini / satyawan-1). Gate: real
  `r_U_secondary.comm_W` equals the in-circuit MSM. Depends on: A.2 +
  bigger-box provisioning.

## PHASE B — S4a primary (bn256-G1) analog  *(pinned, mechanical)*

- **B.1 [S]** `pedersen_msm_bn256_g1` gadget — field-inverse of the
  proven Grumpkin one (native `FpVar<Fr>` scalars, non-native
  `EmulatedFpVar<Fq,Fr>` coords) + isolated primitive proof.
- **B.2 [S]** bn256-G1 point decoder + `bn256::Base→ark_bn254::Fq` exact
  converter (analogs of existing proven helpers).
- **B.3 [S]** `extract_primary_*` + bounded + full real-fixture primary
  binding test (`r_U_primary.comm_W == Σ Wᵢ·ckᵢ + r_W·h`) + adversarial.
- Depends on: PHASE A (shared structure must be proven first).

## PHASE C — S4 integration (the actual soundness closure)

- **C.1 [X]** Bind both recomputed MSM points to the **Section-2-bound
  coordinates** inside the verifier circuit — enforce equality, not just
  isolated gadgets. This is where commitment-binding becomes a real
  soundness constraint.
- **C.2 [X]** Surface the **primary** `comm_W` into the transcript
  (currently only secondary is bound — see `S4_DESIGN.md` structural
  note).
- Depends on: PHASE A + B verified.

## PHASE D — S4b: secondary RelaxedR1CS satisfiability *(THE deep one)*

- **D.1 [I]** Non-native row-sat gadget (`enforce_secondary_relaxed_
  r1cs_sat_nn`) — written; primitive proof pending (A.1).
- **D.2 [X]** Secondary R1CS matrix/witness extraction (a
  `section3_witness`-class extractor, secondary side, emulated-Fq).
- **D.3 [X]** Full secondary RelaxedR1CS enforced in-circuit over
  `EmulatedFpVar<Fq,Fr>` (every `(Az)(Bz)==u(Cz)+E` op non-native).
- **D.4 [X]** Bounded box-verify + full verify on bigger box.
- **Effort: genuinely multi-week** — comparable to all PHASE 0 work
  combined; non-native arithmetic is where ZK soundness bugs hide.
- Depends on: A.1 (gadget proof). Largely parallel to B/C.

## PHASE E — S4-verify (S6-analog, adversarial)

- **E.1 [X]** Determinism: `setup_shape()` R1CS still bit-identical with
  ALL S4 constraints present (S6 re-run, extended).
- **E.2 [X]** Adversarial end-to-end: commitment-mismatch AND
  secondary-unsat BOTH rejected, real fixture, bigger box.
- Depends on: PHASE C + D complete.

## PHASE F — S5: MPC trusted-setup ceremony  *(orthogonal B-2 axis)*

- **F.1 [X]** Ceremony design: Powers-of-Tau (or adopt existing) +
  circuit-specific phase 2 over `setup_shape()`; participant set,
  transcript verification, public attestation.
- **F.2 [X]** Run ceremony; replace `#[deprecated] groth16_wrapper::
  setup` insecure path with ceremony-derived params.
- **F.3 [X]** Verify transcript + key correctness.
- **Effort: multi-week, partly NON-engineering** (external participants,
  logistics). Can run in PARALLEL with D/E (independent of S4a/S4b).

## PHASE G — Independent external audit of the *closed* circuit

- **G.1 [X]** Only scopeable after C+D+E+F green. External ZK auditor
  review of the now-sound verifier + ceremony transcript.
- Depends on: E + F complete.

## PHASE H — Broader codebase re-audit  *(separate track — LOW confidence scope)*

- **H.1 [~/X]** Consensus, DA, bridge, execution, EvaporScript: prior
  audit rounds are project-asserted closed but **NOT re-verified in the
  B-1/B-2 work**. An independent re-audit pass is required before mainnet
  and is **out of scope of this flow's verification** — flagged honestly,
  not estimated.

---

## MAINNET EXIT CRITERIA (all must hold)

1. PHASE A–E green with explicit box evidence (full-`W`, not bounded).
2. PHASE F: ceremony-derived keys in place; insecure setup path removed.
3. PHASE G: independent external audit of the closed circuit passed.
4. PHASE H: broader-codebase re-audit passed (separate track).
5. Only then is the ZK `VerkleProofVerifier` eligible to replace the
   interim BLS-quorum path in `Deploy.s.sol`.

## CRITICAL PATH & HONEST TIMELINE

`A → (B,C) → D → E`, with `F` in parallel, then `G`, then `H`.
The **D (S4b) + F (S5) pair is multi-week-to-month** and is the true
gate; everything in B/C is days (pinned/mechanical). **No
evidence-supported timeline shorter than "multi-week before an external
audit can even begin" exists.** Mitigation throughout: the unsound
circuit is **not deployed** (`Deploy.s.sol` = BLS-quorum) — this is a
pre-mainnet rebuild, not a live incident.

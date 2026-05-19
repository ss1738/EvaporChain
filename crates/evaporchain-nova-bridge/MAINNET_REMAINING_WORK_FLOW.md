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

- **A.1 [V]** S4b primitive proof — BOX-VERIFIED on **Mini 1**
  (2026-05-19): `secondary_relaxed_r1cs_nn_sat_and_adversarial ... ok`,
  `1 passed; 0 failed`, 0.31 s, 0 errors. Proves the S4b/D.1 core
  non-native row-sat logic (correct witness satisfies; perturbed row →
  UNSATISFIABLE).
- **A.2 [V]** Bounded-`W` secondary binding — BOX-VERIFIED on **Mini 1**
  (2026-05-19): `secondary_msm_binds_real_prefix ... ok`, `1 passed`,
  34.61 s (real Nova fixture + N=12 non-native MSM genuinely ran), 0
  errors, first run. Extraction decoders + Pedersen-MSM gadget verified
  on REAL extracted `ck`/`W`/`r_W`; in-circuit == out-of-circuit ark MSM
  + adversarial. (Build-host pivoted node-box→Mini 1; node-box stays
  node-only.)
- **A.3 [X — deferred scale-gate, NOT skipped]** Full-`W` secondary
  binding (`r_U_secondary.comm_W == Σ Wᵢ·ckᵢ + r_W·h`, unbounded). This
  is a *scale* verification, not new soundness logic (logic proven by
  A.1+A.2). Run on the largest available host (Mini 1 16 GB attempt;
  satyawan-1 / cluster if 16 GB insufficient — full secondary `W`
  non-native MSM may exceed 16 GB). Sequenced AFTER B/C critical-path
  code (per `A→(B,C)→D` critical path); remains a MAINNET EXIT
  requirement (exit criterion #1: full-`W`, not bounded).

## PHASE B — S4a primary (bn256-G1) analog  *(RECLASSIFIED: DEEP, not mechanical — box-falsified 2026-05-19)*

> **FINDING:** B.1 compile-failed on Mini 1 (15 errors). ark's SW
> `ProjectiveVar<P,F>` is bounded `F: FieldVar<P::BaseField,
> BasePrimeField<P>>`; bn256-G1 has `P::BaseField = Fq`, so it
> demands a field var with constraint field **Fq**. `EmulatedFpVar
> <Fq,Fr>` is `FieldVar<Fq,Fr>` — does NOT satisfy it. **ark
> `ProjectiveVar` supports ONLY native curve coords (base==circuit
> field).** Secondary worked because Grumpkin base=Fr=circuit field.
> **There is no library drop-in for the primary side.** The earlier
> "mechanical/days" estimate is withdrawn.

- **B.0 [V — DECIDED, source-grounded 2026-05-19]** Strategy =
  **Option 1 (bespoke non-native bn256-G1 SW point gadget)**.
  Justification (nova-snark 0.68 `RecursiveSNARK::verify`,
  nova/mod.rs:567–651): verify calls `is_sat_relaxed` on BOTH
  `r_U_primary`(ck_primary) AND `r_U_secondary`(ck_secondary), and
  `is_sat_relaxed` (r1cs/mod.rs:447–474) recomputes
  `U.comm_W==Commit(ck,W)`. The transcript/hash check only absorbs
  `comm_W` coords as field elements — it does NOT verify the MSM
  relation. ∴ **Option 2 (skip primary MSM) is UNSOUND** (permits
  the B-1 forgery: hash one comm_W, R1CS-sat a different W). Option 3
  (wrapper-as-curve-cycle) is far deeper and discards the working
  S2/S3 single circuit. Option 1 is deep (≈ a second S4b) but bounded
  & standard.
- **B.1 [V]** Non-native bn256-G1 SW add/double over
  `EmulatedFpVar<Fq,Fr>` — BOX-VERIFIED on Mini 1 (2026-05-19):
  `nonnative_bn256_g1_double_add_match_ark ... ok`, 1 passed, 0.34 s,
  0 errors. In-circuit 2G/3G == out-of-circuit ark bn256-G1 (generic
  case). The hardest conceptual piece (bespoke non-native foreign-
  curve arithmetic) is proven; EC math correct first try.
- **B.2 [V]** Native-scalar double-and-add ladder over B.1 —
  BOX-VERIFIED Mini 1 (2026-05-19): `nonnative_bn256_g1_scalar_mul_
  matches_ark ... ok` (5·G == ark 5G, real FpVar<Fr> bits) +
  B.1 regression ok, 2 passed, 0.66 s. Generic-case; edge-hardening
  (identity/leading-zeros/degenerate) = documented follow.
- **B.2b [V]** `pedersen_msm_bn256_g1` full primary MSM —
  BOX-VERIFIED Mini 1 (2026-05-19): `nonnative_bn256_g1_msm_matches_
  ark ... ok` (`Σ sᵢ·baseᵢ + r·h = 35G` == ark) + B.1/B.2 regression,
  3 passed, 2.23 s, 0 errors, first run. **The hardest crypto
  obstacle in the whole flow — in-circuit non-native foreign-curve
  MSM — is proven viable.** Remaining for primary: B.2-hardening
  (edge-safe arbitrary-`W` scalars) + the mechanical decoder/converter.
- **B.2-hardening [V]** Edge-safe arbitrary-scalar mul — BOX-VERIFIED
  Mini 1 (2026-05-19): `nonnative_bn256_g1_complete_scalar_mul_
  arbitrary_and_edges ... ok`, 1 passed, **983 s**, 0 errors, first
  run. RCB complete formulas (SW a=0, projective, identity=(0,1,0)),
  exception-free; proven for k=0(→identity)/1/7/large over the full
  254-bit ladder vs ark bn256-G1. **The primary non-native EC stack
  is now soundness-correct AND edge-safe** — no forgeable edge holes.
  SCALE NOTE: 983 s for 4 small scalars ⇒ full-`W` (thousands of
  254-bit scalars) is *extremely* heavy — B.3 must be bounded on
  Mini 1, full-scale = satyawan-1/cluster (same scale-gate class as
  A.3; a MAINNET EXIT requirement, not a logic gap).
- **B.3a [I→verify]** Primary extraction decoders correct on REAL
  fixture data — decode-only (no circuit): `extract_primary_ck/
  witness/comm_w` → decoded points on bn256-G1 + scalars parse.
  Memory-tractable. The genuinely-open question (gadget logic is
  already `[V]`). Mirror of proven `s4_secondary_extract` decoders.
- **B.3b [X — HARD SCALE-GATE]** Full in-circuit complete-formula
  primary binding on real data. EMPIRICALLY OOM-killed (SIGKILL) on
  Mini 1's 16 GB even at N=2 with JSON freed pre-circuit — the RCB
  complete-formula CS for 2 scalars × 254-bit + a real fixture
  exceeds 16 GB. SCALE boundary, NOT a logic gap (primary gadget
  proven `[V]`). Needs a >>16 GB host (satyawan-1/cluster) — same
  class as A.3; MAINNET EXIT requirement, deferred.
- Depends on: B.0 (DECIDED). Primary gadget logic B.1→B.2-hardening
  all `[V]`; only B.3a (cheap, now) + B.3b (scale-gate) remain.

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

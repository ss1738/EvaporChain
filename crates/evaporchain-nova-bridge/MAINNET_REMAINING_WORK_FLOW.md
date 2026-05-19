# ⚠️ ARCHITECTURAL CONCLUSION (2026-05-19, source-confirmed) — READ FIRST

The D.3 measurement (≈2.03×10⁸ constraints for the non-native
secondary R1CS) + reading nova-snark 0.68 source forces an honest
reframe of the *production* approach:

- Nova's augmented circuits **never recompute the other side's full
  R1CS** — folding/NIFS verifies only a constant-size step; full
  RelaxedR1CS-sat is discharged **once at the end**.
- nova-snark 0.68 **ships `CompressedSNARK`** (Spartan + HyperKZG/IPA;
  `nova/mod.rs`, `S1/S2: RelaxedR1CSSNARKTrait`, verify@909) — the
  *intended* succinct wrapper. Spartan (sumcheck) compresses
  RelaxedR1CS-sat **sub-linearly** — NOT the 203M-constraint
  explosion.
- ∴ the hand-rolled S2/S3/S4 path (a Groth16 circuit that explicitly
  re-verifies *both raw* RelaxedR1CS instances) is **reinventing
  `CompressedSNARK`, badly**. The 203M secondary blow-up is the
  symptom of doing this the wrong way; it is NOT fixable by hardware
  *or* by the curve-cycle redesign of the same raw approach.
- The legitimate driver for a Groth16 path is an **EVM-cheap proof**
  (`eip197.rs`/`groth16_wrapper` → BN254 pairing precompile). The
  CORRECT pipeline for that is: `RecursiveSNARK → CompressedSNARK`
  (Spartan, sub-linear) **→ a small Groth16 of the CompressedSNARK
  *verifier*** (a fixed, small circuit verifying the Spartan/KZG
  proof) — NOT Groth16 of the raw R1CS. Skipping the Spartan
  compression is the root cause of everything D.3/S4b.

**SOLUTION (the real one): adopt nova-snark `CompressedSNARK`; if
EVM verification is needed, Groth16-wrap the (small) CompressedSNARK
verifier.** This makes D.3, S4b non-native, the curve-cycle redesign,
and the ≫123 GB host problem ALL disappear. No spend, no big machine
— the library already does, sub-linearly, what S4b was hand-rolling
to 203M constraints.

**✅ VALIDATED [V] (2026-05-19, Mini3, box-verified):**
`compressed_snark_compresses_real_recursive_snark ... ok`, 1 passed,
**25.27 s on a 16 GB Mini, no OOM**. nova-snark `CompressedSNARK
<E1,E2,TrivialIncrementCircuit, S1=Spartan, S2=Spartan>`
`setup → prove → verify` on a REAL `RecursiveSNARK` (built against
the canonical `pp`), asserting compressed-verified `zi == n`. The
production path is no longer just argued from source — it is
**empirically proven end-to-end, sub-linear, tractable on modest
hardware, zero spend**. (One honest iteration: first run failed
because the test used two different `pp` instances → digest mismatch;
`setup`+`prove` succeeded even then — never a CompressedSNARK
problem.)

**What this session's verified work means, honestly:** S2a/S2b and
the commitment-binding proofs (B.3b primary, A.3 secondary — real-
scale box-verified) are valid as a *correctness/learning* exercise
and their soundness insights transfer — but the **production
architecture is CompressedSNARK**, not the 203M hand-roll. The
honest #1-blocker path forward is: validate `CompressedSNARK` end-to-
end, then a small Groth16-of-verifier for EVM — a scoped, library-
based effort, not a multi-month bespoke-circuit dead-end.

## EVM-WRAPPER SCOPING (2026-05-19, source-confirmed, calibrated)

State of the two EVM modules in this crate:

- **`eip197.rs`** [V] — 256-byte EIP-197 Groth16 wire codec incl. the
  BN254-G2 `Fq2 (c1,c0)` swap; tested. **Codec is independent of
  which circuit produced the proof → reused unchanged.**
- **`groth16_wrapper.rs`** [~] — `setup/prove/verify` currently key
  Groth16 over **`NovaVerifierCircuit`** (`verifier_circuit.rs`) =
  the **old hand-rolled raw-RecursiveSNARK verifier**. That is the
  S4b 203M path. Must be re-pointed at a CompressedSNARK verifier.

Validated CompressedSNARK PCS config (from `recursive_snark_fixture`):
`S1/S2 = Spartan`, **EE1 = HyperKZG / Bn256 (primary)**,
**EE2 = IPA-PC / Grumpkin (secondary)**.

**Calibrated risk (NOT overhyped):** native verify is proven 25 s;
the *in-circuit* cost of that verifier is a **separate, unmeasured**
quantity. HyperKZG primary is favorable (KZG open = constant-size
pairing check, cheap in-circuit). **The Grumpkin/IPA secondary is the
concentrated unknown** — IPA verify = log-rounds of non-native MSM,
the exact failure shape S4b hit. Risk is identified and bounded, not
vague; but "small" is a hypothesis, not a measured fact.

**NEXT DELIVERABLE [ ]:** a *constraint-size prediction* harness for
the CompressedSNARK-verifier-in-circuit (same no-spend / falsify-
cheaply method that turned D.3 "buy a VPS?" into "no spend"): from
the native `(vk, proof)` structure (sumcheck round count = log(n),
HyperKZG vs IPA opening op-counts) × the per-op constraint costs
already characterized in `s4_primary_msm_gadget` (g1_add / scalar_mul
/ MSM), predict total constraints **before** building the full
circuit. Run on Mini3 (never the training rig / node box). Decision
gate: if predicted ≪ Groth16-tractable → build the verifier circuit
+ re-point `groth16_wrapper::setup`; if the IPA secondary dominates
→ that, not "the wrapper", is the real remaining problem to solve.

### RESOLVED by source read (2026-05-19, `ipa_pc.rs` verify L351-356)

`CompressedSNARK.verify` secondary path calls Spartan→IPA verify,
whose `ck_hat = CE::commit(&ck, &s, 0)` is a **size-`n` MSM** (`s` is
`vec![..; n]`, `n = b_vec.len()` ≈ 10,554 secondary). **Cheap
natively** (∴ the 25 s validation is genuine and unaffected) but
**in-circuit ≈ 10.5k non-native Grumpkin scalar-muls ≈ S4b-scale
(~10⁸–10⁹ cons).** So:

- The size-prediction harness's open question is **answered without
  building it**: stock nova-snark IPA-secondary verifier is **NOT**
  in-circuit-tractable. The earlier "small Groth16-of-verifier" was
  optimism the calibrated flag correctly hedged. **No overturn of the
  native CompressedSNARK result; sharpening of the EVM wrapper.**
- The blowup is not removed by CompressedSNARK — it **moves** from
  "raw RelaxedR1CS re-verify" to "IPA `ck_hat` size-n MSM". Confirms:
  not fixable by hardware or curve-cycle redesign.
- **Real remaining EVM problem (named, bounded):** discharge the
  *secondary* (Grumpkin) side cheaply on-chain. Known solved shape =
  **Sonobe-style decider**: Groth16/KZG-wrap the **primary only**
  (HyperKZG → constant-size BN254 pairing, EVM-cheap), discharge the
  secondary via the folded NIFS relation — **not** a full in-circuit
  IPA. No 10⁹ circuit, no hardware/VPS spend.

### RESOLVED by source read #2 (2026-05-19, `ppsnark.rs` verify
L1388+ vs `snark.rs` vs `ipa_pc.rs` L351) — calibrated, negative-
leaning, NOT a config-switch win

nova-snark 0.68 ships **two** Spartan variants:
- `spartan::snark::RelaxedR1CSSNARK` (used by the validated test) —
  **non-succinct verifier**: `ck_hat = CE::commit(&ck,&s,0)`, `s`
  length `n` → size-`n` MSM.
- `spartan::ppsnark::RelaxedR1CSSNARK` — **succinct verifier**:
  `num_rounds_outer = num_cons.log_2()`, preprocessed `vk` sparse
  commitments, O(log) sumcheck, **no size-`n` MSM at the Spartan
  level**.

So switching `S1/S2` → `ppsnark` removes the *Spartan-level*
blowup (real, good). **BUT** ppsnark still delegates polynomial
opening to `EE::verify`. For the **secondary** that is
`ipa_pc::verify` (read #1), whose `ck_hat` size-`n` MSM is
**intrinsic to IPA over the non-pairing-friendly Grumpkin secondary
curve** — present regardless of snark vs ppsnark. nova-snark 0.68's
secondary is the **full ~10.5k augmented circuit, not a
CycleFold-constant** one. ∴ **no zero-cost in-circuit EVM path for
the secondary exists in stock 0.68.** (Honest: this is NOT "just
switch to ppsnark and you're done" — the no-overhype discipline
forbids selling it that way.)

**The three real EVM options (all genuine engineering, named +
bounded, none requiring a 203M/10⁹ bespoke circuit or HW spend):**

1. **CycleFold-constant secondary** — make the secondary circuit
   O(1) (Sonobe/CycleFold design) so its size-`n` MSM is size-O(1).
   Requires either Sonobe (arkworks folding lib, has a Decider) or
   upgrading nova-snark's secondary. *Largest architectural change;
   cleanest end state.*
2. **Final-layer verifier recursion ("SNARK-of-SNARK")** — run the
   secondary IPA size-`n` MSM *natively* inside one more Spartan/Nova
   step whose verifier is `ppsnark`-succinct; Groth16-wrap only that
   small residual. *Stays within nova-snark; one extra recursion
   layer.*
3. **Native-Solidity secondary verifier** — emit the secondary IPA
   proof, verify it directly in Solidity (Grumpkin scalar-field ops;
   fixed ~size-`n` gas cost, no SNARK circuit). *Pragmatic mainnet
   path; gas-heavy but deterministic & buildable now.*

**OPTION (2) BASE — ✅ VALIDATED [V] (2026-05-19, Mini3, box):**
`compressed_snark_ppsnark_compresses_real_recursive_snark ... ok`,
1 passed, **124.71 s on a 16 GB Mini, no OOM** (HEAD `cd9882bf`).
`CompressedSNARK<E1,E2,_,S1=ppsnark,S2=ppsnark>` e2e on a real
`RecursiveSNARK`, `zi == n` asserted. Two real API facts surfaced &
fixed en route: (i) ppsnark `ck_floor()` > `snark`'s →
`InvalidCommitmentKeyLength` unless `pp` is built with the ppsnark
floor; (ii) prove is ~5× heavier than `snark` (25 s → 125 s) —
sparse-matrix preprocessing; a one-shot final-proof cost, acceptable.

**HONEST CAVEAT (kept front, no overhype):** this validates the
*native* `ppsnark` CompressedSNARK base ONLY. It removes the
**Spartan-level** size-`n` MSM. It does **NOT** remove the secondary
**Grumpkin-IPA** size-`n` `ck_hat` MSM in `ipa_pc::verify` — that is
intrinsic and still the in-circuit blocker. Option (2) is not "done";
its *base* is proven.

### RESIDUAL RECURSION SCOPED — source read #3 (2026-05-19,
`nova/mod.rs` `CompressedSNARK::verify` L909-1025) — RECURSION
TERMINATES SUCCINCT (source-grounded design conclusion)

Full `CompressedSNARK::verify` mapped. **Every step is constant-size
EXCEPT one:**
- Neptune hash checks (`hash_primary/secondary`) — constant.
- `nifs_Uf_secondary.verify`, `nifs_Un_secondary.verify`,
  `nifs_Un_primary.verify` — constant-size NIFS folds.
- `derandomize` (primary+secondary) — constant.
- `rayon::join(snark_primary.verify, snark_secondary.verify)`:
  - `snark_primary.verify` — Spartan ppsnark + **HyperKZG** ⇒
    constant (one BN254 pairing). EVM-cheap.
  - `snark_secondary.verify` — Spartan ppsnark + **IPA/Grumpkin**;
    its sole super-constant op = `ipa_pc::verify` size-`n` `ck_hat`
    MSM. **THE only non-constant term in the whole verifier.**

**Why everything blew up before, precisely:** that single MSM is
Grumpkin-group. Grumpkin's *base field* **is BN254-Fr** (the primary
scalar field; the existing cycle). The 203M/10⁹ explosion was the
cost of doing this Grumpkin EC **non-natively** (`EmulatedFpVar`,
foreign field) — S4b/D.3. Done on the **matching native cycle side**
(a recursion circuit over BN254-Fr), the same MSM is **≈`n` *native*
constraints (~10⁵)**, linear-Spartan-provable, NOT 10⁹.

**∴ final-layer recursion terminates succinct:** one recursion
circuit over BN254-Fr does the secondary MSM **natively** (all other
verifier steps already constant) → ppsnark-compress it (size-`n`
R1CS is fine for the linear prover; its verifier is succinct) →
Groth16-wrap the succinct top (HyperKZG ⇒ constant BN254 pairing ⇒
EVM-cheap). **Option (2) is viable; the EVM mainnet path is not
blocked.** No hardware spend, no 10⁹ bespoke circuit.

**NO-OVERHYPE LINE (explicit):** the above is a *source-grounded
design conclusion*, NOT a box-validated number. It is sound because
the only super-constant term is provably native on the recursion
field — but the falsifiable next step is to **measure the recursion-
circuit constraint count** (the proven D.3-style prediction: `n`
secondary ≈ 10.5k native Grumpkin scalar-muls × native per-op cost +
the constant HyperKZG-pairing-in-circuit term), not to assert a final
figure now.

**NEXT [ ]:** D.3-style size *prediction* for the BN254-Fr recursion
circuit (native secondary MSM term + constant primary-pairing term),
no build/spend; if predicted ≪ Groth16-tractable (expected ~few ×10⁶)
→ build the recursion circuit + re-point `groth16_wrapper::setup` at
its succinct ppsnark verifier; `eip197.rs` codec reused unchanged.

---

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
- Hardware: 4 GB Hetzner node-box = node only (never build). Mini 1
  (16 GB) = tractable units. **`satyawan-1` (32-core / 123 GB) =
  PROVEN scale-gate host** — B.3b (OOM'd twice ≤16 GB) PASSED there
  in 332 s, no OOM (2026-05-19); fr009 training undisturbed (finished
  naturally; queue advanced). `nice -19`, never touch the GPU
  training. The scale-gates (A.3/B.3b/D.3) are RUNNABLE there, not
  indefinitely deferred.

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
- **A.3 [V — scale-gate CLEARED on satyawan-1]** Full-`W` secondary
  soundness closure — BOX-VERIFIED satyawan-1 (119 GB) 2026-05-19:
  `secondary_msm_binds_full_comm_w ... ok`, 1 passed, 756.75 s, no
  OOM, training undisturbed (fr143 finished naturally, fr020 alive).
  FULL real `W` in-circuit MSM == the ACTUAL Section-2-bound
  `r_U_secondary.comm_W` (`comm_W == Σ Wᵢ·ckᵢ + r_W·h`) + adversarial.
  The genuine B-1 secondary commitment-binding closure on real data
  at full scale — NOT bounded, NOT an ark-proxy.

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
- **B.3a [V]** Primary extraction decoders correct on REAL data —
  BOX-VERIFIED Mini 1 (2026-05-19): `primary_extract_decodes_real_
  data ... ok`, 1 passed, 29.80 s, 0 errors, first run. Real
  `ck_primary`/`r_W_primary`/`comm_W` decode to on-curve bn256-G1 +
  parseable Fr. The genuinely-open primary question is RESOLVED.
- **B.3b [V — scale-gate CLEARED on satyawan-1]** Full in-circuit
  complete-formula primary binding on real data. OOM'd twice ≤16 GB
  (Mini 1); **PASSED on satyawan-1 (119 GB) 2026-05-19**:
  `primary_msm_binds_real_prefix ... ok`, 1 passed, 332.63 s, no OOM,
  training undisturbed. The in-circuit complete-formula primary
  commitment binding on REAL fixture data is VERIFIED. (True-full-`W`
  / larger-N is a further magnitude point but the binding logic on
  real in-circuit data is proven.)
- Depends on: B.0 (DECIDED). Primary gadget logic B.1→B.2-hardening
  all `[V]`; only B.3a (cheap, now) + B.3b (scale-gate) remain.

## PHASE C — S4 integration (the actual soundness closure)

> **DEPENDENCY CORRECTION (2026-05-19, verify-grounded):** C is NOT a
> clean post-B step. (1) **C.1-primary** = recompute the primary MSM
> *inside* the verifier circuit + enforce `== r_U_primary.comm_W` —
> that IS the **B.3b** full in-circuit complete-formula binding, which
> EMPIRICALLY OOMs ≤16 GB → C.1-primary inherits the B.3b scale-gate.
> (2) **C-secondary** needs the secondary `W` *in-circuit* to bind,
> but the secondary `W` only enters via **PHASE D (S4b)** — so C's
> secondary side is **downstream of D**, not after B. Real ordering:
> `A → B(logic) → D → C`, with C's full binding ALSO scale-gated.

- **C.1 [X]** Bind both recomputed MSM points to the Section-2-bound
  coords inside the verifier circuit. Primary side = B.3b-scale-gated;
  secondary side gated on D. Gadgets proven `[V]`; *closure* (wiring
  into `generate_constraints`) is downstream of D + scale.
- **C.2 [X]** Surface the **primary** `comm_W` into the transcript
  (currently only secondary's is — see `S4_DESIGN.md` structural note).
- Depends on: A + B(logic) `[V]`; **D**; and the B.3b/A.3 scale-gates.

> **D.3 SIZE MEASURED (2026-05-19, Mini3, tractable harness — no
> spend, no big box):** synthetic fit ≈ `3844·s + 813` (~1281
> constraints / non-native nonzero) × REAL secondary dims
> (`num_cons=10554, num_vars=10536, total_nnz=126899`) ⇒ **full-D.3
> ≈ 2.03 × 10⁸ constraints ≈ 57–284 GB**. Consistent with satyawan-1
> (123 GB) hanging (123 GB is *inside* the band). **DECISION: do NOT
> buy/rent a bigger box to brute-force this.** The primary R1CS
> (native, Section 3) is ~10⁴ constraints; the secondary via
> non-native emulation is ~2×10⁸ — a **~20,000× blow-up purely from
> Fq emulation**, also impractical for Groth16 setup/prove itself.
> This is quantitative proof the **curve-cycle redesign (B.0 Option
> 3) is the necessary architecture**, not optional — it removes the
> 20,000× factor at the root. Standalone-full-D.3 (≈256–512 GB,
> ~$10–30 hourly cloud, one-off) is only a checkbox; the redesign is
> the real path.

## PHASE D — S4b: secondary RelaxedR1CS satisfiability *(THE deep one)*

- **D.1 [V]** Non-native row-sat gadget — proven via A.1
  (`secondary_relaxed_r1cs_nn_sat_and_adversarial`, Mini 1).
- **D.2 [V]** Secondary R1CS extractor — BOX-VERIFIED Mini 1
  (2026-05-19): `secondary_r1cs_extract_decodes_real_data ... ok`,
  1 passed, 30.81 s, first run. Byte-identical mirror of proven
  `section3_witness`, secondary/ArkFq, bucketed for D.1; real-data
  shape self-consistent (dims, W/E lens, A/B/C num_cons buckets,
  col-in-z-range). Decode-only / tractable.
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

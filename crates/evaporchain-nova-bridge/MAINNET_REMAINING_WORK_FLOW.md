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

### SIZE PREDICTION — ✅ MEASURED [V] (2026-05-19, Mini3, box,
HEAD `c217dce2`, `predict_native_grumpkin_msm_size_for_recursion_
circuit ... ok`, 1 passed, 1.08 s)

Real `cs.num_constraints()` of the existing native-Grumpkin
`pedersen_msm_grumpkin` gadget (points = BN254-Fr coords = native;
scalars = non-native Fq):

| k | cons |
|---|---|
| 1 | 5,054 |
| 2 | 7,587 |
| 4 | 12,653 |
| 8 | 22,785 |

Linear fit: **2,533 cons / MSM term**, intercept 2,521. **Predicted
at n=10,554: MSM ≈ 26.7M + HyperKZG-pairing const 2M ≈ TOTAL
~28.7M constraints. FALSIFIER (≥1e9) DID NOT FIRE.**

**Verdict (calibrated, no overhype):**
- ✅ ~29M ≪ D.3's 2.03×10⁸ ≪ 1e9. The native-side placement gives
  **~7× under D.3**; source-read-#3 "recursion escapes S4b scale" is
  **empirically supported**, not just argued. The earlier flow text
  "(~10⁵)" was optimistic — corrected: the measured figure is ~10⁷
  (flat MSM), still well inside the conclusion.
- ⚠️ ~29M is **tractable but NOT "small"**: Groth16 needs ~2²⁵
  powers-of-tau, tens-of-GB prover RAM, minutes-scale prove on a
  strong box — a **one-shot per-proof prover cost**. EVM **verify is
  constant** (3 pairings ~250k gas) regardless — that part is cheap.
- ~29M is a **conservative upper bound**: probe uses the *flat*
  per-term MSM (n independent scalar-muls). IPA `ck_hat`'s scalar
  vector has **tensor structure** (`ipa_pc.rs` L334-349) a bespoke
  recursion circuit folds in ~log(n) → potential ~10–100× cut. Lever,
  not requirement.

**Bottom line:** Option (2) is the **validated EVM path** — buildable,
no 10⁹ circuit, no forced HW spend, EVM verify cheap; prover-side
~29M is heavy-but-one-shot and tensor-foldable. The B-1/B-2 #1
mainnet-blocker architecture question is **resolved**.

### BUILD INCREMENT 1 — Section A LIVE ✅ [V] (2026-05-19, Mini3,
box, HEAD `67625605`)

New module `recursion_decider_circuit.rs`:
`ConstraintSynthesizer<Bn254Fr>`. **Section A** (secondary IPA
`ck_hat` MSM — the dominant ~26.7M term) recomputes
`Σ sᵢ·ckᵢ + r·h` natively via `pedersen_msm_grumpkin` and
`enforce_equal`s the claimed commitment. Box-verified
(`recursion_decider_circuit ... 3 passed; 0 failed; 0.27 s`):
- `section_a_correct_commitment_satisfies_cs` — correct ⇒ CS sat.
- `section_a_wrong_commitment_breaks_cs` — wrong ⇒ CS **UNSAT**
  (binding is **non-vacuous** — the exact B-1 `dummy()`-vacuity
  hazard, proven avoided).
- `section_a_length_mismatch_is_unsatisfiable` — malformed ⇒
  `Unsatisfiable` (crate-wide contract).

**Scope boundary (explicit, no overhype):** this proves Section A
*logic* is correct and non-vacuous at **small controlled scale (3
bases)**. It is NOT yet: wired to a real `CompressedSNARK<ppsnark>`
proof's secondary instance; run at n=10,554; nor are Sections B-D
(constant-size Neptune/NIFS/HyperKZG) wired — they are explicit
deferred stubs and `sections_bcd_wired:false` records that. "Section
A logic proven", not "the decider works".

### PREMISE-CHECK — real-proof access path scoped GREEN +
JUSTIFIED REORDER (2026-05-19, source read #4, no spend)

- `CompressedSNARK` derives `Serialize/Deserialize` (nova/mod.rs:319);
  `RelaxedR1CSInstance` and the `ppsnark` proof are serde too. The
  crate ALREADY extracts secondary data this way
  (`dump_ck_secondary_shape`, `s4b_secondary_r1cs_extract`,
  `s4_secondary_extract` parse `serde_json::to_value(pp)`). **No
  private-field blocker — access path GREEN.**
- Honest nuance: Section-A `ck_hat` inputs are NOT raw serde fields.
  `s` is the tensor vector the IPA verifier *derives from
  Fiat-Shamir challenges* (`ipa_pc.rs` L334-349); `ck` =
  `pp.ck_secondary`; claimed `ck_hat` = reconstructed key. The
  adapter must **replay that deterministic derivation** (challenges
  from serde-readable `L_vec/R_vec` via the transcript), not
  field-read it. Defined, deterministic, buildable, no spend.
- **REORDER (flagged, not silent):** the original "increment 2 = B-D
  then increment 3 = adapter" is inverted. The adapter premise is
  now confirmed viable and MUST precede B-D so every section is
  keyed over **real** proof data — never synthetic shapes (the B-1
  `dummy()`-vacuity hazard this whole effort exists to prevent).

**NEXT [ ]:** increment 2 (was 3) — build the real-proof adapter:
generate a `CompressedSNARK<ppsnark>` proof (the validated
`cd9882bf` path), serde-extract the secondary instance + `L_vec/
R_vec`, replay `ipa_pc::verify`'s transcript+`s`-derivation
(L294-356) OUT of circuit to produce the real `(s, ck, ck_hat)`
Section-A witness, and box-verify `RecursionDeciderCircuit` Section A
against it at real n (CS satisfied + non-vacuous, on Mini3). THEN
increment 3 — wire constant Sections B-D against the same real proof
+ flip `sections_bcd_wired`. Heavy 29M Groth16 prove + flat-vs-tensor
MSM decision stays deliberately scheduled (satyawan-1 / a Mini —
never the training rig / node box).

### INCREMENT-2 KERNEL — `ipa_s_tensor` ✅ [V] (2026-05-19, Mini3,
box, HEAD `e48f0386`, `ipa_s_tensor ... 4 passed; 0 failed; 0.24 s`)

`ipa_s_tensor::ipa_s_vector` = bit-exact port of `ipa_pc::verify`'s
`s` derivation (`provider/ipa_pc.rs` L334-349). This is the subtle,
risk-carrying piece of the real-proof adapter: a wrong index /
exponent / round-reversal silently yields a different MSM ⇒ a
vacuous-yet-passing binding (the B-1 hazard). **Falsified
INDEPENDENTLY:** `Σ sᵢ·ckᵢ` (tensor path) == the literal recursive
`ck.fold` (`pedersen.rs::fold` L487 weights `(r⁻¹,r)` + `ipa_pc`
prove loop) at n=8/16/64, plus `s[0]=Πr⁻¹` spot-check. Two unrelated
code paths converge ⇒ the port is faithful to nova-snark.

### INCREMENT 2 COMPLETE ✅ [V] (2026-05-19, Mini3, box, HEAD
`a356fd55`, `section_a_real_bases_real_tensor_pipeline ... ok`,
1 passed, 50.94 s)

End-to-end **real-data** witness-assembly pipeline verified:
`canonical_public_params` → `extract_secondary_ck` (real Grumpkin
bases) → `ipa_s_vector` (real tensor) → real `ck_hat` →
`RecursionDeciderCircuit` Section A. Positive (CS sat) AND
non-vacuous negative (tamper ⇒ UNSAT) on **real curve points** at
n=256. 50.94 s / ~0.65M cons is consistent with the measured
2,533-cons/term linear fit — the size model holds on real bases.

**Scope boundary (explicit):** real `ck` + real tensor structure,
challenges `r` synthetic-but-valid, n=256. NOT yet: `r` bound to a
*specific* `CompressedSNARK<ppsnark>` proof's `L_vec/R_vec`
transcript; full n≈16384 (~41M-cons) synthesis; Sections B-D. Those
are increment 3 + the deliberately-scheduled heavy step.

### INCREMENT-3(a) PREMISE-CHECK ✅ [V] + ⚠️ MATERIAL SIZE
CORRECTION (2026-05-19, Mini3, box, HEAD `7a9857d6`,
`dump_compressed_ppsnark_proof_structure ... ok`, 1 passed,
123.80 s)

**GREEN — serde path pinned:** secondary IPA args extract at
`compressed.snark_secondary.eval_arg.{L_vec, R_vec, a_hat}`;
`CMP_SERDE_ROUNDTRIP = ok` (lossless). Primary `eval_arg =
{com,v,w}` (HyperKZG, constant — the cheap side, confirmed). Top
keys: `snark_primary/secondary, r_U_*, l_u_secondary, nifs_*` — all
serde-reachable.

**⚠️ CALIBRATION CORRECTION (no overhype — the evidence walks back
the earlier optimism):** `L_vec.len() = 17` ⇒ **real n = 2¹⁷ =
131,072**, NOT the ~10,554 (≈2¹⁴) the D.3 `num_cons` implied.
ppsnark's `S_comm.N` padding sets the IPA vector length. Revised
flat-MSM size: **131,072 × 2,533 ≈ ~332M + 2M HyperKZG ≈ ~334M
constraints.**
- Still ≪ 1e9 → the *architecture* conclusion holds (recursion
  terminates, finite, native-not-non-native; falsifier intact).
- BUT ~334M is **~1.6× LARGER than D.3's 2.03×10⁸**, NOT 7× smaller.
  The prior "~29M / ~7× under D.3" headline was an artifact of
  using too-small an n; **flat-MSM at the real n does NOT beat the
  blow-up it was meant to beat.** Correcting that claim explicitly.
- ∴ the tensor-fold — previously logged as an *optional* "~10-100×
  lever held in reserve" — is **NOW MANDATORY**. At n=131,072 the
  tensor-folded `ck_hat` (≈2n native point-adds + 17 scalar-muls ≈
  low single-digit M) is the *only* Groth16-tractable path. The
  flat `pedersen_msm_grumpkin` that `RecursionDeciderCircuit`
  Section A currently uses is **NOT viable at real scale** — it is
  correct (box-verified) but must be replaced by the tensor-folded
  form for the real circuit.

### TENSOR-FOLD DECISION PROBE — ❌ OPTION (2) DEAD-END AT REAL n
[V] (2026-05-19, Mini3, box, HEAD `8dd9745b`,
`ipa_ck_fold_gadget ... 2 passed; 0 failed; 6.06 s`)

`fold_matches_native_recursive_fold ... ok` (in-circuit fold ==
verified tensor-`s` MSM — correctness closed). **`FOLD_PROBE`:
n4:16236 n8:34037 n16:66486 n32:128231 ⇒ A_fold≈4000/n;
fold_pred@131072 ≈ 524M vs flat_pred ≈ 332M, ratio = 1.579.**

**Verdict — clean measured negative (no spin):**
- The recursive fold is **~1.58× WORSE than flat**, not the
  "10-100× lever / low single-digit M" two prior turns asserted.
  That optimism is now **doubly falsified** — by structural
  analysis AND by box measurement. Logging it as a wrong call.
- ∴ **option (2) (final-layer recursion, secondary IPA `ck_hat`
  in-circuit, flat OR fold) is a DEAD-END for the practical EVM
  path.** Flat ≈332M / fold ≈524M: both ≪1e9 (so the *architecture*
  claim "terminates finite, native-not-non-native" still literally
  holds and is not retracted) but **both far beyond practically
  provable Groth16** (~3-5×10⁸ ⇒ ~2²⁹ SRS, hundreds of GB — not
  feasible on satyawan-1 or a Mini).
- Root cause: nova-snark 0.68's **secondary is the full ~2¹⁷
  augmented circuit** with IPA over non-pairing Grumpkin. No
  in-circuit re-expression of a size-2¹⁷ MSM is Groth16-cheap.

**This is now an ARCHITECTURE decision (Satyawan's call — it
changes the mainnet ZK design, protocol-layer he owns), not a code
increment. The honest remaining options, costs un-sugar-coated:**

1. **CycleFold-constant secondary** — the principled fix: a folding
   scheme whose secondary circuit is O(1) (single EC scalar-mul),
   so the size-n MSM becomes size-O(1) and a small Groth16 decider
   works. Cost: adopt Sonobe (arkworks folding lib w/ Decider) OR
   fork/upgrade nova-snark's secondary. Largest change; correct end
   state; the EvaporChain mainnet proof system would be rebuilt on
   it. (Native CompressedSNARK for non-EVM use is unaffected.)
2. **Native-Solidity secondary verifier** — emit the secondary IPA
   proof, verify in Solidity. Honest cost: a size-2¹⁷ Grumpkin MSM
   on-chain ≈ likely millions–tens-of-millions of gas per opening —
   probably impractical for L1 mainnet too. Buildable now but may
   not be economically viable; needs a gas estimate before commit.
3. **Secondary off the critical EVM path** — aggregate/recurse many
   proofs so the per-proof secondary cost amortizes, or
   fraud-proof/optimistic the secondary. Design-level; defers not
   removes the problem.

Salvaged & reusable regardless of choice: `ipa_s_tensor` (faithful,
falsified), `RecursionDeciderCircuit` Section-A pattern + non-vacuity
discipline, the serde-extraction path, the size-probe methodology,
and the confirmed fact that the **primary (HyperKZG) side is
constant/EVM-cheap** — only the secondary is the blocker.

### OPTION-(2)-SOLIDITY GAS ESTIMATE — ❌ ELIMINATED (2026-05-19,
analytical, order-of-magnitude decisive, no spend)

Grumpkin is NOT an EVM precompile curve (BN254 precompiles
0x06/07/08 do not apply). All Grumpkin ops run in pure-EVM
`MULMOD`-based field arithmetic. Anchors: 256-bit modmul ≈ 50-100
gas effective; Grumpkin add/double ≈ ~1-2k gas; one 256-bit
scalar-mul ≈ ~300-575k gas. The secondary IPA verify's unavoidable
core `ck_hat = ⟨s, ck⟩` is a **size-n=131,072 Grumpkin MSM**:
Pippenger-best ≈ ~n adds ≈ **~2×10⁸ gas**; naive ≈ ~10⁹⁺. Ethereum
block/tx gas limit ≈ **3×10⁷**.

**∴ option (2)-Solidity is ~7×–200× over the block limit —
ELIMINATED at order-of-magnitude (no Foundry harness needed; a
precise measurement would only pin an already-conclusive number).
BOTH forms of option (2) (in-circuit Groth16 ~3-5×10⁸ cons AND
native-Solidity ~10⁸-10⁹ gas) are measured-dead at real n=2¹⁷.**

The choice now cleanly narrows to **(1) CycleFold-constant
secondary** vs **(3) secondary-off-critical-path**. Recommendation
(for Satyawan's architecture call): **(1) CycleFold** — it is the
ecosystem's known-correct answer to *exactly* this problem (Nova
secondary too big for EVM); CycleFold makes the secondary a single
O(1) EC scalar-mul, after which a small Groth16 decider + the
already-confirmed constant HyperKZG primary give a genuinely
EVM-cheap proof. (3) is a workaround that adds protocol surface
(fraud/aggregation) without fixing the core. Key sub-decision inside
(1): adopt **Sonobe** (arkworks folding lib, ships a Decider +
CycleFold — migrate EvaporChain's Nova usage to its API) vs
**implement CycleFold around nova-snark** (keep current API, larger
crypto build). Solo-build-protocol-layer preference applies — his
call.

**NEXT [decision, not code]:** Satyawan picks (1) vs (3), and if
(1), Sonobe-adopt vs nova-snark-CycleFold. Do NOT write more code on
the B-1/B-2 circuit path until that call — both option-(2) branches
are measured dead-ends.

### SONOBE PREMISE-CHECK — ⚠️ FRAMING CORRECTION (2026-05-19,
README/Cargo.toml fetched, no spend)

**Architectural fit confirmed (the part of my earlier framing that
held):** Sonobe ships Nova + CycleFold + DeciderEth (Groth16) + a
`solidity-verifiers` crate generating the Solidity EVM verifier
("Currently only supports Nova+CycleFold DeciderEth proofs"). Deps
include `ark-bn254` + `ark-grumpkin` — the cycle is native. Frontend
is `FCircuit` (arkworks-native), clean migration target. PSE +
0xPARC built it — ecosystem-aligned.

**⚠️ Correction I must surface (the part I overstated):** README
verbatim — *"experimental code, do not use in production. The code
has not been audited."* Version `0.1.0`, pre-1.0. Refactor split:
`dev` (latest) → `staging` (revamped Nova+CycleFold being PREPARED
for audit) → `main` (older). Earlier "audit-attention, production-
trodden" was wrong by half — audit is *being prepared*, not done.
Per the just-persisted assert-without-measuring lesson, correcting
this explicitly rather than burying it.

**Honest landing patterns (un-sugar-coated):**
- (1A) Track Sonobe staging; pin after their audit completes.
  Build EvaporChain bridge + step circuits against Sonobe's API now
  so post-audit migration is mechanical. Timeline-dependent on PSE.
- (1B) Adopt Sonobe staging now + commission/run our own audit on
  the pinned revision (or do a deep review). Faster ship; real
  audit-spend item.
- (1C) Switch to implement-CycleFold-around-nova-snark after all
  (you bear the crypto risk; nobody else has reviewed it either, but
  stays in your solo-build column).
- (3) Pause B-1/B-2; revisit when Sonobe's audited release lands.

### SONOBE STAGING PROBE #2 — ⚠️ `solidity-verifiers` NOT ON
STAGING (2026-05-19, no spend; the third honest correction this
arc, per the assert-without-measuring lesson)

Probed (HTTP 200/404):
- `crates/ivc/src/lib.rs` → **200** (Nova+CycleFold IVC on staging
  ✓).
- `solidity-verifiers/Cargo.toml` → 404.
- `crates/solidity-verifiers/Cargo.toml` → 404.

Workspace `[primitives, fs, ivc]`. The `solidity-verifiers` crate
(EVM Solidity verifier templater for DeciderEth proofs — the
last-mile piece that makes mainnet EVM verification work) **exists
only on `main` (pre-refactor layout, ark 0.5) and has not been
ported to the staging refactor yet**. Staging head pin candidate:
`3a86594ec6081bdc8050cbaa1fb7389fb8d37c46` (winderica 2026-05-17).
ark workspace: 0.6.0; edition 2024 / rust 1.85.1.

**Implication for 1B:** pinning staging gets Nova+CycleFold+DeciderEth
proof generation but NOT the EVM Solidity templater. Honest sub-paths:
- **1B-α**: pin staging + port `solidity-verifiers` main→staging
  ourselves (bounded; goes into our audit scope alongside the rest).
- **1B-β**: pin staging + author our own DeciderEth→Solidity
  templater (no PSE dependency for templating; tighter control;
  more work).
- **1B-γ**: hybrid staging-IVC + main-solidity-verifiers (two ark
  versions in workspace; type-identity hell; likely impractical).

Plus the ark 0.5 → 0.6 bump of the EvaporChain bridge (touches
s4_msm_gadget, grumpkin_config, ipa_s_tensor, recursion_decider_
circuit) is required either way.

### ARCHITECTURE LOCKED — 1C: nova-snark + custom CycleFold (solo
build) (2026-05-19, Satyawan's call)

**Why:** Sonobe-state friction was accumulating — audit not yet done
(staging refactor in progress), `solidity-verifiers` not on staging,
ark 0.5→0.6 bump required anyway. Pivot keeps us on the **validated
nova-snark 0.68 substrate** (25 s `snark` + 125 s `ppsnark`
`CompressedSNARK` e2e tests both pass on Mini3) and aligns with the
EvaporChain protocol-layer solo-build preference. All 25 commits of
salvaged work apply.

**Calibrated cost picture (no overhype, per the lesson):** CycleFold
makes the auxiliary (secondary) circuit ~one in-circuit EC
scalar-mul check (~few-k R1CS, independent of step circuit size —
THE architectural win). But ppsnark padding `S_comm.N ≈
next_pow2(max(total_nz, 2·num_vars, num_cons))` still gives
secondary IPA opening n ≈ **8-16k**, not "constant ~hundreds." So
the EVM cost is **~12-25M gas** for the on-chain IPA MSM (Pippenger,
~1.5k gas/add — Solidity, since Grumpkin is not a precompile curve):
**in the L1 block ~30M zone, cheap on L2**. "Constant" in CycleFold
means *independent of step circuit size*, not *trivially small in
absolute terms*. Gas needs measurement at the chosen n, not
assertion. *(Architecture viable; absolute gas TBD.)*

**Reusable assets (the 25-commit arc was not wasted):**
`ipa_s_tensor` (bit-faithful, falsified), `pedersen_msm_grumpkin`
(~2.5k cons/term — the auxiliary scalar-mul building block;
characterized), `grumpkin_config`, `s4_msm_gadget`,
`extract_secondary_ck`, `RecursionDeciderCircuit` Section-A
non-vacuity pattern + tests, HyperKZG-primary-cheap finding (primary
side untouched by the pivot), size-probe methodology. The 1C build
re-uses these wholesale.

**Build roadmap (sequential increments, each box-verified before
the next, never the training rig / node box):**
1. **Aux circuit core**: an `FCircuit`-style step that does *one*
   in-circuit Grumpkin scalar-mul check + an out-of-circuit oracle
   for the expected output. Measure `cs.num_constraints()` (cheapest
   decisive test; expect ~few-k from `pedersen_msm_grumpkin` k=1).
2. **CycleFold instance shape**: define the auxiliary R1CS the
   primary commits to per step. Wire its (small, constant-size)
   instance into the IVC harness on top of nova-snark's
   NIFS/transcript primitives.
3. **Primary augmented circuit**: step + RO update + fold of two
   CycleFold instances (no cross-curve EC scalar-mul in the
   primary; the auxiliary discharges it).
4. **Cycle plumbing**: IVC step driver that maintains primary
   running instance + auxiliary running instance + transcript.
5. **Decider**: `CompressedSNARK<ppsnark>` over the resulting
   small-secondary running instance; box-validate (analogue of the
   125 s ppsnark e2e). MEASURE real n_aux post-CycleFold from the
   real proof's `L_vec.len()` (the lesson — pin n from a real
   artifact, not from estimates).
6. **Solidity verifier**: author DeciderEth → Solidity templater
   for EvaporChain's exact decider shape (primary HyperKZG +
   small-n IPA secondary). Measure on-chain gas via Foundry.
7. **Audit prep**: scope review of the CycleFold construction
   (paper + correctness proofs); EvaporChain-owned, no external
   library dependency.

**NEXT [code]:** increment 1 — aux circuit core + cs probe.
Bounded, box-verifiable, reuses `pedersen_msm_grumpkin` k=1 surface.

### 1C INCREMENT 1 — ✅ AUX CORE [V] (2026-05-19, Mini3, box, HEAD
`01add2da`, `cyclefold_aux_circuit ... 3 passed; 0 failed; 0.10 s`)

New module `cyclefold_aux_circuit.rs`: native BN254-G1 ops over a
**Bn254Fq** constraint system (`ark_bn254::constraints::GVar` —
enabled the `r1cs` feature) + non-native `EmulatedFpVar<Bn254Fr,
Bn254Fq>` scalar = the mirror image of `s4_msm_gadget`. Gadget
verifies `Q = s·P` for the folding scalar × E1 commitment — the
single load-bearing op of the CycleFold auxiliary circuit. Three box
tests: `aux_scalar_mul_matches_native ... ok` (correctness vs ark),
`aux_scalar_mul_wrong_expected_breaks_cs ... ok` (non-vacuity, the
B-1 guard), `aux_scalar_mul_size_probe ... ok`.

**AUX_PROBE: cs.num_constraints = 2,548, witness = 2,375.**

**Predicted (NOT asserted — to be measured at increments 5/6):**
- ppsnark padding `S_comm.N = next_pow2(max(total_nz, 2·num_vars,
  num_cons))` ≈ `next_pow2(2·2375)` = **8,192 = 2¹³**.
- IPA opening `n_aux ≈ 8,192` — a **16× reduction from the
  option-(2) dead-end 2¹⁷=131,072**.
- Solidity Pippenger gas at n=8,192 ≈ **~12M gas** (~40% of L1
  block; cheap on L2).
- The CycleFold reduction looks viable on this number; the real
  end-to-end n_aux + gas wait for the real proof + Foundry, per the
  assert-without-measuring lesson.

**NEXT [code]:** increment 2 — define the CycleFold instance shape
the primary commits to per step (a small R1CS instance wrapping
this aux check, with the primary's NIFS challenge as public input).
Wire its commitment+folding into the existing nova-snark
NIFS/transcript primitives. Bounded; uses the measured ~2.5k-cons
shape as the spec.

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

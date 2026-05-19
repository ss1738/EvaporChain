//! Audit B-1/B-2 S4a PHASE B — primary (bn256-G1) MSM.
//!
//! ## FINDING (2026-05-19, box-falsified — do not regress)
//!
//! The S4_DESIGN assumption that the primary MSM is "mechanical
//! `ProjectiveVar` reuse with `EmulatedFpVar` coords" is **WRONG**.
//! Empirically (Mini 1 compile, 15 errors):
//!
//! `ark_r1cs_std::groups::curves::short_weierstrass::ProjectiveVar
//! <P, F>` is bounded `F: FieldVar<P::BaseField, BasePrimeField<P>>`.
//! For bn256-G1, `P::BaseField = ark_bn254::Fq` and
//! `BasePrimeField<P> = ark_bn254::Fq`, so the gadget REQUIRES a
//! field var whose constraint field is **Fq**. `EmulatedFpVar<Fq,
//! Fr>` is `FieldVar<Fq, Fr>` (constraint field Fr) — it does NOT
//! satisfy `FieldVar<Fq, Fq>`. **ark's SW `ProjectiveVar` only
//! supports the NATIVE case** (curve base field == circuit field).
//!
//! Consequence: the secondary side (Grumpkin, base field = Fr =
//! circuit field) is native and `ProjectiveVar`-reusable (proven:
//! `s4_msm_gadget`, A.1/A.2 [V]). The **primary side is NOT** — bn256
//! G1 coordinates are Fq, foreign to the BN254-Fr circuit. There is
//! no library drop-in.
//!
//! ## Options (require a design decision, NOT a code patch)
//!
//! 1. **Bespoke non-native SW point gadget**: implement G1 add/double
//!    and the scalar-mul ladder explicitly over `EmulatedFpVar<Fq,
//!    Fr>` (incomplete-formula safe). This is ~S4b-class depth — the
//!    primary side is therefore NOT "days/mechanical"; it is deep.
//! 2. **Avoid in-circuit bn256-G1 arithmetic**: bind the primary
//!    `comm_W` through a different mechanism (e.g. via the transcript
//!    hash already absorbing `comm_W.{x,y}` as Fq-derived field
//!    elements) so no in-circuit primary EC MSM is needed. Requires
//!    re-deriving exactly what soundness property the primary binding
//!    must enforce vs. what S2/S3 already cover.
//! 3. **Curve-cycle trick**: perform primary-instance checks in the
//!    secondary circuit of the cycle where bn256 scalars are native —
//!    a deeper Nova-folding-aware redesign.
//!
//! ## B.0 DECISION (2026-05-19, source-grounded) — Option 1
//!
//! `RecursiveSNARK::verify` (nova-snark 0.68 nova/mod.rs:567–651)
//! calls `is_sat_relaxed` on BOTH `r_U_primary`(ck_primary) AND
//! `r_U_secondary`(ck_secondary); `is_sat_relaxed`
//! (r1cs/mod.rs:447–474) recomputes `U.comm_W == Commit(ck, W)`. The
//! verify hash-check only absorbs `comm_W` coords as field elements —
//! it does NOT verify the MSM relation. ∴ **Option 2 (skip the
//! primary MSM) is UNSOUND** (permits the B-1 forgery). Option 3
//! (wrapper-as-curve-cycle) discards the working S2/S3 single
//! circuit. **Chosen: Option 1** — a bespoke non-native bn256-G1 SW
//! point gadget (`EmulatedFpVar<Fq,Fr>` coords + native-`FpVar<Fr>`-
//! scalar double-and-add ladder). Deep (≈ a second S4b) but bounded
//! and standard.
//!
//! Implementation plan (PHASE B.1→B.3, `MAINNET_REMAINING_WORK_FLOW`):
//!   B.1 non-native SW add/double gadget (incomplete-formula-safe) +
//!       isolated proof vs ark bn256-G1;
//!   B.2 native-scalar double-and-add MSM ladder + decoder/converter;
//!   B.3 `extract_primary_*` + real-fixture binding test.
//!
//! Intentionally NO gadget code yet — B.1 is the next deep unit.
//! See `S4_DESIGN.md` (corrected) + `MAINNET_REMAINING_WORK_FLOW.md`.

// No gadget yet — see the FINDING above. This module is deliberately
// implementation-free so the crate stays green and the difficulty is
// not misrepresented.

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
//! Until decided, there is intentionally NO primary MSM gadget here.
//! Leaving broken `ProjectiveVar` code would (a) not compile and
//! (b) misrepresent the difficulty. See `S4_DESIGN.md` (corrected)
//! and `MAINNET_REMAINING_WORK_FLOW.md` PHASE B (reclassified deep).

// No gadget yet — see the FINDING above. This module is deliberately
// implementation-free so the crate stays green and the difficulty is
// not misrepresented.

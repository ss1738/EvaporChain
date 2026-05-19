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

// ─────────────────────────────────────────────────────────────────────
// B.1 — non-native bn256-G1 short-Weierstrass point gadget (Option 1).
//
// bn256/BN254 G1: y² = x³ + 3  (a = 0, b = 3). Coordinates live in
// `ark_bn254::Fq`, foreign to the BN254-`Fr` circuit → emulated via
// `EmulatedFpVar<Fq, Fr>`. Generic-case (non-identity, P≠±Q for add,
// y≠0 for double) affine formulas — sufficient for the isolated
// correctness proof; incomplete-formula edge handling (identity /
// doubling / negation) is the B.2 ladder's concern.

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr};
use ark_r1cs_std::fields::emulated_fp::EmulatedFpVar;
use ark_r1cs_std::fields::FieldVar;
use ark_relations::r1cs::SynthesisError;

type FqV = EmulatedFpVar<Bn254Fq, Bn254Fr>;

/// In-circuit bn256-G1 affine point (generic, non-identity).
pub struct G1AffineVar {
    pub x: FqV,
    pub y: FqV,
}

/// Point doubling, a=0:  λ = 3x²/2y;  x₃ = λ² − 2x;  y₃ = λ(x − x₃) − y.
/// Requires `p` non-identity and `y ≠ 0` (generic case).
pub fn g1_double(p: &G1AffineVar) -> Result<G1AffineVar, SynthesisError> {
    let three = FqV::constant(Bn254Fq::from(3u64));
    let two = FqV::constant(Bn254Fq::from(2u64));
    let x_sq = &p.x * &p.x;
    let num = &three * &x_sq;
    let den = &two * &p.y;
    let lambda = &num * &den.inverse()?;
    let two_x = &p.x + &p.x;
    let x3 = &(&lambda * &lambda) - &two_x;
    let y3 = &(&lambda * &(&p.x - &x3)) - &p.y;
    Ok(G1AffineVar { x: x3, y: y3 })
}

/// Point addition of DISTINCT points (x₁ ≠ x₂):
/// λ = (y₂−y₁)/(x₂−x₁); x₃ = λ² − x₁ − x₂; y₃ = λ(x₁ − x₃) − y₁.
pub fn g1_add(p: &G1AffineVar, q: &G1AffineVar) -> Result<G1AffineVar, SynthesisError> {
    let num = &q.y - &p.y;
    let den = &q.x - &p.x;
    let lambda = &num * &den.inverse()?;
    let x3 = &(&(&lambda * &lambda) - &p.x) - &q.x;
    let y3 = &(&lambda * &(&p.x - &x3)) - &p.y;
    Ok(G1AffineVar { x: x3, y: y3 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::{short_weierstrass::Projective, AffineRepr, CurveGroup};
    use ark_r1cs_std::{alloc::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;

    /// THE B.1 PRIMITIVE PROOF: in-circuit non-native bn256-G1
    /// double + add equal out-of-circuit ark bn256-G1, CS satisfied.
    /// Generic points only (G, 2G, 3G are distinct & non-identity).
    #[test]
    fn nonnative_bn256_g1_double_add_match_ark() {
        let g_aff = ark_bn254::G1Affine::generator();
        let g = Projective::from(g_aff);
        let two_g = (g + g).into_affine();
        let three_g = (g + g + g).into_affine();

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let mkfq = |v: Bn254Fq| FqV::new_witness(cs.clone(), || Ok(v)).unwrap();
        let gx = g_aff.x().unwrap();
        let gy = g_aff.y().unwrap();
        let gp = G1AffineVar { x: mkfq(gx), y: mkfq(gy) };

        // double: 2G
        let d = g1_double(&gp).expect("g1_double");
        // add (distinct): G + 2G = 3G
        let twog_v = G1AffineVar {
            x: mkfq(two_g.x().unwrap()),
            y: mkfq(two_g.y().unwrap()),
        };
        let s = g1_add(&gp, &twog_v).expect("g1_add");

        assert!(cs.is_satisfied().expect("is_satisfied"), "CS must be satisfied");
        assert_eq!(d.x.value().unwrap(), two_g.x().unwrap(), "2G.x");
        assert_eq!(d.y.value().unwrap(), two_g.y().unwrap(), "2G.y");
        assert_eq!(s.x.value().unwrap(), three_g.x().unwrap(), "3G.x");
        assert_eq!(s.y.value().unwrap(), three_g.y().unwrap(), "3G.y");
    }
}

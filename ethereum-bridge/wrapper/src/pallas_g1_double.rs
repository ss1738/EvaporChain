//! `enforce_g1_doubling` — `P3 = 2 · P` for Pallas G1 affine points.
//!
//! Sibling gadget to `enforce_g1_add` in `pallas_g1.rs`. Required by
//! scalar-mul (double-and-add) and by the Halo2 IPA verifier's
//! accumulation step where one operand is the running accumulator
//! and the doubling occurs at every challenge round.
//!
//! # Why a separate gadget
//!
//! The affine-add formula `(y₂ − y₁)/(x₂ − x₁)` divides by zero when
//! `P₁ = P₂` (the doubling case). Doubling uses the tangent-line
//! slope instead:
//!
//! ```text
//!   λ  = (3x² + a) / 2y
//!   x₃ = λ² − 2x
//!   y₃ = λ·(x − x₃) − y
//! ```
//!
//! For Pallas (`y² = x³ + 5`, so `a = 0` per `ark_pallas::PallasConfig::COEFF_A`)
//! the slope simplifies to `λ = 3x²/2y`. Hardcoded here — the gadget
//! is Pallas-specific (cf. the wrapper's BN254 + Pallas combo).
//!
//! # num_additions discipline
//!
//! Same headroom limit as `enforce_g1_add`: each `enforce_equal` must
//! keep `num_add(LHS) + num_add(RHS) + 1 ≤ 3` for the same-bit-size
//! 254↔254 pair. The doubling slope `λ·2y = 3x²` is structurally
//! tighter than the generic-add slope — both sides multiply two terms
//! that themselves require 1-2 additions to construct. Resolution
//! (same pattern as PR #50 used for constraint (2) of `enforce_g1_add`):
//! split via intermediate witnesses so every `enforce_equal` has
//! `num_add ≤ 1` per side.
//!
//! # Caller precondition
//!
//! `P` MUST be non-identity AND not 2-torsion (`y ≠ 0`). For
//! Pallas G1's prime-order group both follow automatically from
//! `P ≠ ∞`, so a single `is_identity` check at the caller suffices.
//! The off-circuit slope computation panics on `2y = 0`; the
//! in-circuit witness allocation just fails to satisfy.
//!
//! # Constraint count
//!
//! 4 non-native Pallas Fq multiplications (`x·x`, `λ·y_doubled`,
//! `λ²`, `λ·(x−x₃)`) at ~3-4k constraints each, plus 7 `enforce_equal`
//! calls + 4 intermediate witness allocations. Empirically lands in
//! the 4-10k range; pinned by the constraint-count test below.

use crate::nonnative_fq::{alloc_nonnative_fq_witness, NonNativeFqVar};
use crate::pallas_g1::NonNativePallasPoint;
use ark_bn254::Fr as Bn254Fr;
use ark_ff::Field;
use ark_pallas::Fq as PallasFq;
use ark_r1cs_std::eq::EqGadget;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use ark_std::Zero;

/// Enforce `P3 = 2 · P` over non-native Pallas G1.
///
/// Caller provides both `P` (the input point) and `P3` (the expected
/// doubled point). The gadget allocates the slope witness λ
/// off-circuit, then enforces the three doubling equations via
/// `num_additions`-careful intermediate-witness splits.
///
/// Returns `SynthesisError::Unsatisfiable` if `2y = 0` off-circuit
/// (the caller violated the non-2-torsion precondition).
pub fn enforce_g1_doubling(
    cs: ConstraintSystemRef<Bn254Fr>,
    p: &NonNativePallasPoint,
    p3: &NonNativePallasPoint,
) -> Result<(), SynthesisError> {
    use ark_r1cs_std::R1CSVar;

    // ── Off-circuit witness computation ────────────────────────
    let x_val: PallasFq = p.x.value()?;
    let y_val: PallasFq = p.y.value()?;
    let two_y = y_val + y_val;
    if two_y.is_zero() {
        // 2-torsion point (only the identity satisfies 2y=0 on a curve
        // with odd group order; identity isn't representable here).
        return Err(SynthesisError::Unsatisfiable);
    }
    let two_y_inv = two_y.inverse().ok_or(SynthesisError::Unsatisfiable)?;
    let x_squared_val = x_val * x_val;
    let three = PallasFq::from(3u64);
    let three_x_sq = x_squared_val * three;
    let lambda_val = three_x_sq * two_y_inv;
    let y_doubled_val = two_y;
    let x_sq_3_val = three_x_sq;
    let two_x_val = x_val + x_val;

    // ── Allocate the slope λ and four intermediate witnesses ──
    let lambda: NonNativeFqVar = alloc_nonnative_fq_witness(cs.clone(), lambda_val)?;
    let y_doubled: NonNativeFqVar = alloc_nonnative_fq_witness(cs.clone(), y_doubled_val)?;
    let x_squared: NonNativeFqVar = alloc_nonnative_fq_witness(cs.clone(), x_squared_val)?;
    let x_sq_3: NonNativeFqVar = alloc_nonnative_fq_witness(cs.clone(), x_sq_3_val)?;
    let two_x: NonNativeFqVar = alloc_nonnative_fq_witness(cs.clone(), two_x_val)?;

    // ── Constraint chain ──
    //
    // Each enforce_equal designed so `num_add(LHS) + num_add(RHS) + 1`
    // stays at or below the 254↔254 headroom budget (≤3).
    //
    // (1) y_doubled == y + y                  (delta 0 + 1 + 1 = 2)
    y_doubled.enforce_equal(&(&p.y + &p.y))?;

    // (2) x_squared == x · x                  (delta 1 + 0 + 1 = 2)
    (&p.x * &p.x).enforce_equal(&x_squared)?;

    // (3) x_sq_3 − x_squared == x_squared + x_squared
    //                                          (delta 1 + 1 + 1 = 3)
    //
    // This is the constraint-(2)-style intermediate-witness split
    // (cf. PR #50 closing notes). `x_sq_3 == x_squared * 3` written
    // as `x_sq_3 - x_squared == x_squared + x_squared` keeps both
    // sides at num_add = 1.
    (&x_sq_3 - &x_squared).enforce_equal(&(&x_squared + &x_squared))?;

    // (4) λ · y_doubled == x_sq_3              (slope main, delta 1 + 0 + 1 = 2)
    (&lambda * &y_doubled).enforce_equal(&x_sq_3)?;

    // (5) two_x == x + x                       (delta 0 + 1 + 1 = 2)
    two_x.enforce_equal(&(&p.x + &p.x))?;

    // (6) λ² == x₃ + two_x                    (x-coord, delta 1 + 1 + 1 = 3)
    (&lambda * &lambda).enforce_equal(&(&p3.x + &two_x))?;

    // (7) λ · (x − x₃) == y₃ + y               (y-coord, delta 1 + 1 + 1 = 3)
    let x_diff = &p.x - &p3.x;
    let lambda_xdiff = &lambda * &x_diff;
    let y_sum = &p3.y + &p.y;
    lambda_xdiff.enforce_equal(&y_sum)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pallas_g1::NonNativePallasPoint;
    use ark_ec::{AffineRepr, CurveGroup};
    use ark_pallas::{Affine as PallasAffine, Projective as PallasProjective};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::rand::SeedableRng;
    use ark_std::UniformRand;

    fn seeded_rng() -> ark_std::rand::rngs::StdRng {
        ark_std::rand::rngs::StdRng::seed_from_u64(0xC0FFEE_u64)
    }

    fn random_non_identity_pallas(
        rng: &mut impl ark_std::rand::Rng,
    ) -> (PallasAffine, PallasAffine) {
        loop {
            let p_proj = PallasProjective::rand(rng);
            let p3_proj = p_proj + p_proj;
            let p = p_proj.into_affine();
            let p3 = p3_proj.into_affine();
            if p.is_zero() || p3.is_zero() {
                continue;
            }
            return (p, p3);
        }
    }

    /// HEADLINE — for a valid `(P, 2P)` pair the constraint system
    /// must be satisfied. Pins the doubling formula end-to-end on
    /// stock arkworks 0.5.
    #[test]
    fn g1_doubling_satisfied_for_valid_pair() {
        let mut rng = seeded_rng();
        let (p, p3) = random_non_identity_pallas(&mut rng);

        // Off-circuit sanity: confirm p3 is actually 2*p
        let expected = (PallasProjective::from(p) + PallasProjective::from(p)).into_affine();
        assert_eq!(expected, p3, "test fixture: p3 must equal 2*p");

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let p_var = NonNativePallasPoint::alloc_witness(cs.clone(), p).expect("alloc p");
        let p3_var = NonNativePallasPoint::alloc_witness(cs.clone(), p3).expect("alloc p3");
        enforce_g1_doubling(cs.clone(), &p_var, &p3_var).expect("enforce");
        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "valid (P, 2P) pair must satisfy doubling constraints"
        );
    }

    /// SOUNDNESS — a tampered `P3 ≠ 2P` must produce an unsatisfied
    /// constraint system. Without this gate the gadget would accept
    /// arbitrary pairs and any scalar-mul layered on top would be
    /// vacuous.
    #[test]
    fn g1_doubling_unsatisfied_when_p3_wrong() {
        let mut rng = seeded_rng();
        let (p, _real_p3) = random_non_identity_pallas(&mut rng);
        let wrong_p3 = PallasProjective::rand(&mut rng).into_affine();
        // Vanishingly unlikely wrong_p3 == 2*p; if it ever happens, the
        // fixture seed needs changing.
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let p_var = NonNativePallasPoint::alloc_witness(cs.clone(), p).expect("alloc p");
        let wrong_p3_var =
            NonNativePallasPoint::alloc_witness(cs.clone(), wrong_p3).expect("alloc wrong p3");
        enforce_g1_doubling(cs.clone(), &p_var, &wrong_p3_var).expect("enforce");
        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "tampered P3 must NOT satisfy doubling constraints"
        );
    }

    /// Pin the constraint count in a stable bracket. Doubling has 4
    /// non-native mults (one more than g1_add's 3) plus four
    /// intermediate witnesses. Empirical range ~4-10k.
    #[test]
    fn g1_doubling_constraint_count_in_expected_range() {
        let mut rng = seeded_rng();
        let (p, p3) = random_non_identity_pallas(&mut rng);
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let p_var = NonNativePallasPoint::alloc_witness(cs.clone(), p).expect("alloc p");
        let p3_var = NonNativePallasPoint::alloc_witness(cs.clone(), p3).expect("alloc p3");
        enforce_g1_doubling(cs.clone(), &p_var, &p3_var).expect("enforce");
        let n = cs.num_constraints();
        assert!(
            n > 3_000 && n < 12_000,
            "g1 doubling constraint count out of expected range: {} \
             (expected ~4-10k, 4 mults dominate the cost)",
            n
        );
    }
}

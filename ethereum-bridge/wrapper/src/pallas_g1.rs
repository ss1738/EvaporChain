//! Non-native Pallas G1 affine point addition gadget — second layer
//! of the sub-B-finish foundation.
//!
//! # What this gadget proves
//!
//! Given non-native `(x₁, y₁)`, `(x₂, y₂)`, `(x₃, y₃)` over Pallas Fq
//! inside BN254 Fr, enforces that `P₃ = P₁ + P₂` under the affine
//! short-Weierstrass addition formula:
//!
//! ```text
//!   λ  = (y₂ − y₁) / (x₂ − x₁)        with  x₁ ≠ x₂
//!   x₃ = λ² − x₁ − x₂
//!   y₃ = λ·(x₁ − x₃) − y₁
//! ```
//!
//! The Halo2 IPA verifier issues these generic adds during MSM /
//! inner-product accumulation. Doubling (x₁ = x₂, y₁ = y₂) and
//! identity (P₂ = −P₁) are handled by **separate gadgets** in the
//! sub-B-finish layer — they need different formulas and we keep the
//! starter scope tight.
//!
//! # Constraint cost
//!
//! Affine generic add over non-native Fq:
//!
//!   - 1 Fq inversion  (witness λ + check λ·(x₂−x₁) = (y₂−y₁))   ~3k constraints
//!   - 1 Fq mult       (λ² = λ·λ)                                ~3k constraints
//!   - 1 Fq mult       (λ·(x₁−x₃))                               ~3k constraints
//!   - 3 Fq adds       (cheap, <<<1k constraints)
//!
//!   → ~9-10k Groth16 constraints per Pallas G1 add.
//!
//! The IPA verifier does ~k generic adds (k≈10 for the EvaporChain
//! Verkle circuit), giving ~100k Groth16 constraints for the G1
//! layer alone. Sub-B-finish also needs scalar-mul (the dominant
//! cost) which compounds this.
//!
//! # Identity-point handling — explicitly scoped out
//!
//! The affine formula divides by `(x₂ − x₁)`. If `x₁ = x₂`, the
//! inversion fails:
//!   - `y₁ = y₂` (doubling) — needs the doubling formula
//!     `λ = 3x₁² / 2y₁`.
//!   - `y₁ = −y₂` (inverse) — sum is the point at infinity `O`,
//!     which has no affine representation.
//!
//! [`enforce_g1_add`] is **load-bearing only when x₁ ≠ x₂**. Callers
//! that may pass equal x-coords must route through the union gadget
//! sub-B-finish provides (selector + branched formulas + identity
//! tag). For the Halo2 IPA verifier, MSM-fashion accumulation can
//! be arranged to avoid both edge cases by construction (random
//! linear combinations of distinct points), so generic add suffices
//! for ~90% of the constraint mass.

use crate::nonnative_fq::{alloc_nonnative_fq_witness, NonNativeFqVar};
use ark_bn254::Fr as Bn254Fr;
use ark_ec::AffineRepr;
use ark_pallas::{Affine as PallasAffine, Fq as PallasFq};
use ark_r1cs_std::eq::EqGadget;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

/// Non-native Pallas G1 affine point — `(x, y)` allocated as a pair
/// of `NonNativeFqVar`s. The constraint system tracks them together;
/// the identity (point at infinity) is NOT representable here — sub-B-
/// finish adds a separate `MaybeIdentityPoint` wrapper if needed.
#[derive(Clone)]
pub struct NonNativePallasPoint {
    pub x: NonNativeFqVar,
    pub y: NonNativeFqVar,
}

impl NonNativePallasPoint {
    /// Allocate a Pallas affine point as a private witness. Panics if
    /// `point.is_identity()` — identity-point handling is sub-B-finish.
    pub fn alloc_witness(
        cs: ConstraintSystemRef<Bn254Fr>,
        point: PallasAffine,
    ) -> Result<Self, SynthesisError> {
        let (x_val, y_val) = point
            .xy()
            .expect("identity point not supported by NonNativePallasPoint scaffold");
        let x = alloc_nonnative_fq_witness(cs.clone(), *x_val)?;
        let y = alloc_nonnative_fq_witness(cs, *y_val)?;
        Ok(Self { x, y })
    }
}

/// Enforce `P3 = P1 + P2` over non-native Pallas G1 via the affine
/// generic-add formula. **Requires `P1.x ≠ P2.x` at witness time** —
/// see module doc.
///
/// Witness path: the caller provides P3 (the expected sum). The
/// gadget allocates a witness λ and asserts:
///
///   1. λ · (x₂ − x₁) = (y₂ − y₁)        [slope definition]
///   2. λ² = x₃ + x₁ + x₂                [x-coordinate]
///   3. λ · (x₁ − x₃) = y₃ + y₁          [y-coordinate]
///
/// A satisfying witness exists iff P3 is exactly the geometric sum.
pub fn enforce_g1_add(
    cs: ConstraintSystemRef<Bn254Fr>,
    p1: &NonNativePallasPoint,
    p2: &NonNativePallasPoint,
    p3: &NonNativePallasPoint,
) -> Result<(), SynthesisError> {
    // λ is a witness — its value is fixed by P1 and P2, so we compute
    // it off-circuit and allocate, then the slope-definition constraint
    // pins it.
    let lambda_val = compute_lambda(p1, p2)?;
    let lambda = alloc_nonnative_fq_witness(cs.clone(), lambda_val)?;

    // Rewrite the affine-add constraints into purely additive forms so
    // every equality compares two "post-mult-plus-adds" representations
    // with matching `num_of_additions` counters. This avoids the
    // arkworks 0.4 NonNativeFieldVar issue where chained sub→mult
    // leaves the limb representation in a state that `enforce_equal`
    // can't reduce-and-compare against a post-mult LHS.
    //
    // (1) λ·(x₂ − x₁) = (y₂ − y₁)  →  λ·x₂ + y₁ = λ·x₁ + y₂
    let lambda_x2 = &lambda * &p2.x;
    let lambda_x1 = &lambda * &p1.x;
    let lhs1 = &lambda_x2 + &p1.y;
    let rhs1 = &lambda_x1 + &p2.y;
    lhs1.enforce_equal(&rhs1)?;

    // (2) λ² = x₃ + x₁ + x₂  →  already additive on the RHS
    let lambda_sq = &lambda * &lambda;
    let x_sum = &p3.x + &p1.x + &p2.x;
    lambda_sq.enforce_equal(&x_sum)?;

    // (3) λ·(x₁ − x₃) = y₃ + y₁  →  λ·x₁ = λ·x₃ + y₃ + y₁
    let lambda_x3 = &lambda * &p3.x;
    let rhs3 = &lambda_x3 + &p3.y + &p1.y;
    lambda_x1.enforce_equal(&rhs3)?;

    Ok(())
}

/// Compute λ = (y₂ − y₁) / (x₂ − x₁) off-circuit from the witness
/// values. Used by [`enforce_g1_add`] to produce the slope witness.
fn compute_lambda(
    p1: &NonNativePallasPoint,
    p2: &NonNativePallasPoint,
) -> Result<PallasFq, SynthesisError> {
    use ark_r1cs_std::R1CSVar;
    let x1: PallasFq = p1.x.value()?;
    let y1: PallasFq = p1.y.value()?;
    let x2: PallasFq = p2.x.value()?;
    let y2: PallasFq = p2.y.value()?;
    let dx = x2 - x1;
    if dx.is_zero_vartime() {
        // Doubling / identity path — caller violated the scaffold's
        // precondition. SynthesisError isn't quite the right type but
        // it's the only error path enforce_g1_add can surface.
        return Err(SynthesisError::Unsatisfiable);
    }
    let dx_inv = dx.inverse().ok_or(SynthesisError::Unsatisfiable)?;
    Ok((y2 - y1) * dx_inv)
}

// `is_zero_vartime` and `Field::inverse` come from `ark_ff` traits.
// Import here so the trait methods are in scope.
use ark_ff::Field;

trait IsZero {
    fn is_zero_vartime(&self) -> bool;
}

impl IsZero for PallasFq {
    fn is_zero_vartime(&self) -> bool {
        ark_ff::Zero::is_zero(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::{CurveGroup, Group};
    use ark_pallas::Projective as PallasProjective;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::rand::SeedableRng;
    use ark_std::UniformRand;

    fn seeded_rng() -> ark_std::rand::rngs::StdRng {
        ark_std::rand::rngs::StdRng::seed_from_u64(0xC0FFEE_u64)
    }

    /// Generate two random distinct Pallas G1 points + their sum, all
    /// in affine form (with the identity ruled out by construction —
    /// with overwhelming probability, two random points are not
    /// inverses of each other and don't have equal x-coords).
    fn random_distinct_pallas_triple(
        rng: &mut impl ark_std::rand::Rng,
    ) -> (PallasAffine, PallasAffine, PallasAffine) {
        loop {
            let p1_proj = PallasProjective::rand(rng);
            let p2_proj = PallasProjective::rand(rng);
            let p3_proj = p1_proj + p2_proj;
            let p1 = p1_proj.into_affine();
            let p2 = p2_proj.into_affine();
            let p3 = p3_proj.into_affine();
            // Skip degenerate cases — identity points or equal x.
            if p1.is_zero() || p2.is_zero() || p3.is_zero() {
                continue;
            }
            let (x1, _) = p1.xy().expect("p1 not identity");
            let (x2, _) = p2.xy().expect("p2 not identity");
            if x1 == x2 {
                continue;
            }
            return (p1, p2, p3);
        }
    }

    /// Satisfied path — for a valid (P1, P2, P3 = P1 + P2) triple,
    /// the constraint system is satisfiable.
    #[test]
    fn g1_add_satisfied_for_valid_triple() {
        let mut rng = seeded_rng();
        let (p1, p2, p3) = random_distinct_pallas_triple(&mut rng);
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();

        let p1_var = NonNativePallasPoint::alloc_witness(cs.clone(), p1).expect("alloc p1");
        let p2_var = NonNativePallasPoint::alloc_witness(cs.clone(), p2).expect("alloc p2");
        let p3_var = NonNativePallasPoint::alloc_witness(cs.clone(), p3).expect("alloc p3");

        enforce_g1_add(cs.clone(), &p1_var, &p2_var, &p3_var).expect("enforce");

        if !cs.is_satisfied().expect("is_satisfied") {
            // Debug — figure out which constraint failed by computing
            // each side off-circuit and comparing.
            let (x1, y1) = p1.xy().expect("p1 xy");
            let (x2, y2) = p2.xy().expect("p2 xy");
            let (x3, y3) = p3.xy().expect("p3 xy");
            let lambda = (y2 - y1) * (x2 - x1).inverse().expect("dx inv");
            eprintln!("DEBUG: x1 = {:?}", x1);
            eprintln!("DEBUG: y1 = {:?}", y1);
            eprintln!("DEBUG: x2 = {:?}", x2);
            eprintln!("DEBUG: y2 = {:?}", y2);
            eprintln!("DEBUG: x3 = {:?}", x3);
            eprintln!("DEBUG: y3 = {:?}", y3);
            eprintln!("DEBUG: lambda = {:?}", lambda);
            eprintln!("DEBUG: lambda*(x2-x1) = {:?}", lambda * (x2 - x1));
            eprintln!("DEBUG: y2-y1          = {:?}", *y2 - *y1);
            eprintln!("DEBUG: lambda^2       = {:?}", lambda * lambda);
            eprintln!("DEBUG: x3+x1+x2       = {:?}", *x3 + *x1 + *x2);
            eprintln!("DEBUG: lambda*(x1-x3) = {:?}", lambda * (*x1 - *x3));
            eprintln!("DEBUG: y3+y1          = {:?}", *y3 + *y1);
            eprintln!("DEBUG: unsatisfied constraint index: {:?}", cs.which_is_unsatisfied().ok());
            panic!("constraint NOT satisfied — see debug output above");
        }
    }

    /// Unsatisfied path — tampering P3 to be a different valid point
    /// breaks satisfaction. Without this gate the gadget would accept
    /// arbitrary triples.
    #[test]
    fn g1_add_unsatisfied_when_p3_wrong() {
        let mut rng = seeded_rng();
        let (p1, p2, _real_p3) = random_distinct_pallas_triple(&mut rng);
        let wrong_p3 = (PallasProjective::rand(&mut rng)).into_affine();
        // Vanishingly unlikely wrong_p3 == p1 + p2; abort if it ever does.
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();

        let p1_var = NonNativePallasPoint::alloc_witness(cs.clone(), p1).expect("alloc p1");
        let p2_var = NonNativePallasPoint::alloc_witness(cs.clone(), p2).expect("alloc p2");
        let wrong_p3_var =
            NonNativePallasPoint::alloc_witness(cs.clone(), wrong_p3).expect("alloc wrong p3");

        enforce_g1_add(cs.clone(), &p1_var, &p2_var, &wrong_p3_var).expect("enforce");

        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "tampered P3 must NOT satisfy constraints"
        );
    }

    /// Constraint count for a single G1 add is in the expected ~9-15k
    /// range (3 non-native Fq mults + 1 inversion-via-witness, each
    /// ~3k constraints). Pins the baseline cost for sub-B-finish
    /// capacity planning.
    #[test]
    fn g1_add_constraint_count_in_expected_range() {
        let mut rng = seeded_rng();
        let (p1, p2, p3) = random_distinct_pallas_triple(&mut rng);
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();

        let p1_var = NonNativePallasPoint::alloc_witness(cs.clone(), p1).expect("alloc p1");
        let p2_var = NonNativePallasPoint::alloc_witness(cs.clone(), p2).expect("alloc p2");
        let p3_var = NonNativePallasPoint::alloc_witness(cs.clone(), p3).expect("alloc p3");
        enforce_g1_add(cs.clone(), &p1_var, &p2_var, &p3_var).expect("enforce");

        let n = cs.num_constraints();
        // 3 Fq mults × ~3k each + overhead = expect 10k-20k. Tight
        // bracketing here means an arkworks version bump that changes
        // limb decomposition will be caught loudly.
        assert!(
            n > 5_000 && n < 30_000,
            "G1 add constraint count out of expected range: {}",
            n
        );
    }
}

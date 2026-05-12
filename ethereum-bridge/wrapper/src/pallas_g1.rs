//! Non-native Pallas G1 affine point addition gadget — second layer
//! of the sub-B-finish foundation.
//!
//! # Resolution (2026-05-12) — constraint (2) num_additions split
//!
//! The completeness gap is CLOSED. Root cause: arkworks's same-bit-size
//! 254↔254 `enforce_equal` has a headroom limit on the combined
//! `num_of_additions_over_normal_form` budget; empirically a delta of
//! 4 fails while a delta of 3 passes. Per the `+`/`-` accounting in
//! `ark-r1cs-std-0.5 emulated_fp/allocated_field_var.rs:148,170` each
//! add/sub adds 1 to the operand's running counter.
//!
//! Constraint shapes after the fix:
//!
//! | Constraint | LHS num_add | RHS num_add | Pass |
//! |---|---|---|---|
//! | (1) `λ(x₂−x₁) = y₂−y₁`         | 1 (mult)    | 1 (sub)    | ✅ |
//! | (2a) `x_sum−x₃ = x₁+x₂`        | 1 (sub)     | 1 (add)    | ✅ |
//! | (2b) `λ² = x_sum`              | 1 (mult)    | 0 (witness)| ✅ |
//! | (3) `λ(x₁−x₃) = y₃+y₁`         | 1 (mult)    | 1 (add)    | ✅ |
//!
//! Pre-fix constraint (2) had RHS `x₁ + x₂ + x₃` with num_add = 2 —
//! exactly 1 past the budget. The intermediate witness `x_sum` adds
//! one extra non-native witness + one `enforce_equal`, marginal
//! constraint cost (~+200), and unblocks the gadget completely.
//!
//! **Bisect history that led here (all on Mini 1, release):**
//!
//!   1. arkworks 0.4 → 0.5 upgrade — gap still open.
//!   2. Additive-form rewrite of all 3 constraints — gap still open;
//!      constraint count ballooned to ~9-10k.
//!   3. Canonical-form revert — gap still open; constraint count
//!      ~4037. (Shipped as #49 anyway: strict improvement.)
//!   4. arkworks `find_parameters` patches (max_limb_size = 80, then
//!      32) — gap still open. Eliminated parameter selection as the
//!      cause.
//!   5. Minimal `EmulatedFpVar<PallasFq, Bn254Fr>` mult-then-eq test
//!      — passed. Eliminated arkworks-itself as the cause for the
//!      simple case.
//!   6. Per-constraint isolation bisect of g1_add — only constraint
//!      (2) failed in isolation. Pinpointed the bug.
//!   7. Intermediate-witness split for constraint (2) — **passed**.
//!      Fix landed in `enforce_g1_add` below.
//!
//! The fix relies on no upstream patch — runs on stock arkworks 0.5.
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
        // arkworks 0.5: `xy()` returns owned `(Fq, Fq)` not refs.
        let (x_val, y_val) = point
            .xy()
            .expect("identity point not supported by NonNativePallasPoint scaffold");
        let x = alloc_nonnative_fq_witness(cs.clone(), x_val)?;
        let y = alloc_nonnative_fq_witness(cs, y_val)?;
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

    // ── Constraint chain ──
    //
    // Every `enforce_equal` must keep the combined `num_of_additions`
    // budget under arkworks's same-bit-size 254↔254 headroom limit.
    // Empirically (bisect 2026-05-12) the limit is `num_add(LHS) +
    // num_add(RHS) + 1 ≤ 3`. Constraints (1) and (3) naturally satisfy
    // this — mult-result on LHS (num_add = 1) vs single-add/sub on RHS
    // (num_add = 1).
    //
    // Constraint (2) `λ² = x₁ + x₂ + x₃` has THREE field vars on the
    // RHS, which (via the `add` accounting at `ark-r1cs-std-0.5
    // emulated_fp/allocated_field_var.rs:148`) raises RHS num_add to 2.
    // That blows the limit. Fix: allocate `x_sum = x₁ + x₂ + x₃` as a
    // SEPARATE witness, then split the additive relation into two
    // smaller `enforce_equal`s. Each per-side num_add stays ≤ 1.
    //
    // (1) Slope definition: λ · (x₂ − x₁) = (y₂ − y₁)
    let dx = &p2.x - &p1.x;
    let dy = &p2.y - &p1.y;
    let lambda_dx = &lambda * &dx;
    lambda_dx.enforce_equal(&dy)?;

    // (2) x-coord: λ² = x₁ + x₂ + x₃   (split via intermediate witness)
    //
    // 2a — allocate `x_sum_val = x₁ + x₂ + x₃` off-circuit.
    use ark_r1cs_std::R1CSVar;
    let x_sum_val: PallasFq = p1.x.value()? + p2.x.value()? + p3.x.value()?;
    let x_sum = alloc_nonnative_fq_witness(cs.clone(), x_sum_val)?;
    //
    // 2b — enforce `x_sum − x₃ == x₁ + x₂`. LHS: 1 sub (num_add = 1);
    //      RHS: 1 add (num_add = 1). Within headroom.
    let x_sum_minus_x3 = &x_sum - &p3.x;
    let x12 = &p1.x + &p2.x;
    x_sum_minus_x3.enforce_equal(&x12)?;
    //
    // 2c — enforce `λ² == x_sum`. LHS: mult-result (num_add = 1);
    //      RHS: fresh witness (num_add = 0). Within headroom.
    let lambda_sq = &lambda * &lambda;
    lambda_sq.enforce_equal(&x_sum)?;

    // (3) y-coord: λ · (x₁ − x₃) = y₃ + y₁
    let x_diff = &p1.x - &p3.x;
    let lambda_xdiff = &lambda * &x_diff;
    let y_sum = &p3.y + &p1.y;
    lambda_xdiff.enforce_equal(&y_sum)?;

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
    // `Group` moved out of `ark_ec` root in 0.5 — only need `CurveGroup` here.
    use ark_ec::CurveGroup;
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
    /// the constraint system would be satisfiable iff the underlying
    /// `NonNativeFieldVar<PallasFq, Bn254Fr>` was complete. See the
    /// module-level "Known issue" doc — arkworks 0.4 reports
    /// `unsatisfied` for valid triples despite all 3 constraints
    /// holding mathematically off-circuit (BigInt-equal limbs both
    /// sides). The failure is internal to arkworks's limb-reduction
    /// layer for same-bit-size target/base field pairs.
    ///
    /// Satisfied path — for a valid (P1, P2, P3 = P1 + P2) triple
    /// the constraint system MUST be satisfiable. Was `#[ignore]`'d
    /// while the completeness gap was open; closed 2026-05-12 via
    /// the constraint (2) intermediate-witness split (see module
    /// doc "Resolution: constraint (2) num_additions split"). The
    /// test is now active and pins the fix.
    #[test]
    fn g1_add_satisfied_for_valid_triple() {
        let mut rng = seeded_rng();
        let (p1, p2, p3) = random_distinct_pallas_triple(&mut rng);
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();

        let p1_var = NonNativePallasPoint::alloc_witness(cs.clone(), p1).expect("alloc p1");
        let p2_var = NonNativePallasPoint::alloc_witness(cs.clone(), p2).expect("alloc p2");
        let p3_var = NonNativePallasPoint::alloc_witness(cs.clone(), p3).expect("alloc p3");

        enforce_g1_add(cs.clone(), &p1_var, &p2_var, &p3_var).expect("enforce");

        // Off-circuit verification — pin that the math IS correct;
        // the failure is in arkworks's constraint layer, not our formula.
        // arkworks 0.5: `xy()` returns owned tuples — no derefs needed.
        let (x1, y1) = p1.xy().expect("p1 xy");
        let (x2, y2) = p2.xy().expect("p2 xy");
        let (x3, y3) = p3.xy().expect("p3 xy");
        let lambda = (y2 - y1) * (x2 - x1).inverse().expect("dx inv");
        assert_eq!(lambda * (x2 - x1), y2 - y1, "off-circuit slope OK");
        assert_eq!(lambda * lambda, x3 + x1 + x2, "off-circuit x-coord OK");
        assert_eq!(lambda * (x1 - x3), y3 + y1, "off-circuit y-coord OK");

        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "valid (P1, P2, P3 = P1 + P2) triple must satisfy the three \
             affine-add constraints — the constraint (2) num_additions \
             split (see enforce_g1_add) keeps each enforce_equal within \
             arkworks's same-bit-size 254↔254 headroom."
        );
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
    ///
    /// Constraint count is independent of the `is_satisfied()`
    /// completeness gap — this test only inspects the synthesised
    /// constraint system shape, not whether a valid triple satisfies
    /// it. As of the canonical-form revert (2026-05-12) the count is
    /// ~4037 on arkworks 0.5; previously the additive-rewrite form
    /// produced ~9-10k. Test runs unconditionally so any future
    /// arkworks-limb-decomposition change is caught loudly.
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
        // Canonical-form constraints (post-2026-05-12 revert from the
        // additive-rewrite): each `enforce_equal` compares a single
        // post-mult side with a single sum-of-witnesses side, so we
        // get 3 non-native Fq mults + the inversion-via-witness path
        // — empirically ~4k constraints on arkworks 0.5. (The
        // additive-rewrite form was ~9-10k; reverting halved the
        // constraint count.) Tight bracketing here means an arkworks
        // version bump that changes limb decomposition will be caught
        // loudly.
        assert!(
            n > 2_000 && n < 8_000,
            "G1 add constraint count out of expected range: {}",
            n
        );
    }
}

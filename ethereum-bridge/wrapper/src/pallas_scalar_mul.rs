//! `enforce_scalar_mul` — `Q = k · P` for Pallas G1 affine points.
//!
//! Layered on `enforce_g1_doubling` (PR #52) + `enforce_g1_add`
//! (PR #50). Implements MSB-first double-and-add over a bit-decomposed
//! scalar.
//!
//! # Ladder shape
//!
//! Caller provides `k_bits` MSB-first (e.g. `vec![1, 0, 1]` for `k = 5`).
//! The walk:
//!
//! ```text
//!   acc ← P                                 (consume MSB; assumes MSB = 1)
//!   for each remaining bit b_i:
//!     doubled  ← 2 · acc                    (always doubling)
//!     added    ← doubled + P                (always add — constant-time)
//!     acc      ← b_i ? added : doubled      (conditional select per coord)
//!   assert  acc == Q
//! ```
//!
//! Each iteration: 1 doubling (~5-6k constraints) + 1 add (~4k) +
//! conditional_select on (x, y) (~hundreds each). For an `n`-bit
//! scalar, total is `(n-1)` iterations ≈ `10k · (n-1)` constraints.
//!
//! # Caller preconditions
//!
//! 1. **MSB must be 1.** The ladder starts by setting `acc = P`,
//!    which is correct iff the topmost bit is `1`. A leading-zero bit
//!    would multiply by `0` and yield identity — not representable
//!    here. Use a minimal bit-length encoding (no leading zeros).
//!
//! 2. **No intermediate identity.** The affine-add formula inside
//!    `enforce_g1_add` divides by `x_acc − x_P`; if the running
//!    accumulator equals `±P` at any step, that division blows up.
//!    For random scalars + random generators this is negligible
//!    (probability ~`1/|Pallas|`). For adversarial `(k, P)` pairs the
//!    caller must use a complete-formula gadget (Renes-Costello),
//!    which is out of scope here.
//!
//! 3. **`P ≠ identity`.** Asserted at witness allocation time.
//!
//! # What this gadget does NOT do
//!
//! - Bit-decomposition of a non-native scalar. Caller supplies `k_bits`
//!   already as `Vec<Boolean<Bn254Fr>>`. The Halo2 IPA verifier
//!   layer above will bit-decompose Fiat-Shamir challenges separately.
//! - Window-based ladders (e.g. 4-bit windows with precomputed
//!   tables). These reduce constraint count by ~3-4× and are the
//!   eventual right shape for the full IPA verifier, but are
//!   self-contained follow-up work.

use crate::nonnative_fq::NonNativeFqVar;
use crate::pallas_g1::{enforce_g1_add, NonNativePallasPoint};
use crate::pallas_g1_double::enforce_g1_doubling;
use ark_bn254::Fr as Bn254Fr;
use ark_ec::{AffineRepr, CurveGroup};
use ark_pallas::{Affine as PallasAffine, Projective as PallasProjective};
use ark_r1cs_std::alloc::AllocVar;
use ark_r1cs_std::boolean::Boolean;
use ark_r1cs_std::eq::EqGadget;
use ark_r1cs_std::select::CondSelectGadget;
use ark_r1cs_std::R1CSVar;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

/// Enforce `Q = k · P` over non-native Pallas G1 via MSB-first
/// double-and-add.
///
/// `k_bits_msb_first` is the bit-decomposition of the scalar with the
/// most significant bit first. The MSB must be `1` (caller precondition,
/// see module doc).
///
/// Returns `SynthesisError::Unsatisfiable` if the bit slice is empty
/// or if `P` is the identity.
pub fn enforce_scalar_mul(
    cs: ConstraintSystemRef<Bn254Fr>,
    p: &NonNativePallasPoint,
    k_bits_msb_first: &[Boolean<Bn254Fr>],
    q: &NonNativePallasPoint,
) -> Result<(), SynthesisError> {
    if k_bits_msb_first.is_empty() {
        return Err(SynthesisError::Unsatisfiable);
    }

    // ── Native bootstrap: reconstruct `P` as a Pallas projective so
    //    the ladder's witness values are computed off-circuit and
    //    fed to `alloc_witness` at each step. ──────────────────────
    let p_x_val = p.x.value()?;
    let p_y_val = p.y.value()?;
    let p_affine = PallasAffine::new_unchecked(p_x_val, p_y_val);
    if p_affine.is_zero() {
        return Err(SynthesisError::Unsatisfiable);
    }
    let p_proj = PallasProjective::from(p_affine);

    // ── Initialise acc = P (consumes the MSB bit, which must be 1). ──
    let mut acc_proj = p_proj;
    let mut acc_var: NonNativePallasPoint = p.clone();

    // Walk the remaining bits MSB-first.
    for bit in k_bits_msb_first.iter().skip(1) {
        // Always-double — produce `2 · acc` as a fresh witness and
        // enforce via the doubling gadget. This keeps acc's
        // `num_of_additions` at 0 after every step.
        let doubled_proj = acc_proj + acc_proj;
        let doubled_affine = doubled_proj.into_affine();
        if doubled_affine.is_zero() {
            // Intermediate identity — not representable. See module
            // doc precondition 2.
            return Err(SynthesisError::Unsatisfiable);
        }
        let doubled_var =
            NonNativePallasPoint::alloc_witness(cs.clone(), doubled_affine)?;
        enforce_g1_doubling(cs.clone(), &acc_var, &doubled_var)?;

        // Always-add — produce `doubled + P` as a fresh witness and
        // enforce via the add gadget. The ladder uses this *only* if
        // the current bit is 1, but always allocates so the
        // constraint count is fixed (constant-time wrt the scalar
        // value, defeating any timing side-channel).
        let added_proj = doubled_proj + p_proj;
        let added_affine = added_proj.into_affine();
        if added_affine.is_zero() {
            // doubled = -P  — intermediate identity. Same case as
            // above; precondition violated.
            return Err(SynthesisError::Unsatisfiable);
        }
        let added_var =
            NonNativePallasPoint::alloc_witness(cs.clone(), added_affine)?;
        enforce_g1_add(cs.clone(), &doubled_var, p, &added_var)?;

        // Conditional select per coordinate: next_acc = bit ? added : doubled.
        let next_x = NonNativeFqVar::conditionally_select(bit, &added_var.x, &doubled_var.x)?;
        let next_y = NonNativeFqVar::conditionally_select(bit, &added_var.y, &doubled_var.y)?;

        // Update the native accumulator using the bit's witness value.
        // bit.value() is always Known during synthesis (it's a witness
        // we control), so the unwrap can't fail.
        let bit_val = bit.value().unwrap_or(false);
        acc_proj = if bit_val { added_proj } else { doubled_proj };
        acc_var = NonNativePallasPoint {
            x: next_x,
            y: next_y,
        };
    }

    // Final equality — both coordinates. Each side has num_add ≤ 0
    // (acc_var is a coord_select output from witness branches; q is a
    // witness). Within headroom.
    acc_var.x.enforce_equal(&q.x)?;
    acc_var.y.enforce_equal(&q.y)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::rand::SeedableRng;
    use ark_std::UniformRand;

    fn seeded_rng() -> ark_std::rand::rngs::StdRng {
        ark_std::rand::rngs::StdRng::seed_from_u64(0xC0FFEE_u64)
    }

    /// MSB-first bit decomposition of a small u64 scalar. Returns
    /// only the significant bits (leading zeros stripped). Asserts
    /// the result is non-empty and starts with a 1 — the gadget's
    /// MSB-must-be-1 precondition.
    fn bits_msb_first_u64(k: u64) -> Vec<bool> {
        let mut bits = Vec::new();
        let mut started = false;
        for i in (0..64).rev() {
            let b = (k >> i) & 1 == 1;
            if b {
                started = true;
            }
            if started {
                bits.push(b);
            }
        }
        assert!(!bits.is_empty(), "k must be > 0");
        assert!(bits[0], "MSB must be 1");
        bits
    }

    fn alloc_bits(
        cs: ConstraintSystemRef<Bn254Fr>,
        bits: &[bool],
    ) -> Vec<Boolean<Bn254Fr>> {
        bits.iter()
            .map(|b| Boolean::new_witness(cs.clone(), || Ok(*b)).expect("alloc bit"))
            .collect()
    }

    /// HEADLINE — `Q = 5 · P` (k_bits = [1, 0, 1]). 3-bit scalar →
    /// 2 ladder iterations (after MSB consumption). Constraint system
    /// must be satisfied for a real `(P, 5P)` pair.
    #[test]
    fn scalar_mul_satisfied_for_k_5() {
        let mut rng = seeded_rng();
        let p_proj = PallasProjective::rand(&mut rng);
        let q_proj = p_proj + p_proj + p_proj + p_proj + p_proj; // 5P
        let p = p_proj.into_affine();
        let q = q_proj.into_affine();
        assert!(!p.is_zero() && !q.is_zero());

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let p_var = NonNativePallasPoint::alloc_witness(cs.clone(), p).expect("alloc p");
        let q_var = NonNativePallasPoint::alloc_witness(cs.clone(), q).expect("alloc q");
        let k_bits = bits_msb_first_u64(5);
        let k_bits_var = alloc_bits(cs.clone(), &k_bits);

        enforce_scalar_mul(cs.clone(), &p_var, &k_bits_var, &q_var).expect("enforce");
        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "valid (P, 5P) must satisfy scalar-mul constraints"
        );
    }

    /// SOUNDNESS — wrong `Q` must not satisfy. Without this gate the
    /// gadget would accept arbitrary `(P, k, Q)` triples.
    #[test]
    fn scalar_mul_unsatisfied_when_q_wrong() {
        let mut rng = seeded_rng();
        let p_proj = PallasProjective::rand(&mut rng);
        let wrong_q_proj = PallasProjective::rand(&mut rng);
        let p = p_proj.into_affine();
        let wrong_q = wrong_q_proj.into_affine();

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let p_var = NonNativePallasPoint::alloc_witness(cs.clone(), p).expect("alloc p");
        let wrong_q_var =
            NonNativePallasPoint::alloc_witness(cs.clone(), wrong_q).expect("alloc wrong q");
        let k_bits = bits_msb_first_u64(5);
        let k_bits_var = alloc_bits(cs.clone(), &k_bits);

        enforce_scalar_mul(cs.clone(), &p_var, &k_bits_var, &wrong_q_var).expect("enforce");
        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "tampered Q must not satisfy scalar-mul constraints"
        );
    }

    /// Constraint count for k=5 (3-bit scalar, 2 ladder iterations).
    /// Each iteration: 1 doubling (~5k) + 1 add (~4k) + cond_select
    /// (~hundreds). Bracket the empirical range.
    #[test]
    fn scalar_mul_constraint_count_in_expected_range() {
        let mut rng = seeded_rng();
        let p_proj = PallasProjective::rand(&mut rng);
        let q_proj = p_proj + p_proj + p_proj + p_proj + p_proj;
        let p = p_proj.into_affine();
        let q = q_proj.into_affine();

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let p_var = NonNativePallasPoint::alloc_witness(cs.clone(), p).expect("alloc p");
        let q_var = NonNativePallasPoint::alloc_witness(cs.clone(), q).expect("alloc q");
        let k_bits = bits_msb_first_u64(5);
        let k_bits_var = alloc_bits(cs.clone(), &k_bits);

        enforce_scalar_mul(cs.clone(), &p_var, &k_bits_var, &q_var).expect("enforce");
        let n = cs.num_constraints();
        // 2 iterations × (~5k doubling + ~4k add + cond_select) ≈ 18-25k.
        // Wide bracket to absorb arkworks-version variation.
        assert!(
            n > 12_000 && n < 40_000,
            "scalar-mul k=5 constraint count out of expected range: {} \
             (expected ~18-25k for 2 ladder iterations)",
            n
        );
    }
}

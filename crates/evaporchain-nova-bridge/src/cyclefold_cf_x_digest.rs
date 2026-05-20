//! B-1/B-2 EVM, option (1C) — increment 4b-β-1: **native
//! `cf_x_digest`** oracle.
//!
//! # What this module establishes
//!
//! The CycleFold-augmented primary's public IO carries a single
//! `Bn254Fr` digest `cf_x_digest` that BINDS the per-step cross-
//! curve scalar-mul tuple `(P_step, s_step, Q_step)`. The aux side
//! independently allocates `(P, s, Q)` over `Bn254Fq` and
//! recomputes the matching digest; equality of the two digests is
//! the cross-circuit binding (because raw Bn254Fq coords cannot
//! safely cross into a Bn254Fr circuit's IO).
//!
//! This module ships the **native** computation of that digest.
//! [`compute_cf_x_digest_native`] is the oracle that 4b-β-2's
//! in-circuit gadget must reproduce bit-for-bit. Pin it FIRST so
//! the in-circuit work has a known-correct target.
//!
//! # Encoding (canonical, bit-exact reproducible)
//!
//! `cf_x_digest = neptune_hash_primary([
//!     limb_lo(P.x), limb_hi(P.x),
//!     limb_lo(P.y), limb_hi(P.y),
//!     primary_scalar(s_step),
//!     limb_lo(Q.x), limb_hi(Q.x),
//!     limb_lo(Q.y), limb_hi(Q.y),
//! ])`
//!
//! Where:
//! - `limb_lo(fq)` = `Bn254Fr(fq.bigint() & ((1<<127)-1))` (the low
//!   127 bits as a Bn254Fr element, always < 2¹²⁷ ≪ Fr modulus).
//! - `limb_hi(fq)` = `Bn254Fr(fq.bigint() >> 127)` (the upper 127
//!   bits; 254-127=127, also < 2¹²⁷).
//! - `primary_scalar(s)` = scalar_adapter same-field conversion
//!   (Bn254Fr ↔ nova `PrimaryScalar`, exact).
//!
//! 127-bit split is chosen because Bn254Fq is ~254 bits and
//! Bn254Fr's modulus is ~254 bits but ever so slightly smaller; a
//! 127-bit cap is comfortably below either modulus and each limb
//! is unambiguously representable as both. Two limbs per Fq value
//! losslessly cover the full 254-bit range.

use crate::neptune_reference::neptune_hash_primary;
use crate::scalar_adapter::{ark_fr_to_primary, primary_to_ark_fr, PrimaryScalar};
use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr, G1Affine};
use ark_ff::{BigInteger, PrimeField};

/// Split a `Bn254Fq` into `(lo, hi)` 127-bit halves represented as
/// `Bn254Fr` elements. Bit-exact and reversible (the bit-level
/// concatenation `hi << 127 | lo` recovers the original Fq value).
fn limb_decompose_fq_to_fr(f: Bn254Fq) -> (Bn254Fr, Bn254Fr) {
    let bits = f.into_bigint().to_bits_le(); // 254 LE bits
    debug_assert!(
        bits.len() >= 127,
        "Bn254Fq bit width must allow 127-bit split"
    );
    let (lo_bits, hi_bits) = bits.split_at(127);
    // hi may have fewer than 127 bits if the value is small; pad
    // to 127 by treating absent bits as 0 (standard LE behaviour).
    let pack_le_to_fr = |bs: &[bool]| -> Bn254Fr {
        let mut acc = Bn254Fr::from(0u64);
        let mut power = Bn254Fr::from(1u64);
        for b in bs {
            if *b {
                acc += power;
            }
            power.double_in_place();
        }
        acc
    };
    (pack_le_to_fr(lo_bits), pack_le_to_fr(hi_bits))
}

/// Compute the native `cf_x_digest` for one step's cross-curve
/// tuple `(P, s, Q)`. Bit-exact, deterministic, the oracle.
///
/// The 4b-β-2 in-circuit gadget will absorb the same 9 Bn254Fr
/// elements (4 limb pairs + the native scalar `s`) into the same
/// Neptune sponge and must produce the same digest. Any divergence
/// = bug in the in-circuit gadget (caught by a future cross-check
/// test that calls both paths on the same inputs).
pub fn compute_cf_x_digest_native(
    p: G1Affine,
    s: Bn254Fr,
    q: G1Affine,
) -> Bn254Fr {
    let (p_x_lo, p_x_hi) = limb_decompose_fq_to_fr(p.x);
    let (p_y_lo, p_y_hi) = limb_decompose_fq_to_fr(p.y);
    let (q_x_lo, q_x_hi) = limb_decompose_fq_to_fr(q.x);
    let (q_y_lo, q_y_hi) = limb_decompose_fq_to_fr(q.y);

    let absorbed_ark: [Bn254Fr; 9] = [
        p_x_lo, p_x_hi, p_y_lo, p_y_hi, s, q_x_lo, q_x_hi, q_y_lo, q_y_hi,
    ];
    let absorbed_nova: Vec<PrimaryScalar> =
        absorbed_ark.iter().copied().map(ark_fr_to_primary).collect();

    primary_to_ark_fr(neptune_hash_primary(&absorbed_nova))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::{AffineRepr, CurveGroup};
    use ark_ff::UniformRand;
    use ark_std::test_rng;

    fn random_tuple() -> (G1Affine, Bn254Fr, G1Affine) {
        let mut rng = test_rng();
        let p = G1Affine::generator();
        let s = Bn254Fr::rand(&mut rng);
        let q = (ark_bn254::G1Projective::from(p) * s).into_affine();
        (p, s, q)
    }

    /// Deterministic: same `(P, s, Q)` ⇒ same digest. Catches a
    /// regression where the Neptune state init leaked rng state.
    #[test]
    fn cf_x_digest_is_deterministic() {
        let (p, s, q) = random_tuple();
        let d1 = compute_cf_x_digest_native(p, s, q);
        let d2 = compute_cf_x_digest_native(p, s, q);
        assert_eq!(d1, d2, "cf_x_digest must be deterministic");
    }

    /// Non-vacuous: different `(P, s, Q)` ⇒ different digest with
    /// overwhelming probability. The binding gate — if the digest
    /// is insensitive to `s`, the primary could fold with a wrong
    /// challenge undetected.
    #[test]
    fn cf_x_digest_distinguishes_distinct_tuples() {
        let mut rng = test_rng();
        let (p, s, q) = random_tuple();
        let d_base = compute_cf_x_digest_native(p, s, q);
        // Change s only ⇒ digest must change.
        let s2 = s + Bn254Fr::from(1u64);
        let q2 = (ark_bn254::G1Projective::from(p) * s2).into_affine();
        let d_s_changed = compute_cf_x_digest_native(p, s2, q2);
        assert_ne!(d_base, d_s_changed, "digest must depend on s");
        // Change Q only (keep P, s same; pick arbitrary unrelated Q).
        let bogus = (ark_bn254::G1Projective::from(p)
            * Bn254Fr::rand(&mut rng))
        .into_affine();
        let d_q_changed = compute_cf_x_digest_native(p, s, bogus);
        assert_ne!(d_base, d_q_changed, "digest must depend on Q");
    }

    /// Limb decomposition is exact: hi<<127 | lo == original Fq.
    /// Pins the encoding so 4b-β-2's in-circuit limb gadget has a
    /// concrete invariant to match.
    #[test]
    fn limb_decomposition_is_lossless() {
        let mut rng = test_rng();
        for _ in 0..16 {
            let f = Bn254Fq::rand(&mut rng);
            let (lo, hi) = limb_decompose_fq_to_fr(f);
            // Reconstruct: hi * 2^127 + lo, computed in Fq via the
            // bigint domain (Fr→Fq via byte transcoding).
            let lo_fq = {
                let bytes = lo.into_bigint().to_bytes_le();
                Bn254Fq::from_le_bytes_mod_order(&bytes)
            };
            let hi_fq = {
                let bytes = hi.into_bigint().to_bytes_le();
                Bn254Fq::from_le_bytes_mod_order(&bytes)
            };
            let two_127 = Bn254Fq::from(2u64).pow([127u64]);
            assert_eq!(
                f,
                hi_fq * two_127 + lo_fq,
                "(hi << 127) | lo must reconstruct the original Fq"
            );
        }
    }
}

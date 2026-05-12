//! Phase 2.3 prep — scalar-type bridge between nova-snark's
//! `halo2curves` field types and arkworks' `ark_bn254` field types.
//!
//! # Why this module exists
//!
//! `RecursiveSNARK<E1=Bn256EngineKZG, E2=GrumpkinEngine, C>` from
//! the chain side carries:
//!
//! - `<E1 as Engine>::Scalar` = `nova_snark::provider::bn256_grumpkin::bn256::Scalar` (BN254 scalar field)
//! - `<E2 as Engine>::Scalar` = `nova_snark::provider::bn256_grumpkin::grumpkin::Scalar` (BN254 base field;
//!   Grumpkin's scalar field is BN254's base field by the curve-cycle
//!   construction)
//!
//! The arkworks verifier circuit operates on:
//!
//! - `ark_bn254::Fr` (BN254 scalar field)
//! - `ark_bn254::Fq` (BN254 base field)
//!
//! Both pairs are the *same mathematical field* — same prime modulus,
//! same arithmetic — but DIFFERENT Rust types with different trait
//! impls. The conversion is byte-level: serialize as 32 little-endian
//! bytes from one side, deserialize on the other.
//!
//! # Where this is used
//!
//! - **Section 2 (Poseidon transcript)**: the adapter pre-converts
//!   each nova scalar in the absorb sequence before feeding it to
//!   the in-circuit Poseidon gadget. `bn256::Fr` → `ark_bn254::Fr`
//!   on the primary side; `grumpkin::Fr` → `ark_bn254::Fq` on the
//!   secondary side (though the wrapper Groth16 proof's public
//!   inputs are all `ark_bn254::Fr`, so the secondary hashes go
//!   through a `Fq → Fr` reduction at adapter time).
//!
//! - **Section 3 (RelaxedR1CS)**: the witness commitments
//!   (`comm_W`, `comm_E`) are curve points on BN256/Grumpkin —
//!   each coordinate is a base-field element. Same byte-level
//!   conversion path.
//!
//! - **Phase 2.3 (the full adapter)**: takes a `CompressedProof`
//!   bytes, deserializes to `RecursiveSNARK<E1, E2, C>`, then
//!   walks every field and runs it through these conversions to
//!   build the `NovaVerifierCircuit` witness.
//!
//! # Correctness invariant
//!
//! For every value `x` representable in both encodings:
//!
//! ```text
//!   bn254_fr_to_nova(nova_to_bn254_fr(x)) == x
//!   nova_to_bn254_fr(bn254_fr_to_nova(y)) == y
//! ```
//!
//! Property tests below exercise this for 1024 random scalars per
//! direction plus the three boundary values `0`, `1`, `p-1`.

use ark_ff::{BigInteger, PrimeField};
use ff::PrimeField as _;

/// Convert a nova-snark `bn256::Fr` scalar (BN254 scalar field) to
/// `ark_bn254::Fr`.
///
/// Both types represent the same prime field. The conversion goes
/// through 32 little-endian bytes — the standard `to_repr` /
/// `from_le_bytes_mod_order` round-trip.
pub fn nova_to_bn254_fr(src: &nova_snark::provider::bn256_grumpkin::bn256::Scalar) -> ark_bn254::Fr {
    let bytes: [u8; 32] = src.to_repr().into();
    ark_bn254::Fr::from_le_bytes_mod_order(&bytes)
}

/// Convert an `ark_bn254::Fr` scalar to nova-snark's
/// `nova_snark::provider::bn256_grumpkin::bn256::Scalar`.
///
/// # Panics
///
/// Panics if the input scalar's canonical byte representation does
/// not deserialize via `nova_snark::provider::bn256_grumpkin::bn256::Scalar::from_repr_vartime`.
/// In practice this cannot happen: both types canonicalize to
/// `[u8; 32]` little-endian in `[0, p)` where `p` is the BN254
/// scalar-field modulus.
pub fn bn254_fr_to_nova(src: &ark_bn254::Fr) -> nova_snark::provider::bn256_grumpkin::bn256::Scalar {
    let bigint = src.into_bigint();
    let bytes_le = bigint.to_bytes_le();
    let mut arr = [0u8; 32];
    let copy_len = bytes_le.len().min(32);
    arr[..copy_len].copy_from_slice(&bytes_le[..copy_len]);
    nova_snark::provider::bn256_grumpkin::bn256::Scalar::from_repr_vartime(arr.into())
        .expect("ark_bn254::Fr canonical bytes must deserialize to nova_snark::provider::bn256_grumpkin::bn256::Scalar — same field, same modulus")
}

/// Convert a nova-snark `grumpkin::Fr` scalar (BN254 base field) to
/// `ark_bn254::Fq`. Used for Section 3 curve-point coordinates and
/// the secondary-side Poseidon transcript.
pub fn nova_grumpkin_to_bn254_fq(src: &nova_snark::provider::bn256_grumpkin::grumpkin::Scalar) -> ark_bn254::Fq {
    let bytes: [u8; 32] = src.to_repr().into();
    ark_bn254::Fq::from_le_bytes_mod_order(&bytes)
}

/// Convert an `ark_bn254::Fq` scalar to nova-snark's
/// `nova_snark::provider::bn256_grumpkin::grumpkin::Scalar`. Inverse of
/// [`nova_grumpkin_to_bn254_fq`].
pub fn bn254_fq_to_nova_grumpkin(src: &ark_bn254::Fq) -> nova_snark::provider::bn256_grumpkin::grumpkin::Scalar {
    let bigint = src.into_bigint();
    let bytes_le = bigint.to_bytes_le();
    let mut arr = [0u8; 32];
    let copy_len = bytes_le.len().min(32);
    arr[..copy_len].copy_from_slice(&bytes_le[..copy_len]);
    nova_snark::provider::bn256_grumpkin::grumpkin::Scalar::from_repr_vartime(arr.into())
        .expect("ark_bn254::Fq canonical bytes must deserialize to nova_snark::provider::bn256_grumpkin::grumpkin::Scalar — same field, same modulus")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ff::Field;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    #[test]
    fn bn254_fr_round_trip_zero_one_max() {
        // Boundary values.
        let zero = ark_bn254::Fr::from(0u64);
        let one = ark_bn254::Fr::from(1u64);
        // `p - 1` — the largest canonical element. Constructed as
        // -1 in the additive group.
        let neg_one = -one;

        for x in [zero, one, neg_one] {
            let n = bn254_fr_to_nova(&x);
            let back = nova_to_bn254_fr(&n);
            assert_eq!(x, back, "ark→nova→ark identity failed for {x:?}");
        }
    }

    #[test]
    fn bn254_fr_round_trip_ark_to_nova_to_ark() {
        // 1024 random scalars: ark → nova → ark must be identity.
        let mut rng = StdRng::seed_from_u64(0xa1ca_da12_0e4f_ba12);
        for _ in 0..1024 {
            let bytes: [u8; 32] = rng.gen();
            let x = ark_bn254::Fr::from_le_bytes_mod_order(&bytes);
            let n = bn254_fr_to_nova(&x);
            let back = nova_to_bn254_fr(&n);
            assert_eq!(x, back);
        }
    }

    #[test]
    fn bn254_fr_round_trip_nova_to_ark_to_nova() {
        // 1024 random scalars: nova → ark → nova must be identity.
        let mut rng = StdRng::seed_from_u64(0xa1ca_da12_0e4f_ba13);
        for _ in 0..1024 {
            let x = nova_snark::provider::bn256_grumpkin::bn256::Scalar::random(&mut rng);
            let a = nova_to_bn254_fr(&x);
            let back = bn254_fr_to_nova(&a);
            assert_eq!(x, back);
        }
    }

    #[test]
    fn bn254_fq_round_trip_grumpkin_boundaries() {
        let zero = ark_bn254::Fq::from(0u64);
        let one = ark_bn254::Fq::from(1u64);
        let neg_one = -one;
        for x in [zero, one, neg_one] {
            let g = bn254_fq_to_nova_grumpkin(&x);
            let back = nova_grumpkin_to_bn254_fq(&g);
            assert_eq!(x, back);
        }
    }

    #[test]
    fn bn254_fq_round_trip_ark_to_grumpkin_to_ark() {
        let mut rng = StdRng::seed_from_u64(0xb0ca_da12_0e4f_ba12);
        for _ in 0..1024 {
            let bytes: [u8; 32] = rng.gen();
            let x = ark_bn254::Fq::from_le_bytes_mod_order(&bytes);
            let g = bn254_fq_to_nova_grumpkin(&x);
            let back = nova_grumpkin_to_bn254_fq(&g);
            assert_eq!(x, back);
        }
    }

    #[test]
    fn bn254_fq_round_trip_grumpkin_to_ark_to_grumpkin() {
        let mut rng = StdRng::seed_from_u64(0xb0ca_da12_0e4f_ba13);
        for _ in 0..1024 {
            let x = nova_snark::provider::bn256_grumpkin::grumpkin::Scalar::random(&mut rng);
            let a = nova_grumpkin_to_bn254_fq(&x);
            let back = bn254_fq_to_nova_grumpkin(&a);
            assert_eq!(x, back);
        }
    }

    #[test]
    fn conversion_preserves_known_constant_one() {
        // Sanity check: the multiplicative identity round-trips with
        // the same numeric value on both sides, not just the same
        // bytes. (Catches a hypothetical "to_repr returns Montgomery
        // form" mismatch.)
        let ark_one = ark_bn254::Fr::from(1u64);
        let nova_one = bn254_fr_to_nova(&ark_one);
        // `nova_snark::provider::bn256_grumpkin::bn256::Scalar::ONE` in canonical form.
        let nova_one_direct = nova_snark::provider::bn256_grumpkin::bn256::Scalar::ONE;
        assert_eq!(nova_one, nova_one_direct);

        // And in the other direction.
        let back = nova_to_bn254_fr(&nova_one_direct);
        assert_eq!(back, ark_one);
    }

    #[test]
    fn conversion_preserves_addition() {
        // For arbitrary a, b: convert separately and add on the
        // nova side; OR add on the ark side and then convert.
        // Both routes must produce the same scalar.
        let mut rng = StdRng::seed_from_u64(0xc1ca_da12_0e4f_ba12);
        for _ in 0..128 {
            let bytes_a: [u8; 32] = rng.gen();
            let bytes_b: [u8; 32] = rng.gen();
            let a = ark_bn254::Fr::from_le_bytes_mod_order(&bytes_a);
            let b = ark_bn254::Fr::from_le_bytes_mod_order(&bytes_b);

            let na = bn254_fr_to_nova(&a);
            let nb = bn254_fr_to_nova(&b);

            let sum_in_ark = a + b;
            let sum_in_nova = na + nb;

            assert_eq!(bn254_fr_to_nova(&sum_in_ark), sum_in_nova);
            assert_eq!(nova_to_bn254_fr(&sum_in_nova), sum_in_ark);
        }
    }
}

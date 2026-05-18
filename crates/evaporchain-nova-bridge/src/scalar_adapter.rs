//! Scalar-type bridge between nova-snark's native field types
//! (halo2curves `bn256::Scalar`, `grumpkin::Scalar`) and the
//! arkworks-side `ark_bn254::Fr` the [`crate::verifier_circuit`]
//! operates over.
//!
//! # Why this module exists
//!
//! `RecursiveSNARK<E1=Bn256EngineKZG, E2=GrumpkinEngine, C>` carries
//! its witness values in nova's native scalar types. The arkworks
//! verifier circuit operates over `ark_bn254::Fr`. The two sides
//! agree on field arithmetic but disagree on Rust types, so a
//! deliberate adapter is needed at the boundary.
//!
//! # Field relationships
//!
//! - **Primary** (nova `bn256::Scalar` ↔ `ark_bn254::Fr`):
//!   These are the **same prime field** (the BN254 scalar field,
//!   modulus `21888242871839275222246405745257275088548364400416034343698204186575808495617`).
//!   Conversion is a pure type-change via LE bytes; round-trip is
//!   byte-lossless and value-preserving for every scalar in the
//!   field.
//!
//! - **Secondary** (nova `grumpkin::Scalar` ↔ `ark_bn254::Fr`):
//!   These are **different prime fields** — grumpkin's scalar
//!   field is BN254's *base* field (`Fq`). Same bit-width (254
//!   bits) but a different modulus, so reading grumpkin bytes as
//!   `ark_bn254::Fr` performs an implicit reduction mod the BN254
//!   scalar modulus. Values larger than the BN254 scalar modulus
//!   wrap — the conversion is therefore **lossy in general** and
//!   only round-trips for values that fit in both moduli.
//!
//!   The L1 Solidity verifier reads the committed hash from
//!   `l_u_secondary.X[1]` (a grumpkin scalar) as a 32-byte BE
//!   blob and treats it byte-for-byte. As long as both prover
//!   and verifier go through *this* adapter, the canonical byte
//!   form they see is identical, so the verification contract
//!   holds.
//!
//! # Adapter contract
//!
//! - LE byte order throughout. `halo2curves::ff::PrimeField::to_repr()`
//!   returns LE; `ark_ff::BigInteger::to_bytes_le()` returns LE.
//! - The `Repr` type from halo2curves is a 32-byte newtype; we go
//!   via `[u8; 32]` to keep the conversion explicit at every step.

use ark_bn254::Fq as ArkFq;
use ark_bn254::Fr as ArkFr;
use ark_ff::{BigInteger, PrimeField as ArkPrimeField};
use ff::PrimeField as FfPrimeField;

/// Type alias for the **primary**-side nova scalar (same field as
/// `ark_bn254::Fr`).
pub type PrimaryScalar = nova_snark::provider::bn256_grumpkin::bn256::Scalar;

/// Type alias for the **secondary**-side nova scalar (BN254's base
/// field; **not** the same field as `ark_bn254::Fr`).
pub type SecondaryScalar = nova_snark::provider::bn256_grumpkin::grumpkin::Scalar;

/// Same-field type conversion: nova `bn256::Scalar` → `ark_bn254::Fr`.
///
/// Pure byte transcoding; byte-lossless and value-preserving for
/// every input. Use the round-trip pair
/// [`ark_fr_to_primary`] / [`primary_to_ark_fr`] when bridging the
/// `RecursiveSNARK::z0` / `zi` / primary-hash witness slots into
/// the arkworks verifier circuit.
pub fn primary_to_ark_fr(s: PrimaryScalar) -> ArkFr {
    let repr: [u8; 32] = s.to_repr().into();
    ArkFr::from_le_bytes_mod_order(&repr)
}

/// Same-field type conversion: `ark_bn254::Fr` → nova `bn256::Scalar`.
///
/// Inverse of [`primary_to_ark_fr`]. Round-trip is byte-lossless
/// for every scalar in the BN254 scalar field.
pub fn ark_fr_to_primary(f: ArkFr) -> PrimaryScalar {
    let bigint_le = f.into_bigint().to_bytes_le();
    let mut bytes = [0u8; 32];
    let len = bigint_le.len().min(32);
    bytes[..len].copy_from_slice(&bigint_le[..len]);
    let repr = <PrimaryScalar as FfPrimeField>::Repr::from(bytes);
    PrimaryScalar::from_repr_vartime(repr)
        .expect("ark Fr value must canonicalise to a valid bn256::Scalar (same field)")
}

/// Cross-field byte transcoding: nova `grumpkin::Scalar` →
/// `ark_bn254::Fr` via LE-bytes-mod-order.
///
/// **Lossy in general.** grumpkin's scalar field is BN254's base
/// field, which has a different modulus from BN254's scalar
/// field. Inputs larger than the BN254 scalar modulus undergo
/// implicit reduction. This is the documented behaviour for
/// `l_u_secondary.X[..]` committed hashes: the L1 verifier reads
/// them as raw 32-byte blobs, and as long as prover and verifier
/// go through this adapter the canonical bytes match.
///
/// Round-trip via [`ark_fr_to_secondary_lossy`] is **not**
/// guaranteed; see that function's documentation.
pub fn secondary_to_ark_fr_lossy(s: SecondaryScalar) -> ArkFr {
    let repr: [u8; 32] = s.to_repr().into();
    ArkFr::from_le_bytes_mod_order(&repr)
}

/// Same-field conversion: nova `grumpkin::Scalar` → `ark_bn254::Fq`.
///
/// grumpkin's scalar field **is** BN254's base field, so this is
/// **exact and value-preserving for every input** — the genuine
/// same-field analog of [`primary_to_ark_fr`], NOT the lossy
/// [`secondary_to_ark_fr_lossy`] (which reduces mod the BN254
/// *scalar* modulus and is wrong for arithmetic-bearing values).
///
/// Audit B-1/B-2 S4a: feeds real secondary `RelaxedR1CSWitness`
/// scalars (`W`, `r_W`) into the Grumpkin Pedersen-MSM gadget
/// ([`crate::s4_msm_gadget`]), whose scalars are
/// `EmulatedFpVar<Fq, Fr>` — they must be the exact Fq values, not
/// Fr-reduced.
pub fn secondary_to_ark_fq(s: SecondaryScalar) -> ArkFq {
    let repr: [u8; 32] = s.to_repr().into();
    ArkFq::from_le_bytes_mod_order(&repr)
}

/// Cross-field byte transcoding: `ark_bn254::Fr` → nova
/// `grumpkin::Scalar` via LE-bytes-mod-order.
///
/// **Lossy in general** — symmetric to [`secondary_to_ark_fr_lossy`].
/// Round-trip `secondary → ark → secondary` only preserves
/// values that fit in both moduli.
pub fn ark_fr_to_secondary_lossy(f: ArkFr) -> SecondaryScalar {
    let bigint_le = f.into_bigint().to_bytes_le();
    let mut bytes = [0u8; 32];
    let len = bigint_le.len().min(32);
    bytes[..len].copy_from_slice(&bigint_le[..len]);
    let repr = <SecondaryScalar as FfPrimeField>::Repr::from(bytes);
    SecondaryScalar::from_repr_vartime(repr)
        .expect(
            "ark Fr value must canonicalise to a valid grumpkin::Scalar via byte read; \
             every byte sequence ≤ 254 bits maps to *some* grumpkin::Scalar via the \
             halo2curves reduction. If this fires the BN254 Fr value somehow \
             exceeds 32 LE bytes after canonical reduction, which is impossible.",
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ff::Field;

    /// Primary round-trip on small values: every small integer
    /// converts identically through ark and back.
    #[test]
    fn primary_round_trip_small_values() {
        for v in [0u64, 1, 2, 7, 42, 99, 100_000, u64::MAX] {
            let nova_in = PrimaryScalar::from(v);
            let ark = primary_to_ark_fr(nova_in);
            let nova_out = ark_fr_to_primary(ark);
            assert_eq!(nova_in, nova_out, "primary round-trip failed for {v}");
            assert_eq!(
                ark,
                ArkFr::from(v),
                "primary→ark must equal ArkFr::from({v}) for small value"
            );
        }
    }

    /// Primary zero / one constants resolve identically in both
    /// directions across both type systems.
    #[test]
    fn primary_zero_one_constants_match() {
        assert_eq!(primary_to_ark_fr(PrimaryScalar::ZERO), ArkFr::from(0u64));
        assert_eq!(primary_to_ark_fr(PrimaryScalar::ONE), ArkFr::from(1u64));
        assert_eq!(ark_fr_to_primary(ArkFr::from(0u64)), PrimaryScalar::ZERO);
        assert_eq!(ark_fr_to_primary(ArkFr::from(1u64)), PrimaryScalar::ONE);
    }

    /// Pinned LE-byte form: confirm both type systems agree on the
    /// canonical byte representation of `42`. This catches any
    /// future endianness drift in either crate.
    #[test]
    fn primary_canonical_bytes_pinned_42() {
        let nova = PrimaryScalar::from(42u64);
        let ark = primary_to_ark_fr(nova);

        let nova_le: [u8; 32] = nova.to_repr().into();
        let ark_le = ark.into_bigint().to_bytes_le();
        let mut ark_le_32 = [0u8; 32];
        ark_le_32[..ark_le.len().min(32)].copy_from_slice(&ark_le[..ark_le.len().min(32)]);

        assert_eq!(nova_le, ark_le_32, "primary LE bytes must agree at value 42");
        // Pin the actual bytes: 42 LE = [0x2a, 0, 0, ..., 0]
        let mut expected = [0u8; 32];
        expected[0] = 42;
        assert_eq!(nova_le, expected, "value 42 must serialize as [0x2a, 0, …, 0]");
    }

    /// Secondary "small value" lossy bridge round-trips for values
    /// that fit in both moduli (small u64s definitely do).
    #[test]
    fn secondary_round_trip_small_values_under_both_moduli() {
        for v in [0u64, 1, 2, 7, 42, 99, 100_000, u64::MAX] {
            let nova_in = SecondaryScalar::from(v);
            let ark = secondary_to_ark_fr_lossy(nova_in);
            let nova_out = ark_fr_to_secondary_lossy(ark);
            assert_eq!(
                nova_in, nova_out,
                "secondary round-trip failed for small value {v} (should fit both moduli)"
            );
        }
    }

    /// Secondary zero / one constants resolve identically.
    #[test]
    fn secondary_zero_one_constants_match() {
        assert_eq!(
            secondary_to_ark_fr_lossy(SecondaryScalar::ZERO),
            ArkFr::from(0u64)
        );
        assert_eq!(
            secondary_to_ark_fr_lossy(SecondaryScalar::ONE),
            ArkFr::from(1u64)
        );
    }

    /// Sanity: bn256 and grumpkin scalar moduli are **different**
    /// despite both being 254-bit prime fields. This pins our
    /// understanding for the "lossy in general" semantics — if
    /// this ever flips, the secondary adapter could be tightened
    /// to byte-lossless.
    #[test]
    fn bn256_and_grumpkin_moduli_differ() {
        // -1 (= modulus - 1) serializes differently in each field.
        let bn256_neg_one_le: [u8; 32] = (-PrimaryScalar::ONE).to_repr().into();
        let grumpkin_neg_one_le: [u8; 32] = (-SecondaryScalar::ONE).to_repr().into();
        assert_ne!(
            bn256_neg_one_le, grumpkin_neg_one_le,
            "bn256 and grumpkin scalar moduli must differ — if this fires our \
             lossy-secondary assumption is wrong and the adapter can be tightened"
        );
    }

    /// S4a: `secondary_to_ark_fq` is exact / value-preserving for
    /// every input (same field), unlike the lossy Fr path.
    #[test]
    fn secondary_to_ark_fq_is_exact_value_preserving() {
        for v in [0u64, 1, 2, 42, 7919, u64::MAX] {
            assert_eq!(
                secondary_to_ark_fq(SecondaryScalar::from(v)),
                ArkFq::from(v),
                "secondary_to_ark_fq must be value-preserving for {v}"
            );
        }
        // q − 1 (Fq additive inverse of 1) must round-trip as the
        // TRUE Fq −1 — i.e. `(q−1) + 1 == 0` in Fq. The lossy Fr path
        // could not preserve this (q−1 > r).
        let neg1_fq = secondary_to_ark_fq(-SecondaryScalar::from(1u64));
        assert_eq!(
            neg1_fq + ArkFq::from(1u64),
            ArkFq::from(0u64),
            "q-1 must map to the true Fq additive inverse of 1 (exactness)"
        );
    }
}

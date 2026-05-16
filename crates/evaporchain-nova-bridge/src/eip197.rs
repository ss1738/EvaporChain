//! Phase 2.5 starter — EIP-197 wire-format encoder/decoder for the
//! 256-byte Groth16 proof the L1 Solidity verifier consumes.
//!
//! # Wire format (256 bytes total)
//!
//! ```text
//!   bytes  0..  32   A.x         (G1, 32-byte BE)
//!   bytes 32..  64   A.y         (G1, 32-byte BE)
//!   bytes 64..  96   B.x.c1      (G2, 32-byte BE)   ← Fq2 c1 first
//!   bytes 96.. 128   B.x.c0      (G2, 32-byte BE)   ← Fq2 c0 second
//!   bytes 128..160   B.y.c1      (G2, 32-byte BE)   ← Fq2 c1 first
//!   bytes 160..192   B.y.c0      (G2, 32-byte BE)   ← Fq2 c0 second
//!   bytes 192..224   C.x         (G1, 32-byte BE)
//!   bytes 224..256   C.y         (G1, 32-byte BE)
//! ```
//!
//! ## The Fq2 (c1, c0) swap
//!
//! Ethereum's BN254 G2 representation flips the natural Fq2 polynomial
//! order — the precompile expects c1 (the X coefficient) before c0
//! (the constant term). arkworks's natural `to_bytes` order is
//! (c0, c1). This module's encoder performs the swap.
//!
//! Forgetting this swap was the original failure mode that motivated
//! a dedicated codec module rather than calling
//! `proof.serialize_compressed` directly.
//!
//! # No header / no length prefix
//!
//! The L1 verifier treats the 256-byte slice as a fixed-length
//! tuple, not as serde. There is no preamble byte, no length
//! prefix, no padding. Producers must emit exactly 256 bytes;
//! consumers must reject anything else.

use ark_bn254::{Bn254, Fq, Fq2, G1Affine, G2Affine};
use ark_ff::{BigInteger, PrimeField};
use ark_groth16::Proof;

/// Number of bytes a single Fq element occupies in big-endian
/// fixed-width form. BN254's prime is 254 bits, so 32 bytes is
/// enough with leading zero padding for small values.
pub const FQ_BYTES_BE: usize = 32;

/// Total wire-format length of an EIP-197 Groth16 proof: A (G1, 64)
/// + B (G2, 128) + C (G1, 64) = 256 bytes.
pub const EIP197_PROOF_BYTES: usize = 256;

/// Errors surfaced by [`eip197_bytes_to_proof`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Eip197DecodeError {
    /// Input slice was not exactly [`EIP197_PROOF_BYTES`] long.
    #[error("EIP-197 proof must be {EIP197_PROOF_BYTES} bytes; got {0}")]
    WrongLength(usize),

    /// One of the eight 32-byte limbs failed to round-trip into a
    /// canonical Fq element (value ≥ field modulus).
    #[error("EIP-197 limb at byte-offset {offset} is not a canonical Fq element")]
    NonCanonicalLimb {
        /// Byte offset of the limb that failed to canonicalise.
        offset: usize,
    },

    /// One of the points decoded but is not on the curve.
    #[error("EIP-197 {which} point is not on the BN254 curve")]
    PointNotOnCurve {
        /// Which named point failed (`"A"` G1, `"B"` G2, or `"C"` G1).
        which: &'static str,
    },
}

/// Encode an arkworks Groth16 proof to the 256-byte EIP-197 wire
/// format the L1 Solidity verifier reads.
///
/// **Round-trip pair** of [`eip197_bytes_to_proof`]; both honour
/// the Fq2 (c1, c0) swap on the G2 coordinates.
pub fn proof_to_eip197_bytes(proof: &Proof<Bn254>) -> [u8; EIP197_PROOF_BYTES] {
    let mut out = [0u8; EIP197_PROOF_BYTES];

    // A (G1): bytes 0..64
    write_g1_be(&proof.a, &mut out[0..64]);
    // B (G2): bytes 64..192 — c1, c0 order (the EIP-197 swap)
    write_g2_be_swapped(&proof.b, &mut out[64..192]);
    // C (G1): bytes 192..256
    write_g1_be(&proof.c, &mut out[192..256]);

    out
}

/// Decode a 256-byte EIP-197 blob back to an arkworks Groth16 proof.
///
/// Validates: length (256), every limb canonicalises, every point
/// lies on the BN254 curve.
pub fn eip197_bytes_to_proof(bytes: &[u8]) -> Result<Proof<Bn254>, Eip197DecodeError> {
    if bytes.len() != EIP197_PROOF_BYTES {
        return Err(Eip197DecodeError::WrongLength(bytes.len()));
    }

    let a = read_g1_be(&bytes[0..64], 0)?;
    if !a.is_on_curve() {
        return Err(Eip197DecodeError::PointNotOnCurve { which: "A" });
    }

    let b = read_g2_be_swapped(&bytes[64..192], 64)?;
    if !b.is_on_curve() {
        return Err(Eip197DecodeError::PointNotOnCurve { which: "B" });
    }

    let c = read_g1_be(&bytes[192..256], 192)?;
    if !c.is_on_curve() {
        return Err(Eip197DecodeError::PointNotOnCurve { which: "C" });
    }

    Ok(Proof { a, b, c })
}

// ── Helpers ───────────────────────────────────────────────────

fn write_fq_be(f: &Fq, dst: &mut [u8]) {
    debug_assert_eq!(dst.len(), FQ_BYTES_BE);
    let mut le = f.into_bigint().to_bytes_le();
    le.resize(FQ_BYTES_BE, 0);
    le.reverse();
    dst.copy_from_slice(&le);
}

fn read_fq_be(src: &[u8], abs_offset: usize) -> Result<Fq, Eip197DecodeError> {
    debug_assert_eq!(src.len(), FQ_BYTES_BE);
    let mut le = [0u8; FQ_BYTES_BE];
    le.copy_from_slice(src);
    le.reverse();
    // Round-trip check: the value must canonicalise to itself.
    let candidate = Fq::from_le_bytes_mod_order(&le);
    let canon_le = candidate.into_bigint().to_bytes_le();
    let mut canon_padded = [0u8; FQ_BYTES_BE];
    canon_padded[..canon_le.len().min(FQ_BYTES_BE)].copy_from_slice(&canon_le);
    if canon_padded != le {
        return Err(Eip197DecodeError::NonCanonicalLimb { offset: abs_offset });
    }
    Ok(candidate)
}

fn write_g1_be(p: &G1Affine, dst: &mut [u8]) {
    debug_assert_eq!(dst.len(), 64);
    write_fq_be(&p.x, &mut dst[0..32]);
    write_fq_be(&p.y, &mut dst[32..64]);
}

fn read_g1_be(src: &[u8], abs_offset: usize) -> Result<G1Affine, Eip197DecodeError> {
    debug_assert_eq!(src.len(), 64);
    let x = read_fq_be(&src[0..32], abs_offset)?;
    let y = read_fq_be(&src[32..64], abs_offset + 32)?;
    Ok(G1Affine::new_unchecked(x, y))
}

fn write_g2_be_swapped(p: &G2Affine, dst: &mut [u8]) {
    debug_assert_eq!(dst.len(), 128);
    // The c1, c0 swap is here — Ethereum precompile expects c1 first.
    write_fq_be(&p.x.c1, &mut dst[0..32]);
    write_fq_be(&p.x.c0, &mut dst[32..64]);
    write_fq_be(&p.y.c1, &mut dst[64..96]);
    write_fq_be(&p.y.c0, &mut dst[96..128]);
}

fn read_g2_be_swapped(src: &[u8], abs_offset: usize) -> Result<G2Affine, Eip197DecodeError> {
    debug_assert_eq!(src.len(), 128);
    let x_c1 = read_fq_be(&src[0..32], abs_offset)?;
    let x_c0 = read_fq_be(&src[32..64], abs_offset + 32)?;
    let y_c1 = read_fq_be(&src[64..96], abs_offset + 64)?;
    let y_c0 = read_fq_be(&src[96..128], abs_offset + 96)?;
    Ok(G2Affine::new_unchecked(
        Fq2::new(x_c0, x_c1),
        Fq2::new(y_c0, y_c1),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::groth16_wrapper::{prove, setup};
    use crate::verifier_circuit::NovaVerifierCircuit;
    use ark_std::rand::SeedableRng;

    /// End-to-end round-trip: prove(dummy) → eip197 encode →
    /// eip197 decode → deep-equal original proof.
    #[test]
    fn proof_round_trips_through_eip197_bytes() {
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(303);
        let (pk, _vk) = setup(&mut rng).expect("setup");
        let proof = prove(&pk, NovaVerifierCircuit::dummy(), &mut rng).expect("prove");

        let bytes = proof_to_eip197_bytes(&proof);
        assert_eq!(bytes.len(), EIP197_PROOF_BYTES);

        let decoded = eip197_bytes_to_proof(&bytes).expect("decode");
        assert_eq!(decoded.a, proof.a, "A round-trip");
        assert_eq!(decoded.b, proof.b, "B round-trip with Fq2 swap");
        assert_eq!(decoded.c, proof.c, "C round-trip");

        // Re-encode the decoded proof: byte-stable.
        let bytes_back = proof_to_eip197_bytes(&decoded);
        assert_eq!(bytes, bytes_back, "encode → decode → encode is byte-stable");
    }

    /// Wrong-length input must be rejected with a typed error.
    #[test]
    fn decode_rejects_wrong_length() {
        let short = vec![0u8; 100];
        assert_eq!(
            eip197_bytes_to_proof(&short),
            Err(Eip197DecodeError::WrongLength(100)),
        );
        let long = vec![0u8; 300];
        assert_eq!(
            eip197_bytes_to_proof(&long),
            Err(Eip197DecodeError::WrongLength(300)),
        );
    }

    /// All-FF input cannot canonicalise — 32 bytes of 0xff exceeds
    /// the BN254 base-field modulus. The first limb (A.x) should
    /// fire `NonCanonicalLimb { offset: 0 }`.
    #[test]
    fn decode_rejects_non_canonical_limb() {
        let bytes = [0xffu8; EIP197_PROOF_BYTES];
        match eip197_bytes_to_proof(&bytes) {
            Err(Eip197DecodeError::NonCanonicalLimb { offset }) => {
                assert_eq!(offset, 0, "first limb at offset 0 should be the failing one");
            }
            other => panic!("expected NonCanonicalLimb at offset 0, got {other:?}"),
        }
    }

    /// Generator points encode + decode + lie on the curve. This
    /// is the cheapest possible sanity check that the codec
    /// preserves curve membership.
    #[test]
    fn generator_points_round_trip_and_stay_on_curve() {
        use ark_ec::AffineRepr;
        let proof = Proof::<Bn254> {
            a: G1Affine::generator(),
            b: G2Affine::generator(),
            c: G1Affine::generator(),
        };
        let bytes = proof_to_eip197_bytes(&proof);
        let decoded = eip197_bytes_to_proof(&bytes).expect("decode generator");
        assert_eq!(decoded.a, proof.a);
        assert_eq!(decoded.b, proof.b);
        assert_eq!(decoded.c, proof.c);
        assert!(decoded.a.is_on_curve());
        assert!(decoded.b.is_on_curve());
        assert!(decoded.c.is_on_curve());
    }

    /// `PointNotOnCurve { which: "A" }` fires when bytes decode to a
    /// canonical (x, y) pair that doesn't satisfy y² = x³ + 3.
    #[test]
    fn decode_rejects_off_curve_a() {
        use ark_ec::AffineRepr;
        let proof = Proof::<Bn254> {
            a: G1Affine::generator(),
            b: G2Affine::generator(),
            c: G1Affine::generator(),
        };
        let mut bytes = proof_to_eip197_bytes(&proof);
        // Replace A with (1, 1). 1² = 1; 1³ + 3 = 4. Off curve.
        bytes[0..32].fill(0);
        bytes[31] = 1;
        bytes[32..64].fill(0);
        bytes[63] = 1;
        match eip197_bytes_to_proof(&bytes) {
            Err(Eip197DecodeError::PointNotOnCurve { which }) => assert_eq!(which, "A"),
            other => panic!("expected PointNotOnCurve A, got {other:?}"),
        }
    }

    /// `PointNotOnCurve { which: "C" }` fires when C decodes as
    /// canonical but is not on the curve.
    #[test]
    fn decode_rejects_off_curve_c() {
        use ark_ec::AffineRepr;
        let proof = Proof::<Bn254> {
            a: G1Affine::generator(),
            b: G2Affine::generator(),
            c: G1Affine::generator(),
        };
        let mut bytes = proof_to_eip197_bytes(&proof);
        bytes[192..224].fill(0);
        bytes[223] = 1;
        bytes[224..256].fill(0);
        bytes[255] = 1;
        match eip197_bytes_to_proof(&bytes) {
            Err(Eip197DecodeError::PointNotOnCurve { which }) => assert_eq!(which, "C"),
            other => panic!("expected PointNotOnCurve C, got {other:?}"),
        }
    }

    #[test]
    fn decode_error_displays_all_variants() {
        assert!(Eip197DecodeError::WrongLength(100).to_string().contains("100"));
        assert!(Eip197DecodeError::NonCanonicalLimb { offset: 64 }
            .to_string()
            .contains("64"));
        assert!(Eip197DecodeError::PointNotOnCurve { which: "B" }
            .to_string()
            .contains("B"));
    }

    #[test]
    fn proof_size_consts_match_spec() {
        assert_eq!(FQ_BYTES_BE, 32);
        assert_eq!(EIP197_PROOF_BYTES, 256);
        // 8 limbs of 32 bytes = 256 (A=2, B=4, C=2).
        assert_eq!(8 * FQ_BYTES_BE, EIP197_PROOF_BYTES);
    }

    #[test]
    fn decode_error_clone_eq_works() {
        let e = Eip197DecodeError::WrongLength(7);
        assert_eq!(e.clone(), e);
        assert_ne!(e, Eip197DecodeError::WrongLength(8));
    }

    /// Pin the Fq2 (c1, c0) swap explicitly. After encode, the
    /// G2 X-coordinate's c1 limb must sit at bytes 64..96 (the
    /// FIRST G2 slot), not 96..128. A regression here would
    /// silently mis-encode every proof going to L1.
    #[test]
    fn fq2_swap_pins_c1_before_c0() {
        use ark_ec::AffineRepr;
        let proof = Proof::<Bn254> {
            a: G1Affine::generator(),
            b: G2Affine::generator(),
            c: G1Affine::generator(),
        };
        let bytes = proof_to_eip197_bytes(&proof);

        // Manually serialise the G2 generator's x.c1 in BE-32 and
        // confirm it matches bytes 64..96 (which is supposed to
        // hold x.c1, not x.c0).
        let mut expected_xc1_be = G2Affine::generator()
            .x
            .c1
            .into_bigint()
            .to_bytes_le();
        expected_xc1_be.resize(32, 0);
        expected_xc1_be.reverse();

        assert_eq!(
            &bytes[64..96],
            expected_xc1_be.as_slice(),
            "bytes[64..96] must hold G2.x.c1 — Fq2 (c1, c0) swap must be present"
        );

        // And bytes 96..128 must hold x.c0, NOT x.c1.
        let mut expected_xc0_be = G2Affine::generator()
            .x
            .c0
            .into_bigint()
            .to_bytes_le();
        expected_xc0_be.resize(32, 0);
        expected_xc0_be.reverse();
        assert_eq!(
            &bytes[96..128],
            expected_xc0_be.as_slice(),
            "bytes[96..128] must hold G2.x.c0",
        );
    }
}

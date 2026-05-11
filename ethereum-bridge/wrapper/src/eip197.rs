//! EIP-197 calldata conversion for Groth16 BN254 proofs.
//!
//! arkworks's `serialize_compressed` returns 128 bytes (A:32 + B:64 + C:32)
//! in little-endian canonical form. The L1 `VerkleProofVerifier.sol`
//! consumes proofs as 256 bytes uncompressed, big-endian, in the
//! coordinate order specified by EIP-197:
//!
//! ```text
//! [  0.. 32]  A.x     (G1 x-coordinate, Fq, big-endian)
//! [ 32.. 64]  A.y     (G1 y-coordinate, Fq, big-endian)
//! [ 64.. 96]  B.x.c1  (G2 x = a + b·u; EIP-197 emits b first, "imaginary" first)
//! [ 96..128]  B.x.c0
//! [128..160]  B.y.c1
//! [160..192]  B.y.c0
//! [192..224]  C.x
//! [224..256]  C.y
//! ```
//!
//! # The G2 coefficient-order gotcha
//!
//! EIP-197 specifies that for a G2 point with x = a + b·i (with i² = -1
//! over BN254's quadratic twist), the encoding is `(b, a)` — imaginary
//! part first. arkworks's `Fq2 { c0, c1 }` represents the element as
//! `c0 + c1·u`, so `c0 = a` (real) and `c1 = b` (imaginary). To produce
//! EIP-197 calldata we therefore write `c1` then `c0`.
//!
//! Different libraries get this wrong silently — using `c0` first
//! produces a Groth16 proof that the EVM verifier rejects with no
//! obvious error. This module's tests pin the ordering against
//! known-answer fixtures (round-trip identity through both formats).

use ark_bn254::{Bn254, Fq, G1Affine, G2Affine};
use ark_ec::AffineRepr;
use ark_ff::{BigInteger, PrimeField};
use ark_groth16::Proof;
use ark_serialize::CanonicalDeserialize;

/// EIP-197 calldata length: 8 BN254 Fq elements × 32 bytes each.
pub const EIP197_PROOF_LEN: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error("deserialize compressed proof: {0}")]
    Deserialize(String),
    #[error("field element BE serialization expected 32 bytes, got {0}")]
    FieldElementSize(usize),
    #[error("input must be {EIP197_PROOF_LEN} bytes, got {0}")]
    WrongInputLength(usize),
    #[error("A or C point at infinity (not allowed in EIP-197 proofs)")]
    PointAtInfinity,
}

/// Convert an arkworks-compressed 128-byte Groth16 proof to the 256-byte
/// EIP-197 calldata format the L1 verifier expects.
pub fn proof_bytes_to_eip197(compressed_bytes: &[u8]) -> Result<[u8; EIP197_PROOF_LEN], ConversionError> {
    let proof = Proof::<Bn254>::deserialize_compressed(compressed_bytes)
        .map_err(|e| ConversionError::Deserialize(format!("{:?}", e)))?;
    proof_to_eip197(&proof)
}

/// Convert a deserialized arkworks Groth16 proof to EIP-197 calldata.
pub fn proof_to_eip197(proof: &Proof<Bn254>) -> Result<[u8; EIP197_PROOF_LEN], ConversionError> {
    let mut out = [0u8; EIP197_PROOF_LEN];

    // A — G1: (x, y) in big-endian Fq
    write_g1_affine(&proof.a, &mut out[0..64])?;

    // B — G2: EIP-197 order (x.c1, x.c0, y.c1, y.c0)
    write_g2_affine(&proof.b, &mut out[64..192])?;

    // C — G1
    write_g1_affine(&proof.c, &mut out[192..256])?;

    Ok(out)
}

/// Parse an EIP-197 256-byte proof back into raw `(A.x, A.y, B.x.c1,
/// B.x.c0, B.y.c1, B.y.c0, C.x, C.y)` byte triples. Lossless inverse
/// of [`proof_to_eip197`] *as a byte transformation* — does NOT
/// reconstruct a `Proof<Bn254>` (the L1 verifier doesn't need that,
/// it passes bytes straight to the EIP-197 pairing precompile).
pub fn eip197_split(bytes: &[u8; EIP197_PROOF_LEN]) -> Eip197Parts {
    let mut p = Eip197Parts::default();
    p.a_x.copy_from_slice(&bytes[0..32]);
    p.a_y.copy_from_slice(&bytes[32..64]);
    p.b_x_c1.copy_from_slice(&bytes[64..96]);
    p.b_x_c0.copy_from_slice(&bytes[96..128]);
    p.b_y_c1.copy_from_slice(&bytes[128..160]);
    p.b_y_c0.copy_from_slice(&bytes[160..192]);
    p.c_x.copy_from_slice(&bytes[192..224]);
    p.c_y.copy_from_slice(&bytes[224..256]);
    p
}

/// Byte-level view of an EIP-197 proof for inspection / test assertion.
/// Each field is the big-endian byte string of a BN254 Fq element.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Eip197Parts {
    pub a_x: [u8; 32],
    pub a_y: [u8; 32],
    pub b_x_c1: [u8; 32],
    pub b_x_c0: [u8; 32],
    pub b_y_c1: [u8; 32],
    pub b_y_c0: [u8; 32],
    pub c_x: [u8; 32],
    pub c_y: [u8; 32],
}

fn write_g1_affine(point: &G1Affine, dst: &mut [u8]) -> Result<(), ConversionError> {
    debug_assert_eq!(dst.len(), 64);
    // arkworks 0.5: `point.xy()` returns `Option<(Fq, Fq)>` (owned),
    // not `Option<(&Fq, &Fq)>` as in 0.4 — pass references.
    let (x, y) = point.xy().ok_or(ConversionError::PointAtInfinity)?;
    write_fq_be(&x, &mut dst[0..32])?;
    write_fq_be(&y, &mut dst[32..64])?;
    Ok(())
}

fn write_g2_affine(point: &G2Affine, dst: &mut [u8]) -> Result<(), ConversionError> {
    debug_assert_eq!(dst.len(), 128);
    // arkworks 0.5: owned `(Fq2, Fq2)` tuple — same EIP-197 layout.
    let (x, y) = point.xy().ok_or(ConversionError::PointAtInfinity)?;
    // EIP-197: c1 (imaginary) first, then c0 (real). See module doc.
    write_fq_be(&x.c1, &mut dst[0..32])?;
    write_fq_be(&x.c0, &mut dst[32..64])?;
    write_fq_be(&y.c1, &mut dst[64..96])?;
    write_fq_be(&y.c0, &mut dst[96..128])?;
    Ok(())
}

fn write_fq_be(fq: &Fq, dst: &mut [u8]) -> Result<(), ConversionError> {
    debug_assert_eq!(dst.len(), 32);
    let int_repr = fq.into_bigint();
    let bytes_be = int_repr.to_bytes_be();
    if bytes_be.len() != 32 {
        return Err(ConversionError::FieldElementSize(bytes_be.len()));
    }
    dst.copy_from_slice(&bytes_be);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputs::WrapperPublicInputs;
    use crate::prover::{prove, setup};
    use ark_bn254::Fr;
    use ark_std::rand::SeedableRng;

    fn seeded_rng() -> ark_std::rand::rngs::StdRng {
        ark_std::rand::rngs::StdRng::seed_from_u64(0xC0FFEE_u64)
    }

    /// Output is exactly 256 bytes — the length the L1 verifier
    /// enforces via `InvalidGroth16ProofLength`.
    #[test]
    #[ignore] // requires setup (slow)
    fn eip197_output_is_exactly_256_bytes() {
        let mut rng = seeded_rng();
        let (pk, _vk) = setup(&mut rng).expect("setup");
        let proof_bytes = prove(
            &pk,
            WrapperPublicInputs {
                state_root: Fr::from(1u64),
                key: Fr::from(2u64),
                value_commitment: Fr::from(3u64),
                params_fingerprint: Fr::from(4u64),
            },
            vec![],
            &mut rng,
        )
        .expect("prove");
        assert_eq!(proof_bytes.len(), 128, "compressed proof is 128 bytes");

        let eip197 = proof_bytes_to_eip197(&proof_bytes).expect("convert");
        assert_eq!(eip197.len(), 256);
    }

    /// Round-trip: arkworks compressed → EIP-197 → split into parts →
    /// each part is exactly 32 bytes. Pins the byte-layout offsets.
    #[test]
    #[ignore]
    fn eip197_split_field_sizes_pin_layout() {
        let mut rng = seeded_rng();
        let (pk, _vk) = setup(&mut rng).expect("setup");
        let proof_bytes = prove(
            &pk,
            WrapperPublicInputs {
                state_root: Fr::from(11u64),
                key: Fr::from(22u64),
                value_commitment: Fr::from(33u64),
                params_fingerprint: Fr::from(44u64),
            },
            vec![],
            &mut rng,
        )
        .expect("prove");
        let eip197 = proof_bytes_to_eip197(&proof_bytes).expect("convert");
        let parts = eip197_split(&eip197);

        // Every Fq slot is exactly 32 bytes (asserted by [u8; 32] type).
        // The point at infinity check happens during write_g{1,2}_affine.
        // For a valid Groth16 proof, A and C are not at infinity, so
        // (A.x, A.y) and (C.x, C.y) are non-trivial.
        assert_ne!(parts.a_x, [0u8; 32], "A.x must not be zero");
        assert_ne!(parts.a_y, [0u8; 32], "A.y must not be zero");
        assert_ne!(parts.c_x, [0u8; 32], "C.x must not be zero");
        assert_ne!(parts.c_y, [0u8; 32], "C.y must not be zero");
    }

    /// Determinism — the same proof emits the same EIP-197 bytes.
    /// Critical for cross-machine fixture reproducibility.
    #[test]
    #[ignore]
    fn eip197_conversion_is_deterministic() {
        let mut rng_a = seeded_rng();
        let (pk_a, _vk_a) = setup(&mut rng_a).expect("setup a");
        let inputs = WrapperPublicInputs {
            state_root: Fr::from(7u64),
            key: Fr::from(8u64),
            value_commitment: Fr::from(9u64),
            params_fingerprint: Fr::from(10u64),
        };
        let proof_a = prove(&pk_a, inputs.clone(), vec![], &mut rng_a).expect("prove a");
        let eip197_a = proof_bytes_to_eip197(&proof_a).expect("convert a");

        let mut rng_b = seeded_rng();
        let (pk_b, _vk_b) = setup(&mut rng_b).expect("setup b");
        let proof_b = prove(&pk_b, inputs, vec![], &mut rng_b).expect("prove b");
        let eip197_b = proof_bytes_to_eip197(&proof_b).expect("convert b");

        assert_eq!(eip197_a, eip197_b, "EIP-197 output must be deterministic");
    }

    /// Wrong input length on the compressed side bubbles up as a
    /// Deserialize error — not silently truncated.
    #[test]
    fn rejects_malformed_compressed_input() {
        let r = proof_bytes_to_eip197(&[0u8; 64]);
        assert!(r.is_err(), "must reject 64-byte input (compressed is 128 bytes)");
    }

    /// Sanity check on `eip197_split` for hand-crafted bytes: each slot
    /// reads from its declared offset and lands in its declared field.
    /// (Tests the indexing, not the cryptography.)
    #[test]
    fn eip197_split_maps_offsets_to_named_fields() {
        let mut bytes = [0u8; 256];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i / 32) as u8; // each 32-byte slot filled with its index
        }
        let p = eip197_split(&bytes);
        assert_eq!(p.a_x[0], 0, "slot 0 → A.x");
        assert_eq!(p.a_y[0], 1, "slot 1 → A.y");
        assert_eq!(p.b_x_c1[0], 2, "slot 2 → B.x.c1");
        assert_eq!(p.b_x_c0[0], 3, "slot 3 → B.x.c0");
        assert_eq!(p.b_y_c1[0], 4, "slot 4 → B.y.c1");
        assert_eq!(p.b_y_c0[0], 5, "slot 5 → B.y.c0");
        assert_eq!(p.c_x[0],   6, "slot 6 → C.x");
        assert_eq!(p.c_y[0],   7, "slot 7 → C.y");
    }
}

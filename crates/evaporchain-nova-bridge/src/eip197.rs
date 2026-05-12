//! Phase 2.5 prep — EIP-197 byte-layout conversion for the Groth16
//! proof produced by [`crate::groth16_wrapper`].
//!
//! # What's the byte layout
//!
//! Ethereum's EIP-197 pairing precompile (`address(0x08)`) reads
//! group elements in a specific uncompressed encoding:
//!
//! - **G1 point** (64 bytes): `x || y`, each coordinate as a 32-byte
//!   big-endian `uint256`.
//!
//! - **G2 point** (128 bytes): `x.c1 || x.c0 || y.c1 || y.c0`. Each
//!   field-of-extension component as a 32-byte big-endian
//!   `uint256`. **The imaginary component (`c1`) comes first** —
//!   this is the silent-failure trap. arkworks orders `Fq2` as
//!   `(c0, c1)` internally; EIP-197 expects the reverse on the
//!   wire.
//!
//! - **Groth16 proof** (256 bytes): `A || B || C` where `A: G1`,
//!   `B: G2`, `C: G1`.
//!
//! # Why a module
//!
//! Three responsibilities:
//!
//! 1. Encode an `ark_bn254::G1Affine` to its 64-byte EIP-197 form.
//! 2. Encode an `ark_bn254::G2Affine` to its 128-byte EIP-197 form,
//!    swapping the `c0`/`c1` order on each coordinate.
//! 3. Walk an `ark_groth16::Proof<Bn254>` and concatenate
//!    `A || B || C` into the 256-byte block that the Solidity
//!    verifier passes to the precompile.
//!
//! # What's NOT here
//!
//! - The verifying-key encoding for Solidity. The `vk` has six
//!   group elements (`α: G1, β: G2, γ: G2, δ: G2, γ_abc[]: G1`)
//!   plus the per-public-input commitment vector — these are
//!   typically baked into the Solidity contract as constants at
//!   deploy time, not passed dynamically. A future PR will emit
//!   the Solidity-snippet vk constants.
//!
//! - Identity / point-at-infinity handling. ark-bn254 encodes
//!   identity as `(0, 0)` with the infinity flag set; EIP-197
//!   reads `(0, 0)` as identity unconditionally. The conversion
//!   here writes `(0, 0)` for identity, which is the EIP-197
//!   convention — but a real Groth16 proof never contains an
//!   identity, so this path is for completeness only.

use ark_bn254::{Bn254, Fq, Fq2, G1Affine, G2Affine};
use ark_ff::{BigInteger, PrimeField};
use ark_groth16::Proof;

/// Encode a `Fq` element as 32 big-endian bytes (EIP-197 uint256
/// layout).
fn fq_to_be32(x: &Fq) -> [u8; 32] {
    let bigint = x.into_bigint();
    let mut buf = [0u8; 32];
    let bytes_be = bigint.to_bytes_be();
    // BN254 Fq fits in 254 bits; `to_bytes_be` returns ≤ 32 bytes.
    // Right-align into a 32-byte buffer.
    let pad = 32 - bytes_be.len();
    buf[pad..].copy_from_slice(&bytes_be);
    buf
}

/// Encode an `ark_bn254::G1Affine` as the 64-byte EIP-197 layout
/// `x || y`. Identity encodes as `(0, 0)`.
pub fn g1_to_eip197(g: &G1Affine) -> [u8; 64] {
    let mut out = [0u8; 64];
    if g.infinity {
        return out;
    }
    out[..32].copy_from_slice(&fq_to_be32(&g.x));
    out[32..].copy_from_slice(&fq_to_be32(&g.y));
    out
}

/// Encode an `ark_bn254::G2Affine` as the 128-byte EIP-197 layout
/// `x.c1 || x.c0 || y.c1 || y.c0`. Identity encodes as four zero
/// 32-byte words.
///
/// **Cross-ordering note.** ark-bn254 stores `Fq2` as `(c0, c1)`
/// (real then imaginary). EIP-197's expected wire order is
/// `(c1, c0)` (imaginary then real). This function applies the
/// swap. Skipping the swap silently passes wrong group elements
/// to the pairing precompile and every proof fails to verify.
pub fn g2_to_eip197(g: &G2Affine) -> [u8; 128] {
    let mut out = [0u8; 128];
    if g.infinity {
        return out;
    }
    let x: &Fq2 = &g.x;
    let y: &Fq2 = &g.y;
    out[..32].copy_from_slice(&fq_to_be32(&x.c1));
    out[32..64].copy_from_slice(&fq_to_be32(&x.c0));
    out[64..96].copy_from_slice(&fq_to_be32(&y.c1));
    out[96..].copy_from_slice(&fq_to_be32(&y.c0));
    out
}

/// Concatenate a Groth16 proof's `(A, B, C)` into the 256-byte
/// EIP-197 block: `A (64) || B (128) || C (64)`.
pub fn proof_to_eip197(proof: &Proof<Bn254>) -> [u8; 256] {
    let mut out = [0u8; 256];
    out[..64].copy_from_slice(&g1_to_eip197(&proof.a));
    out[64..192].copy_from_slice(&g2_to_eip197(&proof.b));
    out[192..].copy_from_slice(&g1_to_eip197(&proof.c));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::groth16_wrapper::{prove, public_inputs_in_alloc_order, setup};
    use crate::verifier_circuit::NovaVerifierCircuit;
    use ark_bn254::Fr;
    use ark_ec::AffineRepr;
    use ark_std::rand::rngs::StdRng;
    use ark_std::rand::SeedableRng;

    #[test]
    fn g1_identity_encodes_as_zeros() {
        let identity = G1Affine::identity();
        let bytes = g1_to_eip197(&identity);
        assert_eq!(bytes, [0u8; 64]);
    }

    #[test]
    fn g2_identity_encodes_as_zeros() {
        let identity = G2Affine::identity();
        let bytes = g2_to_eip197(&identity);
        assert_eq!(bytes, [0u8; 128]);
    }

    /// G1 generator must round-trip through encoding to a non-zero
    /// 64-byte block with x and y in the expected halves. Catches
    /// any accidental coordinate swap or length error.
    #[test]
    fn g1_generator_encodes_to_known_shape() {
        let gen = G1Affine::generator();
        let bytes = g1_to_eip197(&gen);
        assert_ne!(bytes, [0u8; 64], "generator must not encode as identity");
        // BN254 G1 generator: x=1, y=2. In 32-byte BE both fit at
        // the tail of the buffer.
        let mut expected_x = [0u8; 32];
        expected_x[31] = 1;
        let mut expected_y = [0u8; 32];
        expected_y[31] = 2;
        assert_eq!(&bytes[..32], &expected_x);
        assert_eq!(&bytes[32..], &expected_y);
    }

    /// Confirm the c0 / c1 swap on G2. BN254 G2 generator has
    /// well-known coordinates; flipping the swap would produce a
    /// different 32-byte half. This is the most common bridge bug.
    #[test]
    fn g2_generator_encodes_with_c1_first() {
        let gen = G2Affine::generator();
        let bytes = g2_to_eip197(&gen);
        assert_ne!(bytes[..128], [0u8; 128][..]);
        // Crucially, the bytes representing the IMAGINARY part of
        // x (`x.c1`) live at offset 0..32, not 32..64. If we
        // accidentally encoded `(c0, c1)`, the c0 bytes would land
        // there instead. Cross-check by also encoding directly and
        // comparing.
        assert_eq!(&bytes[..32], &fq_to_be32(&gen.x.c1));
        assert_eq!(&bytes[32..64], &fq_to_be32(&gen.x.c0));
        assert_eq!(&bytes[64..96], &fq_to_be32(&gen.y.c1));
        assert_eq!(&bytes[96..128], &fq_to_be32(&gen.y.c0));
    }

    /// End-to-end: generate a real Groth16 proof via the wrapper,
    /// convert to EIP-197 bytes, sanity-check the structure
    /// (256 bytes, non-zero, internal segments non-zero). Any
    /// Groth16 proof's A, B, C are guaranteed non-identity in
    /// practice — if a 64/128/64-byte segment came back all zero,
    /// the conversion is broken.
    #[test]
    fn real_proof_encodes_to_256_bytes() {
        let mut rng = StdRng::seed_from_u64(0xe197_b17e_0u64);
        let keys = setup(&mut rng).expect("setup");
        let circuit = NovaVerifierCircuit::new(
            5,
            vec![Fr::from(7u64)],
            vec![Fr::from(11u64)],
            Fr::from(0x1234u64),
            Fr::from(0x5678u64),
        );
        let _public_inputs = public_inputs_in_alloc_order(&circuit);
        let proof = prove(&keys.pk, circuit, &mut rng).expect("prove");

        let bytes = proof_to_eip197(&proof);
        assert_eq!(bytes.len(), 256);

        // Each segment must be non-identity for a real proof.
        let a_zeros = &bytes[..64] == &[0u8; 64][..];
        let b_zeros = &bytes[64..192] == &[0u8; 128][..];
        let c_zeros = &bytes[192..] == &[0u8; 64][..];
        assert!(
            !a_zeros && !b_zeros && !c_zeros,
            "proof segments must be non-identity: A_zeros={a_zeros} B_zeros={b_zeros} C_zeros={c_zeros}"
        );
    }

    /// Pin the precise byte counts so a future arkworks API change
    /// can't silently expand a coordinate.
    #[test]
    fn segment_sizes_are_exact() {
        let g1 = G1Affine::generator();
        let g2 = G2Affine::generator();
        assert_eq!(g1_to_eip197(&g1).len(), 64);
        assert_eq!(g2_to_eip197(&g2).len(), 128);
    }
}

//! Phase 2.4 follow-up — canonical arkworks byte serialization
//! for the Groth16 verifying key, proving key, and proof.
//!
//! # Why a separate module from `eip197`
//!
//! Two different byte layouts exist for the same artifacts:
//!
//! - **Canonical arkworks** (this module) — what
//!   `CanonicalSerialize` / `CanonicalDeserialize` use. Round-trips
//!   losslessly through Rust callers, supports compressed and
//!   uncompressed encodings, has a single concatenated layout for
//!   complex structs (proving keys with thousands of group
//!   elements, etc.). This is the on-disk + in-memory persistence
//!   format.
//!
//! - **EIP-197 uncompressed BE-uint256** (`crate::eip197`) — what
//!   Ethereum's pairing precompile reads. Only handles single G1
//!   and G2 points, in a specific cross-ordering. This is the
//!   on-chain wire format for proofs.
//!
//! Both are needed:
//! - Operator runs `setup`, persists `pk` + `vk` via this module's
//!   canonical bytes.
//! - Operator runs `prove`, persists the `Proof<Bn254>` via this
//!   module's canonical bytes for archival / pipeline plumbing.
//! - Bridge sends the same proof to L1 via [`crate::eip197::proof_to_eip197`].
//!
//! # Compressed vs uncompressed
//!
//! Arkworks compressed group-element encoding saves ~50% but
//! requires a square-root computation at deserialization. The
//! gas-sensitive on-chain path uses EIP-197 uncompressed; the
//! off-chain persistence path uses arkworks **compressed** here
//! because storage is the binding constraint. For a `pk` with
//! thousands of group elements that's a meaningful saving.
//!
//! Add a `_uncompressed` variant if a future caller needs the
//! larger encoding off-chain too.

use ark_bn254::Bn254;
use ark_groth16::{Proof, ProvingKey, VerifyingKey};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize, SerializationError};

/// Serialize a `VerifyingKey<Bn254>` to canonical compressed bytes.
pub fn vk_to_bytes(vk: &VerifyingKey<Bn254>) -> Result<Vec<u8>, SerializationError> {
    let mut buf = Vec::new();
    vk.serialize_compressed(&mut buf)?;
    Ok(buf)
}

/// Deserialize a `VerifyingKey<Bn254>` from canonical compressed bytes.
pub fn vk_from_bytes(bytes: &[u8]) -> Result<VerifyingKey<Bn254>, SerializationError> {
    VerifyingKey::<Bn254>::deserialize_compressed(bytes)
}

/// Serialize a `ProvingKey<Bn254>` to canonical compressed bytes.
/// Used by operators persisting setup output between sessions.
/// Note: `pk` is multi-megabyte even compressed — caller should
/// stream this to disk, not hold in-memory beyond the round trip.
pub fn pk_to_bytes(pk: &ProvingKey<Bn254>) -> Result<Vec<u8>, SerializationError> {
    let mut buf = Vec::new();
    pk.serialize_compressed(&mut buf)?;
    Ok(buf)
}

/// Deserialize a `ProvingKey<Bn254>` from canonical compressed bytes.
pub fn pk_from_bytes(bytes: &[u8]) -> Result<ProvingKey<Bn254>, SerializationError> {
    ProvingKey::<Bn254>::deserialize_compressed(bytes)
}

/// Serialize a `Proof<Bn254>` to canonical compressed bytes.
/// Use this for archival / Rust-to-Rust transport; use
/// [`crate::eip197::proof_to_eip197`] for the on-chain
/// uncompressed wire format.
pub fn proof_to_bytes(proof: &Proof<Bn254>) -> Result<Vec<u8>, SerializationError> {
    let mut buf = Vec::new();
    proof.serialize_compressed(&mut buf)?;
    Ok(buf)
}

/// Deserialize a `Proof<Bn254>` from canonical compressed bytes.
pub fn proof_from_bytes(bytes: &[u8]) -> Result<Proof<Bn254>, SerializationError> {
    Proof::<Bn254>::deserialize_compressed(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::groth16_wrapper::{prove, public_inputs_in_alloc_order, setup, verify};
    use crate::verifier_circuit::NovaVerifierCircuit;
    use ark_bn254::Fr;
    use ark_std::rand::rngs::StdRng;
    use ark_std::rand::SeedableRng;

    /// Round-trip the VK through canonical bytes and confirm the
    /// deserialized VK still verifies the same proof — proves the
    /// byte representation losslessly preserves all 5 group
    /// elements (α, β, γ, δ, γ_abc[]).
    #[test]
    fn vk_round_trips_through_canonical_bytes() {
        let mut rng = StdRng::seed_from_u64(0xca_70_5e_71_0u64);
        let keys = setup(&mut rng).expect("setup");

        let bytes = vk_to_bytes(&keys.vk).expect("vk serialize");
        assert!(!bytes.is_empty(), "vk bytes must be non-empty");
        let vk_back = vk_from_bytes(&bytes).expect("vk deserialize");

        // Most direct correctness check: a proof produced with the
        // original `vk`'s associated `pk` must verify under the
        // round-tripped `vk_back`. Anything less catches partial
        // serialization but not field-by-field correctness.
        let circuit = NovaVerifierCircuit::new(
            5,
            vec![Fr::from(7u64)],
            vec![Fr::from(11u64)],
            Fr::from(0x1234u64),
            Fr::from(0x5678u64),
        );
        let public_inputs = public_inputs_in_alloc_order(&circuit);
        let proof = prove(&keys.pk, circuit, &mut rng).expect("prove");
        let ok = verify(&vk_back, &public_inputs, &proof).expect("verify");
        assert!(ok, "round-tripped vk must verify a valid proof");
    }

    /// Round-trip the PK through canonical bytes and confirm a
    /// proof produced with the deserialized PK verifies under the
    /// original VK. Catches drift in either direction of the PK's
    /// CRS encoding.
    #[test]
    fn pk_round_trips_through_canonical_bytes() {
        let mut rng = StdRng::seed_from_u64(0xca_70_5e_71_1u64);
        let keys = setup(&mut rng).expect("setup");

        let bytes = pk_to_bytes(&keys.pk).expect("pk serialize");
        assert!(bytes.len() > 1024, "pk bytes should be at least 1KB even at trivial circuit size");
        let pk_back = pk_from_bytes(&bytes).expect("pk deserialize");

        let circuit = NovaVerifierCircuit::new(
            3,
            vec![Fr::from(42u64)],
            vec![Fr::from(99u64)],
            Fr::from(0xaabb_ccdd_u64),
            Fr::from(0xeeff_0011_u64),
        );
        let public_inputs = public_inputs_in_alloc_order(&circuit);
        let proof = prove(&pk_back, circuit, &mut rng).expect("prove with round-tripped pk");
        let ok = verify(&keys.vk, &public_inputs, &proof).expect("verify");
        assert!(ok, "proof from round-tripped pk must verify");
    }

    /// Round-trip a proof through canonical bytes and confirm the
    /// deserialized proof still verifies under the same vk +
    /// public inputs.
    #[test]
    fn proof_round_trips_through_canonical_bytes() {
        let mut rng = StdRng::seed_from_u64(0xca_70_5e_71_2u64);
        let keys = setup(&mut rng).expect("setup");
        let circuit = NovaVerifierCircuit::new(
            7,
            vec![Fr::from(2u64)],
            vec![Fr::from(3u64)],
            Fr::from(0xa_a_a_a_u64),
            Fr::from(0xb_b_b_b_u64),
        );
        let public_inputs = public_inputs_in_alloc_order(&circuit);
        let proof = prove(&keys.pk, circuit, &mut rng).expect("prove");

        let bytes = proof_to_bytes(&proof).expect("proof serialize");
        // Groth16-on-BN254 compressed proof: A (32) + B (64) + C (32) = 128 bytes.
        assert!(
            (96..=160).contains(&bytes.len()),
            "compressed proof size out of expected range: {}",
            bytes.len()
        );

        let proof_back = proof_from_bytes(&bytes).expect("proof deserialize");
        let ok = verify(&keys.vk, &public_inputs, &proof_back).expect("verify");
        assert!(ok, "round-tripped proof must verify");
    }

    /// Confirm proof deserialization rejects truncated input —
    /// catches accidental partial reads from disk / wire.
    #[test]
    fn proof_deserialize_rejects_truncated() {
        let mut rng = StdRng::seed_from_u64(0xca_70_5e_71_3u64);
        let keys = setup(&mut rng).expect("setup");
        let circuit = NovaVerifierCircuit::new(
            1,
            vec![Fr::from(0u64)],
            vec![Fr::from(0u64)],
            Fr::from(0u64),
            Fr::from(0u64),
        );
        let proof = prove(&keys.pk, circuit, &mut rng).expect("prove");

        let bytes = proof_to_bytes(&proof).expect("serialize");
        // Truncate the last 8 bytes — must fail to deserialize.
        let truncated = &bytes[..bytes.len().saturating_sub(8)];
        let result = proof_from_bytes(truncated);
        assert!(
            result.is_err(),
            "truncated proof bytes must fail to deserialize"
        );
    }

    /// Confirm vk byte length stability: with the dummy circuit's
    /// fixed `|gamma_abc_g1| = 5` (one per public input + 1), the
    /// vk byte size is deterministic. If a future arkworks change
    /// alters the encoding, this test fires.
    #[test]
    fn vk_byte_length_is_deterministic_for_skeleton() {
        let mut rng1 = StdRng::seed_from_u64(0x11_11u64);
        let mut rng2 = StdRng::seed_from_u64(0x22_22u64);
        let keys1 = setup(&mut rng1).expect("setup");
        let keys2 = setup(&mut rng2).expect("setup");
        let bytes1 = vk_to_bytes(&keys1.vk).expect("serialize 1");
        let bytes2 = vk_to_bytes(&keys2.vk).expect("serialize 2");
        // Different RNGs produce different vk *contents*, but the
        // STRUCTURE is identical so the byte length matches.
        assert_eq!(
            bytes1.len(),
            bytes2.len(),
            "vk byte length must be deterministic for fixed circuit shape"
        );
        assert_ne!(
            bytes1, bytes2,
            "different RNG seeds must produce different vk content"
        );
    }
}

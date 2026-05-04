pub mod accumulator;
pub mod bls_key_store;
pub mod energy_verkle;
pub mod hash;
pub mod secret_file_store;
pub mod signatures;
pub mod verkle;
pub mod vrf;

pub use accumulator::{
    Accumulator, EnergyStampedNullifier, InMemoryAccumulator, MMRPosition, MMRProof,
    MembershipProof, MerkleMountainRange, NonMembershipProof,
};
pub use energy_verkle::{
    EnergyMeta, EnergyVerkleMultiProof, EnergyVerkleProof, EnergyVerkleTrie, TrieHealth,
};
pub use hash::{blake3_hash, poseidon_hash, Blake3Hasher, HashEngine, PoseidonHasher};
pub use signatures::{
    BlsError, BlsKeypair, BlsPublicKey, BlsSecretKey, BlsSignature, BlsVerifier, EcdsaError,
    EcdsaKeypair, EcdsaVerifier, HybridKeypair, HybridVerifier, MlDsaKeypair, MlDsaVerifier,
    Signer, Verifier, HYBRID_PK_LEN, HYBRID_SIG_LEN,
};
pub use verkle::{VerkleProof, VerkleTrie};
pub use vrf::{
    leader_vrf_input, sortition, sortition_vrf_input, vrf_leader_check, vrf_verify,
    RandomnessBeacon, VrfKeypair, VrfOutput, VrfProof,
};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "evaporchain-crypto's hybrid signer is
    /// **non-short-circuit**: a valid hybrid signature requires BOTH
    /// the ECDSA and ML-DSA components to verify. Forging only one
    /// component is rejected. The hybrid encoding is length-tagged so
    /// classical signatures cannot impersonate post-quantum ones."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        use signatures::Signer;
        let kp = HybridKeypair::generate();
        let pk = kp.public_key_bytes();
        let msg = b"audit-fix: hybrid sig must be non-short-circuit";

        // Honest sign+verify round-trip.
        let sig = kp.sign(msg);
        assert!(HybridVerifier::verify_hybrid(msg, &sig, &pk));

        // Tampered ECDSA half (offset 1, the first ECDSA byte) → reject.
        let mut bad_ecdsa = sig.clone();
        bad_ecdsa[1] ^= 0xFF;
        assert!(!HybridVerifier::verify_hybrid(msg, &bad_ecdsa, &pk));

        // Tampered ML-DSA half (offset 1+ECDSA_SIG_LEN) → reject.
        let mut bad_mldsa = sig.clone();
        let last = bad_mldsa.len() - 1;
        bad_mldsa[last] ^= 0xFF;
        assert!(!HybridVerifier::verify_hybrid(msg, &bad_mldsa, &pk));

        // Wrong tag (drop hybrid byte): not a hybrid sig.
        let truncated = &sig[1..];
        assert!(!HybridVerifier::is_hybrid_sig(truncated));
        assert!(!HybridVerifier::verify_hybrid(msg, truncated, &pk));

        // Wrong message → reject.
        assert!(!HybridVerifier::verify_hybrid(b"different", &sig, &pk));

        // BLAKE3 is deterministic across two calls (sanity).
        assert_eq!(blake3_hash(msg), blake3_hash(msg));
    }
}

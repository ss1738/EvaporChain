//! Phase 2.4 starter — Groth16 trusted setup over
//! [`crate::verifier_circuit::NovaVerifierCircuit`].
//!
//! # What ships at this milestone
//!
//! [`setup`] — wraps `Groth16::<Bn254>::circuit_specific_setup` on
//! `NovaVerifierCircuit::dummy()`, producing a
//! `(ProvingKey, VerifyingKey)` pair sized to the circuit shape
//! `dummy()` pins (5 public inputs: 2 hashes + 1 z0 + 1 zi + 1
//! Groth16 const).
//!
//! `prove` + `verify` wrappers land in a follow-up PR once the
//! `l_u_secondary` access gap (see [`crate::circuit_builder`])
//! is resolved.
//!
//! # Why setup is safe to ship without the access gap closed
//!
//! Setup operates on `dummy()` — a witness-independent shape
//! template. The dummy's `committed_hash_primary` /
//! `committed_hash_secondary` are zero placeholders, but setup
//! doesn't *prove* anything about them; it only reads the circuit
//! shape (number of public inputs, total constraints) to size the
//! prover/verifier keys. Once the access gap closes for real
//! `prove`-time witnesses, the keys produced here remain valid
//! because the shape is identical between `dummy()` and any real
//! witness (same arity for hashes, z0, zi).
//!
//! # Trusted-setup caveat (production deployment)
//!
//! `circuit_specific_setup` uses arkworks's *insecure* test
//! randomness. For mainnet, the keys must come from a multi-party
//! ceremony (e.g. Powers of Tau + circuit-specific phase 2). This
//! module's `setup` is the right entry point for testing and for
//! development, but is **not** production-safe. Production swap-in
//! point is the same function signature with the ceremony-derived
//! parameters substituted.

use ark_bn254::Bn254;
use ark_groth16::{Groth16, ProvingKey, VerifyingKey};
use ark_snark::SNARK;
use ark_std::rand::{CryptoRng, RngCore};

use crate::verifier_circuit::NovaVerifierCircuit;

/// Run Groth16's circuit-specific trusted setup against
/// `NovaVerifierCircuit::dummy()`. Returns `(pk, vk)`.
///
/// **Test/dev only.** See module docstring for the production
/// trusted-setup story.
pub fn setup<R: RngCore + CryptoRng>(
    rng: &mut R,
) -> Result<(ProvingKey<Bn254>, VerifyingKey<Bn254>), ark_relations::r1cs::SynthesisError> {
    let dummy = NovaVerifierCircuit::dummy();
    Groth16::<Bn254>::circuit_specific_setup(dummy, rng)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
    use ark_std::rand::SeedableRng;

    /// Setup produces a non-trivial (pk, vk) pair on the dummy
    /// circuit. Pins the public-input-count contract that Section
    /// 1 already enforces (5 entries) — Groth16's verification
    /// key's `gamma_abc_g1` array has length `num_instance_variables`,
    /// so we get a structural cross-check here too.
    #[test]
    fn setup_produces_keys_with_expected_public_input_count() {
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(0);
        let (pk, vk) = setup(&mut rng).expect("setup must succeed on dummy");

        // 5 public inputs as pinned by
        // `verifier_circuit::tests::skeleton_dummy_synthesizes_with_expected_public_input_arity`.
        // Groth16's verifying key adds one extra entry (the
        // constant 1 input), so `gamma_abc_g1` has length 5.
        assert_eq!(
            vk.gamma_abc_g1.len(),
            5,
            "vk.gamma_abc_g1 must hold 5 entries (2 hashes + |z0| + |zi| + 1 const)"
        );

        // Prover key cross-link: `pk.vk` must equal the returned vk.
        let mut vk_bytes = Vec::new();
        let mut pk_vk_bytes = Vec::new();
        vk.serialize_compressed(&mut vk_bytes).expect("vk serialize");
        pk.vk
            .serialize_compressed(&mut pk_vk_bytes)
            .expect("pk.vk serialize");
        assert_eq!(
            vk_bytes, pk_vk_bytes,
            "pk.vk must be byte-identical to the standalone vk"
        );
    }

    /// Both keys round-trip through arkworks compressed
    /// serialization. Catches accidental dependency-bump-induced
    /// schema drift if a future arkworks version changes the
    /// canonical form.
    #[test]
    fn setup_keys_round_trip_through_canonical_serialize() {
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(7);
        let (pk, vk) = setup(&mut rng).expect("setup");

        let mut pk_buf = Vec::new();
        pk.serialize_compressed(&mut pk_buf).expect("pk serialize");
        let pk_back: ProvingKey<Bn254> =
            ProvingKey::deserialize_compressed(&pk_buf[..]).expect("pk deserialize");
        let mut pk_back_buf = Vec::new();
        pk_back
            .serialize_compressed(&mut pk_back_buf)
            .expect("pk_back serialize");
        assert_eq!(pk_buf, pk_back_buf, "pk round-trip must be byte-stable");

        let mut vk_buf = Vec::new();
        vk.serialize_compressed(&mut vk_buf).expect("vk serialize");
        let vk_back: VerifyingKey<Bn254> =
            VerifyingKey::deserialize_compressed(&vk_buf[..]).expect("vk deserialize");
        let mut vk_back_buf = Vec::new();
        vk_back
            .serialize_compressed(&mut vk_back_buf)
            .expect("vk_back serialize");
        assert_eq!(vk_buf, vk_back_buf, "vk round-trip must be byte-stable");
    }

    /// Setup is deterministic for a fixed seed. Catches a future
    /// arkworks bug where randomness leaks in from outside the
    /// RNG (e.g. system time, hardware-RNG fallback).
    #[test]
    fn setup_is_deterministic_for_fixed_seed() {
        let mut rng_a = ark_std::rand::rngs::StdRng::seed_from_u64(42);
        let mut rng_b = ark_std::rand::rngs::StdRng::seed_from_u64(42);
        let (pk_a, vk_a) = setup(&mut rng_a).expect("setup a");
        let (pk_b, vk_b) = setup(&mut rng_b).expect("setup b");

        let mut a_bytes = Vec::new();
        let mut b_bytes = Vec::new();
        vk_a.serialize_compressed(&mut a_bytes).unwrap();
        vk_b.serialize_compressed(&mut b_bytes).unwrap();
        assert_eq!(a_bytes, b_bytes, "vk must be deterministic for fixed seed");

        let mut pa = Vec::new();
        let mut pb = Vec::new();
        pk_a.serialize_compressed(&mut pa).unwrap();
        pk_b.serialize_compressed(&mut pb).unwrap();
        assert_eq!(pa, pb, "pk must be deterministic for fixed seed");
    }
}

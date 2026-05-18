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

use ark_bn254::{Bn254, Fr as Bn254Fr};
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_snark::SNARK;
use ark_std::rand::{CryptoRng, RngCore};

use crate::verifier_circuit::NovaVerifierCircuit;

/// Run Groth16's circuit-specific trusted setup against
/// `NovaVerifierCircuit::dummy()`. Returns `(pk, vk)`.
///
/// **Test/dev only.** See module docstring for the production
/// trusted-setup story.
///
/// # SECURITY (audit B-2, 2026-05-18)
///
/// `circuit_specific_setup` uses arkworks's *insecure* test
/// randomness (the "toxic waste" is recoverable), and the circuit it
/// sizes (`dummy()`) currently emits **no soundness-binding
/// constraints** (audit B-1: Sections 2/3 are `Option` and absent in
/// `dummy()`). Keys from this function are therefore forgeable and
/// MUST NOT reach mainnet. Production requires (a) a fixed-shape,
/// section-bearing setup circuit and (b) an MPC ceremony — tracked as
/// the audit's #1 mainnet-blocker. This `#[deprecated]` marker makes
/// every call site emit a build warning so the insecure path cannot
/// be shipped silently; it is intentionally not removed until the
/// ceremony-derived, sound-circuit replacement lands.
#[deprecated(
    note = "INSECURE test/dev trusted setup (audit B-1/B-2): recoverable \
            toxic waste + a constraint-vacuous circuit — forgeable keys. \
            MUST NOT reach mainnet. Replace with the fixed-shape \
            section-bearing circuit + MPC ceremony (audit #1 \
            mainnet-blocker) before any production use."
)]
pub fn setup<R: RngCore + CryptoRng>(
    rng: &mut R,
) -> Result<(ProvingKey<Bn254>, VerifyingKey<Bn254>), ark_relations::r1cs::SynthesisError> {
    // Audit B-1/B-2 S2a: key the circuit over the section-bearing
    // setup_shape() (sections at exact prover R1CS shape) — NOT the
    // constraint-vacuous dummy() — so the soundness bindings are part
    // of the keyed circuit. (#[deprecated] insecure-randomness still
    // applies; the MPC ceremony is the separate S5 gap.)
    let circuit = NovaVerifierCircuit::setup_shape()
        .map_err(|_| ark_relations::r1cs::SynthesisError::Unsatisfiable)?;
    Groth16::<Bn254>::circuit_specific_setup(circuit, rng)
}

/// Generate a Groth16 proof for a concrete `NovaVerifierCircuit`
/// witness, using the provided proving key.
///
/// The circuit can be any `NovaVerifierCircuit` that synthesises
/// a satisfied CS — `NovaVerifierCircuit::dummy()` works today;
/// `circuit_builder::build_circuit_from_fixture(&rs)` will work
/// once Sections 2/3 wire up.
pub fn prove<R: RngCore + CryptoRng>(
    pk: &ProvingKey<Bn254>,
    circuit: NovaVerifierCircuit,
    rng: &mut R,
) -> Result<Proof<Bn254>, ark_relations::r1cs::SynthesisError> {
    Groth16::<Bn254>::prove(pk, circuit, rng)
}

/// Verify a Groth16 proof against the verifying key + public
/// inputs.
///
/// `public_inputs` must be supplied in the same order as the
/// circuit allocates them via `new_input`:
///
///   `[committed_hash_primary, committed_hash_secondary, z0[..], zi[..]]`
///
/// (The Groth16 const-1 input is added implicitly by arkworks
/// and is not part of this slice.)
pub fn verify(
    vk: &VerifyingKey<Bn254>,
    public_inputs: &[Bn254Fr],
    proof: &Proof<Bn254>,
) -> Result<bool, ark_relations::r1cs::SynthesisError> {
    Groth16::<Bn254>::verify(vk, public_inputs, proof)
}

/// Convenience: derive the public-inputs slice for a given
/// `NovaVerifierCircuit` in the order [`verify`] expects.
///
/// Useful when the same `NovaVerifierCircuit` is fed to [`prove`]
/// — both sides must agree on the slice, and writing the order
/// out twice at call sites invites drift.
pub fn public_inputs_for(circuit: &NovaVerifierCircuit) -> Vec<Bn254Fr> {
    let mut out = Vec::with_capacity(2 + circuit.z0.len() + circuit.zi.len());
    out.push(circuit.committed_hash_primary);
    out.push(circuit.committed_hash_secondary);
    out.extend_from_slice(&circuit.z0);
    out.extend_from_slice(&circuit.zi);
    out
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

    /// End-to-end Groth16 round-trip on the dummy circuit:
    /// setup → prove → verify(true). Pins that all three wrappers
    /// agree on the circuit shape.
    ///
    /// The dummy witness has zero hash placeholders. Sections 2+3
    /// are wired and gated on `section2/3.is_some()`. The dummy
    /// circuit (no section witnesses attached) verifies the
    /// Section 1 structural gate only.
    #[test]
    fn prove_and_verify_dummy_round_trip_accepts() {
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(101);
        let (pk, vk) = setup(&mut rng).expect("setup");

        let dummy = NovaVerifierCircuit::dummy();
        let public_inputs = public_inputs_for(&dummy);
        let proof = prove(&pk, dummy, &mut rng).expect("prove");

        let accepted = verify(&vk, &public_inputs, &proof).expect("verify");
        assert!(accepted, "dummy proof must verify against dummy public inputs");
    }

    /// Tampered public input must be rejected by verify. Catches
    /// a regression where verify accidentally short-circuits to
    /// `Ok(true)` regardless of input.
    #[test]
    fn verify_rejects_tampered_public_input() {
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(202);
        let (pk, vk) = setup(&mut rng).expect("setup");

        let dummy = NovaVerifierCircuit::dummy();
        let mut public_inputs = public_inputs_for(&dummy);
        let proof = prove(&pk, dummy, &mut rng).expect("prove");

        // Sanity: untouched verify accepts.
        assert!(verify(&vk, &public_inputs, &proof).expect("verify clean"));

        // Tamper: bump the first public input (committed_hash_primary
        // was 0 for the dummy; make it 1).
        public_inputs[0] = Bn254Fr::from(1u64);
        let rejected = verify(&vk, &public_inputs, &proof).expect("verify tampered");
        assert!(!rejected, "verify must reject a proof against tampered public inputs");
    }

    /// `public_inputs_for` returns exactly the two committed-hash
    /// entries when z0 and zi are empty.
    #[test]
    fn public_inputs_for_empty_state_yields_only_hashes() {
        let circuit = NovaVerifierCircuit::new(
            1,
            vec![],
            vec![],
            Bn254Fr::from(7u64),
            Bn254Fr::from(11u64),
        );
        let pi = public_inputs_for(&circuit);
        assert_eq!(pi, vec![Bn254Fr::from(7u64), Bn254Fr::from(11u64)]);
    }

    /// Tampering the *second* hash slot also rejects. Existing
    /// tampered-input test only flips slot 0.
    #[test]
    fn verify_rejects_tampered_secondary_hash() {
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(404);
        let (pk, vk) = setup(&mut rng).expect("setup");
        let dummy = NovaVerifierCircuit::dummy();
        let mut public_inputs = public_inputs_for(&dummy);
        let proof = prove(&pk, dummy, &mut rng).expect("prove");
        public_inputs[1] = Bn254Fr::from(99u64);
        assert!(!verify(&vk, &public_inputs, &proof).expect("verify"));
    }

    /// A proof under one setup must not verify against a different
    /// setup's vk.
    #[test]
    fn verify_rejects_proof_against_wrong_vk() {
        let mut rng_a = ark_std::rand::rngs::StdRng::seed_from_u64(11);
        let mut rng_b = ark_std::rand::rngs::StdRng::seed_from_u64(22);
        let (pk_a, _vk_a) = setup(&mut rng_a).expect("setup a");
        let (_pk_b, vk_b) = setup(&mut rng_b).expect("setup b");
        let dummy = NovaVerifierCircuit::dummy();
        let pi = public_inputs_for(&dummy);
        let proof = prove(&pk_a, dummy, &mut rng_a).expect("prove with pk_a");
        assert!(
            !verify(&vk_b, &pi, &proof).expect("verify against wrong vk"),
            "proof under setup A must not verify under setup B's vk"
        );
    }

    /// `public_inputs_for` honours the circuit's allocation order:
    /// [hash_primary, hash_secondary, z0[..], zi[..]]. Pin the
    /// exact layout for a circuit with non-trivial arity so future
    /// refactors don't silently shift the public-input slice.
    #[test]
    fn public_inputs_for_honours_circuit_allocation_order() {
        let circuit = NovaVerifierCircuit::new(
            7,
            vec![Bn254Fr::from(11u64), Bn254Fr::from(13u64)],
            vec![Bn254Fr::from(17u64), Bn254Fr::from(19u64)],
            Bn254Fr::from(2u64),
            Bn254Fr::from(3u64),
        );
        let pi = public_inputs_for(&circuit);
        assert_eq!(
            pi,
            vec![
                Bn254Fr::from(2u64),  // committed_hash_primary
                Bn254Fr::from(3u64),  // committed_hash_secondary
                Bn254Fr::from(11u64), // z0[0]
                Bn254Fr::from(13u64), // z0[1]
                Bn254Fr::from(17u64), // zi[0]
                Bn254Fr::from(19u64), // zi[1]
            ],
        );
    }
}

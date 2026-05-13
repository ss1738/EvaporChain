//! End-to-end integration test for the real-fixture-witness
//! proof pipeline. Companion to `full_pipeline.rs` (#148), which
//! exercises the same wrappers on the dummy witness.
//!
//! Pipeline under test:
//!
//! ```text
//!   generate_fixture(num_steps)
//!     → extract_committed_hashes_via_serde      (#151)
//!     → scalar_adapter                          (#143)
//!     → build_circuit_from_fixture              (#152)
//!     → groth16_wrapper::setup                  (#145)
//!     → groth16_wrapper::prove                  (#146)
//!     → proof_to_eip197_bytes                   (#147)
//!     → eip197_bytes_to_proof                   (#147, decode)
//!     → groth16_wrapper::verify                 (#146)
//! ```
//!
//! Pins:
//! - Real (non-zero) committed hashes flow through.
//! - The encoded → decoded proof verifies against the real
//!   public-input slice.
//! - A different fixture (different num_steps) produces a proof
//!   that does NOT verify against the original public inputs —
//!   guards against the proof being "universal" for any binding.
//!
//! Runs `generate_fixture` 2× per test, so expect ~30s on Mini 1.

use ark_bn254::Fr as Bn254Fr;
use evaporchain_nova_bridge::circuit_builder::build_circuit_from_fixture;
use evaporchain_nova_bridge::eip197::{eip197_bytes_to_proof, proof_to_eip197_bytes, EIP197_PROOF_BYTES};
use evaporchain_nova_bridge::groth16_wrapper::{prove, public_inputs_for, setup, verify};
use evaporchain_nova_bridge::recursive_snark_fixture::generate_fixture;

use ark_std::rand::SeedableRng;

#[test]
fn real_fixture_pipeline_accepts_through_eip197_round_trip() {
    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(606);
    let (pk, vk) = setup(&mut rng).expect("setup");

    let rs = generate_fixture(2).expect("nova fixture");
    let circuit = build_circuit_from_fixture(&rs).expect("build circuit from fixture");

    // Real hashes (non-zero) — same pin as
    // `circuit_builder::fixture_to_circuit_to_satisfied_cs` but
    // through the public crate API.
    assert!(
        circuit.committed_hash_primary != Bn254Fr::from(0u64)
            || circuit.committed_hash_secondary != Bn254Fr::from(0u64),
        "real fixture must produce at least one non-zero committed hash"
    );

    let public_inputs = public_inputs_for(&circuit);
    let proof = prove(&pk, circuit, &mut rng).expect("prove");

    // Codec round-trip.
    let bytes = proof_to_eip197_bytes(&proof);
    assert_eq!(bytes.len(), EIP197_PROOF_BYTES);
    let decoded = eip197_bytes_to_proof(&bytes).expect("decode");

    // Verify against the DECODED proof + the REAL public inputs.
    let accepted = verify(&vk, &public_inputs, &decoded).expect("verify");
    assert!(
        accepted,
        "real-fixture proof must accept against real public inputs after eip197 round-trip"
    );
}

#[test]
fn real_fixture_proof_is_bound_to_its_specific_public_inputs() {
    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(707);
    let (pk, vk) = setup(&mut rng).expect("setup");

    // Two distinct fixtures — different num_steps so the
    // committed hashes AND zi differ.
    let rs_a = generate_fixture(2).expect("fixture a");
    let rs_b = generate_fixture(3).expect("fixture b");

    let circuit_a = build_circuit_from_fixture(&rs_a).expect("build a");
    let circuit_b = build_circuit_from_fixture(&rs_b).expect("build b");

    let pi_a = public_inputs_for(&circuit_a);
    let pi_b = public_inputs_for(&circuit_b);

    // Sanity: different fixtures → different public-input slices.
    assert_ne!(
        pi_a, pi_b,
        "different fixtures must produce different public-input slices"
    );

    // Prove fixture A; verify accepts pi_a, rejects pi_b.
    let proof_a = prove(&pk, circuit_a, &mut rng).expect("prove a");

    assert!(
        verify(&vk, &pi_a, &proof_a).expect("verify a/a"),
        "proof_a must accept against pi_a"
    );
    assert!(
        !verify(&vk, &pi_b, &proof_a).expect("verify a/b"),
        "proof_a must REJECT against pi_b — proof is bound to its specific public inputs"
    );
}

//! End-to-end integration test exercising every Phase 2 wrapper
//! introduced after the Section 1 / Section 2 substrate landed:
//!
//!   setup (#145) → prove (#146) → eip197 encode (#147) →
//!   eip197 decode → verify (#146) → assert true
//!
//! Audit B-1/B-2 S2b: runs on `NovaVerifierCircuit::setup_shape()`
//! — the canonical section-bearing circuit `setup()` keys pk/vk
//! over — so the pipeline exercises a REAL, non-vacuous proof
//! (the old `dummy()` version round-tripped an empty-circuit
//! proof). `setup_shape()` sources neptune params from the
//! embedded asset, so this needs no `/tmp` dump and stays a
//! standard (non-ignored) integration test.

use ark_std::rand::SeedableRng;
use evaporchain_nova_bridge::eip197::{eip197_bytes_to_proof, proof_to_eip197_bytes, EIP197_PROOF_BYTES};
use evaporchain_nova_bridge::groth16_wrapper::{prove, public_inputs_for, setup, verify};
use evaporchain_nova_bridge::verifier_circuit::NovaVerifierCircuit;

/// Full end-to-end on the dummy circuit:
///   setup → prove → encode → decode → verify(accepted).
///
/// Also confirms the encoded proof is exactly 256 bytes and that
/// the decoded proof verifies (catches the case where the codec
/// silently loses information that doesn't show up in a
/// shallow `assert_eq!` of the `Proof` struct fields).
#[test]
fn full_pipeline_setup_shape_setup_prove_encode_decode_verify_accepts() {
    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(404);
    let (pk, vk) = setup(&mut rng).expect("setup");

    let circuit = NovaVerifierCircuit::setup_shape().expect("setup_shape");
    let public_inputs = public_inputs_for(&circuit);
    let proof = prove(&pk, circuit, &mut rng).expect("prove");

    // Codec round-trip.
    let bytes = proof_to_eip197_bytes(&proof);
    assert_eq!(bytes.len(), EIP197_PROOF_BYTES);
    let decoded = eip197_bytes_to_proof(&bytes).expect("decode");

    // Verify against the DECODED proof — catches any codec
    // information loss that a struct-field equality check would
    // miss.
    let accepted = verify(&vk, &public_inputs, &decoded).expect("verify decoded");
    assert!(
        accepted,
        "full pipeline must accept decoded proof against setup_shape public inputs"
    );

    // Belt-and-suspenders: original (pre-encode) proof also verifies.
    assert!(
        verify(&vk, &public_inputs, &proof).expect("verify original"),
        "original proof must also accept against setup_shape public inputs"
    );
}

/// Negative-path pin: a decoded proof against tampered public
/// inputs must reject. Catches a regression where the codec
/// accidentally writes the public inputs INTO the proof bytes
/// (impossible by design, but the test costs nothing).
#[test]
fn full_pipeline_decoded_proof_rejects_tampered_public_inputs() {
    use ark_bn254::Fr as Bn254Fr;

    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(505);
    let (pk, vk) = setup(&mut rng).expect("setup");

    let circuit = NovaVerifierCircuit::setup_shape().expect("setup_shape");
    let mut public_inputs = public_inputs_for(&circuit);
    let proof = prove(&pk, circuit, &mut rng).expect("prove");

    let decoded = eip197_bytes_to_proof(&proof_to_eip197_bytes(&proof)).expect("decode");

    // Perturb committed_hash_primary so verification must reject.
    public_inputs[0] += Bn254Fr::from(1u64);
    let rejected = verify(&vk, &public_inputs, &decoded).expect("verify");
    assert!(
        !rejected,
        "decoded proof must reject tampered public inputs"
    );
}

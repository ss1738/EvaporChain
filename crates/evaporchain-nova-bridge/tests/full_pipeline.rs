//! End-to-end integration test exercising every Phase 2 wrapper
//! introduced after the Section 1 / Section 2 substrate landed:
//!
//!   setup (#145) → prove (#146) → eip197 encode (#147) →
//!   eip197 decode → verify (#146) → assert true
//!
//! Runs on `NovaVerifierCircuit::dummy()` because Sections 2 and
//! 3 are still TODO in `generate_constraints` and the
//! `l_u_secondary` access gap blocks real fixture witnesses
//! (see [`evaporchain_nova_bridge::circuit_builder`] module
//! docs). The dummy round-trip pins that the wrapper chain
//! doesn't drop bytes anywhere between Rust and the L1
//! wire-format.
//!
//! When Section 2 + 3 wire up and the access gap closes, this
//! test's `NovaVerifierCircuit::dummy()` swap-in becomes
//! `circuit_builder::build_circuit_from_fixture(&rs)` and the
//! same pipeline produces a real L1-verifiable proof — no other
//! wrapper changes.

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
fn full_pipeline_dummy_setup_prove_encode_decode_verify_accepts() {
    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(404);
    let (pk, vk) = setup(&mut rng).expect("setup");

    let dummy = NovaVerifierCircuit::dummy();
    let public_inputs = public_inputs_for(&dummy);
    let proof = prove(&pk, dummy, &mut rng).expect("prove");

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
        "full pipeline must accept decoded proof against dummy public inputs"
    );

    // Belt-and-suspenders: original (pre-encode) proof also verifies.
    assert!(
        verify(&vk, &public_inputs, &proof).expect("verify original"),
        "original proof must also accept against dummy public inputs"
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

    let dummy = NovaVerifierCircuit::dummy();
    let mut public_inputs = public_inputs_for(&dummy);
    let proof = prove(&pk, dummy, &mut rng).expect("prove");

    let decoded = eip197_bytes_to_proof(&proof_to_eip197_bytes(&proof)).expect("decode");

    // Bump committed_hash_primary from 0 to 1.
    public_inputs[0] = Bn254Fr::from(1u64);
    let rejected = verify(&vk, &public_inputs, &decoded).expect("verify");
    assert!(
        !rejected,
        "decoded proof must reject tampered public inputs"
    );
}

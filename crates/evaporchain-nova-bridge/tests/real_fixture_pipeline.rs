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
//! Audit B-1/B-2 S2b: `build_circuit_from_fixture` attaches NO
//! section witnesses. Under the mandatory-binding contract such a
//! circuit is no longer provable, so these tests pin the SECURITY
//! property end-to-end via the public crate API: the section-less
//! pipeline is rejected (the constraint-vacuity hole is closed all
//! the way to `prove()`).
//!
//! The POSITIVE real-fixture+sections pipeline (real proof, codec
//! round-trip, fixture-specific proof binding) is covered by the S6
//! determinism proof (`circuit_builder::tests::
//! s2a_setup_shape_matches_real_prover_r1cs`) and the `#[ignore]`d
//! `build_circuit_with_section2/3_synthesizes_and_is_satisfied`
//! tests; full public-API positive wiring is the dedicated
//! S2b-prover sub-stage (SOUNDNESS_REBUILD_SPEC.md).
//!
//! Runs `generate_fixture` once, so expect ~15s on Mini 1.

use ark_bn254::Fr as Bn254Fr;
use evaporchain_nova_bridge::circuit_builder::build_circuit_from_fixture;
use evaporchain_nova_bridge::groth16_wrapper::{prove, setup};
use evaporchain_nova_bridge::recursive_snark_fixture::generate_fixture;

use ark_std::rand::SeedableRng;

/// Section-less real-fixture circuit is unprovable end-to-end —
/// the vacuity hole is closed through the public crate API, not
/// just at the unit level.
#[test]
fn real_fixture_section_less_pipeline_is_rejected() {
    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(606);
    let (pk, _vk) = setup(&mut rng).expect("setup");

    let rs = generate_fixture(2).expect("nova fixture");
    let circuit = build_circuit_from_fixture(&rs).expect("build circuit from fixture");

    // Extraction still works (real, non-zero committed hashes) —
    // the rejection is the *binding* contract, not an extraction
    // failure.
    assert!(
        circuit.committed_hash_primary != Bn254Fr::from(0u64)
            || circuit.committed_hash_secondary != Bn254Fr::from(0u64),
        "real fixture must still produce at least one non-zero committed hash"
    );

    let result = prove(&pk, circuit, &mut rng);
    assert!(
        result.is_err(),
        "section-less real-fixture circuit must be unprovable under S2b, got Ok"
    );
}

//! End-to-end integration pin for the Phase 2 wrapper pipeline
//! (setup → prove → eip197 codec → verify).
//!
//! Audit B-1/B-2 S2b: the positive pipeline (a REAL proof through
//! the eip197 codec) is covered by the `#[ignore]`d real-fixture
//! tests `eip197::tests::proof_round_trips_through_eip197_bytes`
//! and `groth16_wrapper::tests::
//! prove_and_verify_real_fixture_round_trip_accepts` (a satisfiable
//! witness now requires a real Nova fixture; `setup_shape()` is a
//! zeroed shape-only template and cannot yield a verifying proof).
//!
//! This integration test pins the SECURITY property end-to-end
//! through the public crate API: a section-less circuit is rejected
//! at `prove()` — the pipeline can never even reach the codec to
//! emit a forgeable empty-circuit proof. Complements
//! `real_fixture_pipeline.rs` (same property, fixture entry point).

use ark_std::rand::SeedableRng;
use evaporchain_nova_bridge::circuit_builder::build_circuit_from_fixture;
use evaporchain_nova_bridge::groth16_wrapper::{prove, setup};
use evaporchain_nova_bridge::recursive_snark_fixture::generate_fixture;

/// The public setup→prove pipeline refuses a section-less circuit:
/// `prove()` surfaces the mandatory-binding rejection, so no
/// vacuous-circuit proof is ever produced or encoded.
#[test]
fn full_pipeline_section_less_circuit_is_rejected_before_codec() {
    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(404);
    let (pk, _vk) = setup(&mut rng).expect("setup");

    let rs = generate_fixture(2).expect("nova fixture");
    let circuit = build_circuit_from_fixture(&rs).expect("build circuit from fixture");

    let result = prove(&pk, circuit, &mut rng);
    assert!(
        result.is_err(),
        "section-less circuit must be unprovable end-to-end (pipeline must \
         not reach the eip197 codec with a vacuous-circuit proof), got Ok"
    );
}

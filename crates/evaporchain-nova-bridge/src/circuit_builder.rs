//! Adapter-time orchestration: pulls a real `RecursiveSNARK<E1, E2,
//! TrivialIncrementCircuit>` fixture (from [`crate::recursive_snark_fixture::generate_fixture`])
//! through [`crate::scalar_adapter`] and produces a populated
//! [`crate::verifier_circuit::NovaVerifierCircuit`] ready to be
//! synthesised by Groth16.
//!
//! # What this ships
//!
//! [`build_circuit_from_fixture`] — single entry point that maps:
//!
//! | NovaVerifierCircuit field   | Source on `RecursiveSNARK`            |
//! |-----------------------------|---------------------------------------|
//! | `num_steps`                 | `rs.num_steps()` (public)             |
//! | `zi`                        | `rs.outputs()` (public)               |
//! | `z0`                        | `[Scalar1::ZERO]` (fixture contract)  |
//! | `committed_hash_primary`    | `rs.l_u_secondary.X[0]` via serde     |
//! | `committed_hash_secondary`  | `rs.l_u_secondary.X[1]` via serde     |
//!
//! # The l_u_secondary access gap is closed via serde reflection
//!
//! `RecursiveSNARK::l_u_secondary` is a private field in nova-snark
//! v0.68 (`nova-snark-0.68/src/nova/mod.rs:338`) with no public
//! accessor. [`crate::l_u_secondary_extract::extract_committed_hashes_via_serde`]
//! works around this by `serde_json::to_value(rs)` — the
//! `#[derive(Serialize)]` on both `RecursiveSNARK` and `R1CSInstance`
//! makes the field tree walkable by name without duplicating types.
//!
//! This is a brittle workaround pinned to nova-snark v0.68's
//! named-field layout. The proper resolution is an upstream PR
//! adding a `pub fn l_u_secondary(&self) -> &R1CSInstance<E2>`
//! accessor. See `l_u_secondary_extract` module docs for the
//! drift-detection test pin.
//!
//! # What real witness values mean for Section 2 status
//!
//! With real committed hashes flowing through, the circuit's
//! `committed_hash_primary` and `committed_hash_secondary`
//! witness slots now bind to actual Nova accumulator state. The
//! Groth16 proof produced via [`crate::groth16_wrapper::prove`]
//! still verifies (because Section 2 / Section 3 are still TODO
//! in `generate_constraints` — no constraint actually consumes
//! these witnesses yet), but the proof is no longer the
//! "trivial zero-witness" proof: it commits to a specific
//! accumulator state.
//!
//! When Sections 2 + 3 wire up, the same `build_circuit_from_fixture`
//! call site keeps working — at that point the proof becomes
//! both bound AND verifying-with-soundness.

use crate::l_u_secondary_extract::{extract_committed_hashes_via_serde, ExtractError};
use crate::recursive_snark_fixture::{TrivialIncrementCircuit, E1, E2};
use crate::scalar_adapter::primary_to_ark_fr;
use crate::verifier_circuit::NovaVerifierCircuit;
use ark_bn254::Fr as ArkFr;
use nova_snark::nova::RecursiveSNARK;

/// Map a real `RecursiveSNARK` fixture into the populated
/// `NovaVerifierCircuit` witness shape — including real
/// `l_u_secondary.X[..2]` values extracted via serde reflection.
///
/// Returns `Err` only if the extraction step fails (nova-snark
/// layout drift). See module docstring for the field-by-field
/// mapping table.
pub fn build_circuit_from_fixture(
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
) -> Result<NovaVerifierCircuit, ExtractError> {
    let num_steps = rs.num_steps() as u64;

    // z0 is hard-coded by `generate_fixture` to `[Scalar1::ZERO]`;
    // mirror that contract in ark-side units.
    let z0 = vec![ArkFr::from(0u64)];

    // zi from the public `outputs()` accessor, type-changed
    // through the same-field scalar adapter.
    let zi: Vec<ArkFr> = rs.outputs().iter().copied().map(primary_to_ark_fr).collect();

    // Real committed hashes from l_u_secondary.X[..2], via serde
    // reflection (see crate::l_u_secondary_extract).
    let (committed_hash_primary, committed_hash_secondary) =
        extract_committed_hashes_via_serde(rs)?;

    Ok(NovaVerifierCircuit::new(
        num_steps,
        z0,
        zi,
        committed_hash_primary,
        committed_hash_secondary,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recursive_snark_fixture::generate_fixture;
    use ark_relations::r1cs::ConstraintSynthesizer;
    use ark_relations::r1cs::ConstraintSystem;

    /// End-to-end pin: real fixture → adapter → satisfied CS.
    ///
    /// Builds a 2-step `RecursiveSNARK`, runs it through
    /// `build_circuit_from_fixture` (with real committed hashes
    /// from `l_u_secondary.X[..2]` via serde reflection),
    /// synthesises with arkworks, and confirms `cs.is_satisfied()`.
    ///
    /// Caveat per module docstring: Section 2 in-circuit re-hash is
    /// still TODO, so the proof binds the witness to a specific
    /// accumulator state but doesn't yet verify Nova-soundness in
    /// circuit. That gap closes when Section 2 wires up — the same
    /// `build_circuit_from_fixture` call site keeps working.
    #[test]
    fn fixture_to_circuit_to_satisfied_cs() {
        let rs = generate_fixture(2).expect("generate 2-step fixture");
        let circuit = build_circuit_from_fixture(&rs).expect("build");

        assert_eq!(circuit.num_steps, 2, "num_steps must be 2");
        assert_eq!(circuit.z0.len(), 1, "z0 arity matches TrivialIncrementCircuit");
        assert_eq!(circuit.zi.len(), 1, "zi arity matches z0");
        // TrivialIncrementCircuit increments [0] by 1 per step; after 2 steps z_i = 2.
        assert_eq!(
            circuit.zi[0],
            ArkFr::from(2u64),
            "zi must equal 2 after 2 increment steps from z0=0"
        );

        // Real (non-zero) committed hashes — see
        // `l_u_secondary_extract::extract_returns_non_trivial_hashes_for_real_fixture`
        // for the lower-level pin.
        assert!(
            circuit.committed_hash_primary != ArkFr::from(0u64)
                || circuit.committed_hash_secondary != ArkFr::from(0u64),
            "real fixture must produce at least one non-zero committed hash — \
             zero on both sides would indicate the extraction path silently failed"
        );

        let cs = ConstraintSystem::<ArkFr>::new_ref();
        circuit
            .generate_constraints(cs.clone())
            .expect("synthesize real-fixture circuit");
        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "real-fixture circuit must produce a satisfied CS today (Section 2 TODO; \
             only Section 1 structural gate is active)"
        );

        // Public-input arity contract: 2 hashes + |z0| + |zi| + 1 const = 5.
        assert_eq!(cs.num_instance_variables(), 5);
    }

    /// Same shape with 5 steps to confirm zi tracks num_steps.
    #[test]
    fn fixture_to_circuit_zi_tracks_num_steps() {
        let rs = generate_fixture(5).expect("generate 5-step fixture");
        let circuit = build_circuit_from_fixture(&rs).expect("build");
        assert_eq!(circuit.num_steps, 5);
        assert_eq!(circuit.zi[0], ArkFr::from(5u64), "zi after 5 increments");
    }

    /// The Section 1 gate must accept a real fixture (num_steps
    /// non-zero, balanced z0/zi).
    #[test]
    fn fixture_to_circuit_passes_structural_validation() {
        let rs = generate_fixture(1).expect("generate 1-step fixture");
        let circuit = build_circuit_from_fixture(&rs).expect("build");
        assert_eq!(circuit.validate_structurally(), Ok(()));
    }
}

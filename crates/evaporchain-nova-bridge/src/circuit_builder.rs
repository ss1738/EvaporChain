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
//! | NovaVerifierCircuit field   | Source on `RecursiveSNARK`        | Status |
//! |-----------------------------|-----------------------------------|--------|
//! | `num_steps`                 | `rs.num_steps()` (public)         | wired  |
//! | `zi`                        | `rs.outputs()` (public)           | wired  |
//! | `z0`                        | `[Scalar1::ZERO]` (fixture contract) | wired  |
//! | `committed_hash_primary`    | `rs.l_u_secondary.X[0]` (private) | PLACEHOLDER ZERO |
//! | `committed_hash_secondary`  | `rs.l_u_secondary.X[1]` (private) | PLACEHOLDER ZERO |
//!
//! # The l_u_secondary access gap
//!
//! `RecursiveSNARK::l_u_secondary` is a private field in nova-snark
//! v0.68 (`nova-snark-0.68/src/nova/mod.rs:338`) and has **no public
//! accessor**. The two committed hashes we need for Section 2's
//! transcript check live at `l_u_secondary.X[..2]`.
//!
//! Three paths to close this gap, in order of cleanness:
//!
//! 1. **Upstream PR to nova-snark** adding `pub fn l_u_secondary(&self)
//!    -> &R1CSInstance<E2>` (or just `pub fn committed_hashes(&self)
//!    -> [E2::Scalar; 2]`). Cleanest but requires upstream merge.
//! 2. **Re-run the Poseidon transcript ourselves** using
//!    [`crate::neptune_reference::neptune_hash_primary`] on the same
//!    inputs `RecursiveSNARK::verify` consumes (pp.digest, num_steps,
//!    z0, zi, r_U_secondary instance fields, ri_primary). Doesn't
//!    require any nova change; this is what Section 2's in-circuit
//!    re-hash does anyway. Effectively merges the "extract" step
//!    with the "verify" step.
//! 3. **Carry our own fork** of nova-snark that exposes the field.
//!    Heaviest; only if upstream is unresponsive.
//!
//! Path 2 is the natural next milestone for Phase 2.3 — wires up
//! the off-circuit oracle as the source of truth.
//!
//! # Why ship the scaffold with placeholder hashes
//!
//! Section 2 is still a TODO in `NovaVerifierCircuit::generate_constraints`,
//! so the committed-hash variables are allocated as public inputs
//! but NEVER constrained against the re-hashed value. A
//! `NovaVerifierCircuit` with zeroed hash fields therefore
//! synthesises a *valid* (satisfied) constraint system today —
//! enough to confirm the structural wiring end-to-end. Once
//! Section 2's in-circuit re-hash lands, the placeholder zeros
//! become the real Section-2 mismatch the canary documents, and
//! this adapter MUST move to path 2 (compute the hashes off-circuit)
//! before any real proof can be generated.

use crate::recursive_snark_fixture::{TrivialIncrementCircuit, E1, E2};
use crate::scalar_adapter::primary_to_ark_fr;
use crate::verifier_circuit::NovaVerifierCircuit;
use ark_bn254::Fr as ArkFr;
use nova_snark::nova::RecursiveSNARK;

/// Map a real `RecursiveSNARK` fixture into the populated
/// `NovaVerifierCircuit` witness shape, via the scalar adapter.
///
/// See module docstring for the field-by-field mapping table and
/// the documented gap on `l_u_secondary` access.
pub fn build_circuit_from_fixture(
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
) -> NovaVerifierCircuit {
    let num_steps = rs.num_steps() as u64;

    // z0 is hard-coded by `generate_fixture` to `[Scalar1::ZERO]`;
    // we mirror that contract in ark-side units.
    let z0 = vec![ArkFr::from(0u64)];

    // zi from the public `outputs()` accessor, type-changed
    // through the same-field scalar adapter.
    let zi: Vec<ArkFr> = rs.outputs().iter().copied().map(primary_to_ark_fr).collect();

    // l_u_secondary is private — see module docstring path 2 for
    // the planned resolution. Zero placeholders are valid today
    // because Section 2 is still TODO in generate_constraints
    // (no constraint actually consumes these witness values yet).
    let committed_hash_primary = ArkFr::from(0u64);
    let committed_hash_secondary = ArkFr::from(0u64);

    NovaVerifierCircuit::new(
        num_steps,
        z0,
        zi,
        committed_hash_primary,
        committed_hash_secondary,
    )
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
    /// `build_circuit_from_fixture`, synthesises with arkworks,
    /// and confirms `cs.is_satisfied()`. The first real
    /// "Nova fixture → bridge circuit" round-trip on `main`.
    ///
    /// Caveat per module docstring: committed hashes are zero
    /// placeholders, so this proves only the structural wiring,
    /// not Section 2 transcript correctness.
    #[test]
    fn fixture_to_circuit_to_satisfied_cs() {
        let rs = generate_fixture(2).expect("generate 2-step fixture");
        let circuit = build_circuit_from_fixture(&rs);

        assert_eq!(circuit.num_steps, 2, "num_steps must be 2");
        assert_eq!(circuit.z0.len(), 1, "z0 arity matches TrivialIncrementCircuit");
        assert_eq!(circuit.zi.len(), 1, "zi arity matches z0");
        // TrivialIncrementCircuit increments [0] by 1 per step; after 2 steps z_i = 2.
        assert_eq!(
            circuit.zi[0],
            ArkFr::from(2u64),
            "zi must equal 2 after 2 increment steps from z0=0"
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
        let circuit = build_circuit_from_fixture(&rs);
        assert_eq!(circuit.num_steps, 5);
        assert_eq!(circuit.zi[0], ArkFr::from(5u64), "zi after 5 increments");
    }

    /// The Section 1 gate must accept a real fixture (num_steps
    /// non-zero, balanced z0/zi).
    #[test]
    fn fixture_to_circuit_passes_structural_validation() {
        let rs = generate_fixture(1).expect("generate 1-step fixture");
        let circuit = build_circuit_from_fixture(&rs);
        assert_eq!(circuit.validate_structurally(), Ok(()));
    }
}

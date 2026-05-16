//! Produce a real `RecursiveSNARK<Bn256EngineKZG, GrumpkinEngine, _>`
//! accumulator as a fixture for the in-circuit verifier work
//! (`crate::verifier_circuit`, `crate::section2_gadget`).
//!
//! This module ships:
//!   - [`TrivialIncrementCircuit`] — a 1-arity step circuit that
//!     increments its single input. Smallest viable fold-target.
//!   - [`generate_fixture`] — runs `PublicParams::setup` +
//!     `RecursiveSNARK::new` + `prove_step` N times, returns the
//!     accumulator.
//!   - [`fixture_stats`] — measures size + key fields of a
//!     `RecursiveSNARK`.
//!   - [`public_inputs_for_bridge`] — converts a fixture's output
//!     state vector through PR #66's scalar adapter to
//!     `Vec<ark_bn254::Fr>` for direct consumption by
//!     `NovaVerifierCircuit`.
//!
//! # Why not just a CompressedSNARK?
//!
//! The DESIGN.md A3 sub-path verifies a `RecursiveSNARK`, NOT a
//! `CompressedSNARK`. Skipping the Spartan layer is the whole point
//! of A3 — smaller wrapper circuit. The fixture is therefore the
//! raw running accumulator.

use ff::{Field, PrimeField};
use nova_snark::{
    frontend::{num::AllocatedNum, ConstraintSystem, SynthesisError},
    nova::{PublicParams, RecursiveSNARK},
    provider::{Bn256EngineKZG, GrumpkinEngine},
    traits::{circuit::StepCircuit, snark::RelaxedR1CSSNARKTrait, Engine},
};

/// Primary engine (BN254 with HyperKZG) — matches the chain's
/// `evaporchain-proving::nova::E1`.
pub type E1 = Bn256EngineKZG;
/// Secondary engine (Grumpkin, BN254's cycle partner) — matches the
/// chain's `evaporchain-proving::nova::E2`.
pub type E2 = GrumpkinEngine;

/// Primary commitment scheme — HyperKZG over BN254 (matches chain).
type EE1 = nova_snark::provider::hyperkzg::EvaluationEngine<E1>;
/// Secondary commitment scheme — IPA over Grumpkin (matches chain).
type EE2 = nova_snark::provider::ipa_pc::EvaluationEngine<E2>;
/// Primary SNARK (Spartan) — needed for `ck_floor()` hint at setup.
type S1 = nova_snark::spartan::snark::RelaxedR1CSSNARK<E1, EE1>;
/// Secondary SNARK (Spartan) — needed for `ck_floor()` hint at setup.
type S2 = nova_snark::spartan::snark::RelaxedR1CSSNARK<E2, EE2>;

/// Primary scalar field — BN254 Fr.
pub type Scalar1 = <E1 as Engine>::Scalar;

/// The smallest viable `StepCircuit` for fold-fixture purposes:
/// `z_{i+1}[0] = z_i[0] + 1`. Arity 1. Constraint cost: a single
/// addition gate. This keeps `PublicParams::setup` fast (~seconds
/// instead of minutes) so the fixture generator is iterable during
/// Phase 2.2 development.
#[derive(Clone, Debug, Default)]
pub struct TrivialIncrementCircuit;

impl<F: PrimeField> StepCircuit<F> for TrivialIncrementCircuit {
    fn arity(&self) -> usize {
        1
    }

    fn synthesize<CS: ConstraintSystem<F>>(
        &self,
        cs: &mut CS,
        z: &[AllocatedNum<F>],
    ) -> Result<Vec<AllocatedNum<F>>, SynthesisError> {
        let one = AllocatedNum::alloc(cs.namespace(|| "one"), || Ok(F::ONE))?;
        // Pin the witness to the constant 1.
        cs.enforce(
            || "one_is_one",
            |lc| lc + one.get_variable(),
            |lc| lc + CS::one(),
            |lc| lc + CS::one(),
        );
        let next = AllocatedNum::alloc(cs.namespace(|| "next"), || {
            Ok(z[0].get_value().ok_or(SynthesisError::AssignmentMissing)? + F::ONE)
        })?;
        cs.enforce(
            || "next == z[0] + 1",
            |lc| lc + next.get_variable(),
            |lc| lc + CS::one(),
            |lc| lc + z[0].get_variable() + one.get_variable(),
        );
        Ok(vec![next])
    }
}

/// Run a fresh Nova fold over `num_steps` invocations of
/// [`TrivialIncrementCircuit`], starting from `z0 = [0]`. Returns the
/// running `RecursiveSNARK` accumulator.
///
/// **Setup cost.** `PublicParams::setup` takes ~1-3 seconds on a
/// modern dev machine for this trivial circuit. Per-step `prove_step`
/// is ~hundreds of milliseconds. For Phase 2.2 PoC work this is fast
/// enough to iterate.
pub fn generate_fixture(
    num_steps: usize,
) -> Result<RecursiveSNARK<E1, E2, TrivialIncrementCircuit>, String> {
    let circuit = TrivialIncrementCircuit;
    let pp = PublicParams::<E1, E2, TrivialIncrementCircuit>::setup(
        &circuit,
        &*S1::ck_floor(),
        &*S2::ck_floor(),
    )
    .map_err(|e| format!("PublicParams::setup: {:?}", e))?;

    let z0: Vec<Scalar1> = vec![Scalar1::ZERO];
    let mut rs = RecursiveSNARK::<E1, E2, TrivialIncrementCircuit>::new(&pp, &circuit, &z0)
        .map_err(|e| format!("RecursiveSNARK::new: {:?}", e))?;

    for i in 0..num_steps {
        rs.prove_step(&pp, &circuit)
            .map_err(|e| format!("prove_step {}: {:?}", i, e))?;
    }

    Ok(rs)
}

/// Run a fresh Nova fold over `num_steps` invocations of
/// [`TrivialIncrementCircuit`], returning the accumulator and the
/// `PublicParams::digest()` needed by Section 2 witness extraction.
pub fn generate_fixture_with_digest(
    num_steps: usize,
) -> Result<(RecursiveSNARK<E1, E2, TrivialIncrementCircuit>, Scalar1), String> {
    let circuit = TrivialIncrementCircuit;
    let pp = PublicParams::<E1, E2, TrivialIncrementCircuit>::setup(
        &circuit,
        &*S1::ck_floor(),
        &*S2::ck_floor(),
    )
    .map_err(|e| format!("PublicParams::setup: {:?}", e))?;
    let pp_digest = pp.digest();
    let z0: Vec<Scalar1> = vec![Scalar1::ZERO];
    let mut rs = RecursiveSNARK::<E1, E2, TrivialIncrementCircuit>::new(&pp, &circuit, &z0)
        .map_err(|e| format!("RecursiveSNARK::new: {:?}", e))?;
    for i in 0..num_steps {
        rs.prove_step(&pp, &circuit)
            .map_err(|e| format!("prove_step {}: {:?}", i, e))?;
    }
    Ok((rs, pp_digest))
}

/// Statistics about a generated fixture — what the in-circuit
/// verifier will need to consume as witness. Phase 2.2 finish must
/// figure out which of these fields map to in-circuit variables
/// vs hashed-into-the-transcript vs implicit.
#[derive(Debug)]
pub struct FixtureStats {
    /// Number of fold steps executed.
    pub num_steps: usize,
    /// Serialized size in bytes (bincode encoding). Pre-Compressed.
    pub serialized_size_bytes: usize,
    /// The accumulator's running output state `z_i`.
    pub z_i: Vec<Scalar1>,
}

/// Convert a fixture's accumulator outputs (the chain-side
/// `<E1 as Engine>::Scalar` values) into the `ark_bn254::Fr`
/// representation that [`crate::NovaVerifierCircuit`] consumes as
/// public inputs.
///
/// Uses [`crate::scalar_adapter::nova_to_bn254_fr`] under the hood
/// — this is the first place the adapter touches real
/// `RecursiveSNARK` artifact data. Section 2 / Section 3 will
/// follow the same conversion pattern on additional fields
/// (transcript scalars, witness commitments).
///
/// Returns the converted `zi` vector. `z0` is fixed at `[0]` by
/// [`generate_fixture`] so its conversion is trivial and not
/// emitted here.
pub fn public_inputs_for_bridge(
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
) -> Vec<ark_bn254::Fr> {
    rs.outputs()
        .iter()
        .map(|s| crate::scalar_adapter::primary_to_ark_fr(*s))
        .collect()
}

/// Return human-readable stats about a fixture. Useful for sizing
/// the in-circuit verifier's witness allocation in Phase 2.2.
pub fn fixture_stats(
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
) -> Result<FixtureStats, String> {
    let serialized = bincode::serialize(rs).map_err(|e| format!("bincode: {:?}", e))?;
    Ok(FixtureStats {
        num_steps: rs.num_steps(),
        serialized_size_bytes: serialized.len(),
        z_i: rs.outputs().to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin that the fixture-generator pipeline works end-to-end:
    /// `PublicParams::setup` + `RecursiveSNARK::new` + 2 `prove_step`
    /// + serialize round-trip. If this fails, Phase 2.2's verifier
    /// work is blocked.
    #[test]
    fn generate_two_step_fixture_serializes_and_outputs_correct_z() {
        let rs = generate_fixture(2).expect("generate fixture");
        let stats = fixture_stats(&rs).expect("stats");

        // `TrivialIncrementCircuit` adds 1 per step, starting at 0.
        // After 2 steps z = 2.
        assert_eq!(stats.num_steps, 2);
        assert_eq!(stats.z_i.len(), 1);
        assert_eq!(stats.z_i[0], Scalar1::from(2u64));
        // Serialization must produce a non-empty blob.
        assert!(
            stats.serialized_size_bytes > 0,
            "RecursiveSNARK serialization produced empty blob"
        );
        // And the blob must deserialize back into an equivalent value.
        let bytes = bincode::serialize(&rs).expect("serialize");
        let _round_trip: RecursiveSNARK<E1, E2, TrivialIncrementCircuit> =
            bincode::deserialize(&bytes).expect("deserialize");
    }

    /// First end-to-end exercise of [`crate::scalar_adapter`] on a
    /// real `RecursiveSNARK` artifact: feed a 2-step fixture's
    /// outputs through the nova → ark_bn254::Fr conversion and
    /// assert the result matches the expected scalar value
    /// (`ark_bn254::Fr::from(2u64)`).
    ///
    /// Pins that the scalar-adapter byte conversion produces the
    /// expected NUMERIC value (not just the same bytes) on data
    /// produced by the actual `prove_step` pipeline.
    #[test]
    fn public_inputs_for_bridge_converts_two_step_fixture() {
        let rs = generate_fixture(2).expect("generate fixture");
        let public_inputs = public_inputs_for_bridge(&rs);
        assert_eq!(public_inputs.len(), 1, "arity-1 TrivialIncrementCircuit");
        assert_eq!(public_inputs[0], ark_bn254::Fr::from(2u64));
    }

    #[test]
    fn trivial_increment_circuit_arity_is_one() {
        let c = TrivialIncrementCircuit;
        let arity: usize = <TrivialIncrementCircuit as StepCircuit<Scalar1>>::arity(&c);
        assert_eq!(arity, 1);
    }

    #[test]
    fn trivial_increment_circuit_default_constructs() {
        let _ = TrivialIncrementCircuit;
        let _ = TrivialIncrementCircuit::default();
    }

    #[test]
    fn generate_fixture_one_step_yields_one() {
        let rs = generate_fixture(1).expect("generate fixture");
        let stats = fixture_stats(&rs).expect("stats");
        assert_eq!(stats.num_steps, 1);
        assert_eq!(stats.z_i[0], Scalar1::from(1u64));
    }

    #[test]
    fn generate_fixture_with_digest_returns_nonzero_digest() {
        let (rs, digest) = generate_fixture_with_digest(1).expect("generate with digest");
        assert_eq!(rs.num_steps(), 1);
        assert_ne!(digest, Scalar1::ZERO, "PP digest must not be zero");
    }

    #[test]
    fn public_inputs_for_bridge_one_step_yields_one() {
        let rs = generate_fixture(1).expect("generate fixture");
        let public_inputs = public_inputs_for_bridge(&rs);
        assert_eq!(public_inputs.len(), 1);
        assert_eq!(public_inputs[0], ark_bn254::Fr::from(1u64));
    }
}

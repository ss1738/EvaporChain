//! B-1/B-2 EVM, option (1C) — increment 4a: the **multi-step
//! CycleFold fold accumulator** composition test.
//!
//! # Honest scope split (4a vs 4b)
//!
//! The flow originally framed increment 4 as "compose with
//! nova-snark's `RecursiveSNARK` (already validated)". That's
//! imprecise: reusing nova-snark's primary RecursiveSNARK keeps its
//! heavy original augmented circuit (~2¹⁷ secondary), so we wouldn't
//! actually get CycleFold's reduction. Getting CycleFold's
//! reduction requires authoring a NEW primary augmented circuit
//! (step + RO + emit `(P, s, Q)`, no heavy non-native E2
//! verification) — that is a substantial multi-day construction,
//! tracked separately as increment 4b.
//!
//! **This module (4a)** is the cheapest decisive sub-step that
//! validates the composition PATTERN without the real primary
//! augmented circuit: synthesise N cross-curve scalar-mul tuples,
//! bridge each to a `CycleFoldInstanceCircuit`, fold sequentially
//! via [`crate::cyclefold_r1cs_bridge`] + nova-snark NIFS, and
//! assert `is_sat_relaxed` after every step. If the accumulator
//! works for synthetic tuples it will work for real ones; only the
//! tuple SOURCE differs in 4b.

use crate::cyclefold_instance_circuit::CycleFoldInstanceCircuit;
use crate::cyclefold_r1cs_bridge::{
    arkworks_cs_to_nova_grumpkin_satisfied_pair, BridgeError, NovaGrumpkinR1CSArtifacts,
};
use crate::scalar_adapter::SecondaryScalar;
use ark_bn254::{Fr as Bn254Fr, G1Affine, G1Projective};
use ark_ec::CurveGroup;
use nova_snark::nova::nifs::NIFS;
use nova_snark::provider::GrumpkinEngine;
use nova_snark::r1cs::{
    R1CSShape, RelaxedR1CSInstance, RelaxedR1CSWitness,
};
use nova_snark::traits::ROConstants;

/// Bridge ONE synthetic cross-curve scalar-mul tuple `(P, s, Q)`
/// (the kind a real primary's per-step fold would emit) into a
/// satisfied nova-snark R1CS pair on `GrumpkinEngine`.
pub fn bridge_cf_tuple(
    p: G1Affine,
    s: Bn254Fr,
    q: G1Affine,
    ck_label: &'static [u8],
) -> Result<NovaGrumpkinR1CSArtifacts, BridgeError> {
    let circuit = CycleFoldInstanceCircuit::new(p, s, q);
    arkworks_cs_to_nova_grumpkin_satisfied_pair(circuit, ck_label)
}

/// Run `num_steps` of the CycleFold fold accumulator on synthetic
/// cross-curve tuples. Returns the final running relaxed pair +
/// the shape + the shared `ck`. After each fold the caller is
/// expected to have asserted `is_sat_relaxed`; this function panics
/// instead of returning silently if any step's fold fails to verify
/// (the test below treats this as the soundness gate).
pub fn run_synthetic_cf_accumulator(
    num_steps: usize,
    ck_label: &'static [u8],
) -> Result<
    (
        R1CSShape<GrumpkinEngine>,
        nova_snark::provider::pedersen::CommitmentKey<GrumpkinEngine>,
        RelaxedR1CSInstance<GrumpkinEngine>,
        RelaxedR1CSWitness<GrumpkinEngine>,
    ),
    BridgeError,
> {
    assert!(num_steps >= 1, "need ≥1 fold step");
    use ark_ff::UniformRand;
    use ark_std::test_rng;
    let mut rng = test_rng();

    let make_triple = |rng: &mut _| -> (G1Affine, Bn254Fr, G1Affine) {
        let p = G1Affine::generator();
        let s = Bn254Fr::rand(rng);
        let q = (G1Projective::from(p) * s).into_affine();
        (p, s, q)
    };

    // Step 0: bridge first tuple, lift to relaxed (the "running"
    // start).
    let (p0, s0, q0) = make_triple(&mut rng);
    let art0 = bridge_cf_tuple(p0, s0, q0, ck_label)?;
    let shape = art0.shape.clone();
    let ck = art0.ck.clone();
    let mut u_running = RelaxedR1CSInstance::<GrumpkinEngine>::from_r1cs_instance(
        &ck,
        &shape,
        &art0.instance,
    );
    let mut w_running = RelaxedR1CSWitness::<GrumpkinEngine>::from_r1cs_witness(
        &shape,
        &art0.witness,
    );
    // Sanity: starting relaxed pair satisfies shape.
    shape
        .is_sat_relaxed(&ck, &u_running, &w_running)
        .expect("initial lifted relaxed pair must satisfy shape");

    let ro_consts = ROConstants::<GrumpkinEngine>::default();
    let pp_digest = SecondaryScalar::from(0u64);

    // Folds 1..num_steps: each step bridges a fresh tuple and
    // composes it into the running pair via NIFS::prove. is_sat_
    // relaxed is the per-step soundness gate.
    for step in 1..num_steps {
        let (p, s, q) = make_triple(&mut rng);
        let art = bridge_cf_tuple(p, s, q, ck_label)?;
        let (_nifs, (u_new, w_new)) = NIFS::<GrumpkinEngine>::prove(
            &ck,
            &ro_consts,
            &pp_digest,
            &shape,
            &u_running,
            &w_running,
            &art.instance,
            &art.witness,
        )
        .map_err(BridgeError::NovaShapeRejected)?;
        shape
            .is_sat_relaxed(&ck, &u_new, &w_new)
            .unwrap_or_else(|e| panic!("step {step}: running pair UNSAT after fold: {e:?}"));
        u_running = u_new;
        w_running = w_new;
    }

    Ok((shape, ck, u_running, w_running))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1C INCREMENT 4a — multi-step fold accumulator composition.
    /// Bridge + fold N synthetic CF instances; the accumulator's
    /// running relaxed pair must satisfy `is_sat_relaxed` after
    /// every fold (per-step assertion lives inside
    /// `run_synthetic_cf_accumulator`). Final assertion: the
    /// returned final running pair is still satisfied — confirms
    /// the composition pattern is sound end-to-end across N
    /// successive folds, the foundation increment 4b's real primary
    /// augmented circuit will compose into.
    #[test]
    fn cf_accumulator_3_steps_running_pair_stays_satisfied() {
        let (shape, ck, u_final, w_final) =
            run_synthetic_cf_accumulator(3, b"ev-cf-ck")
                .expect("3-step accumulator must complete");
        shape
            .is_sat_relaxed(&ck, &u_final, &w_final)
            .expect("final accumulator pair must satisfy is_sat_relaxed");
    }

    /// Stress: 6 steps. Catches accumulator drift that a 3-step
    /// test might miss (relaxed `u` grows; cross-term commitments
    /// compose). Same soundness gate per step.
    #[test]
    fn cf_accumulator_6_steps_running_pair_stays_satisfied() {
        let (shape, ck, u_final, w_final) =
            run_synthetic_cf_accumulator(6, b"ev-cf-ck")
                .expect("6-step accumulator must complete");
        shape
            .is_sat_relaxed(&ck, &u_final, &w_final)
            .expect("final accumulator pair must satisfy is_sat_relaxed");
    }
}

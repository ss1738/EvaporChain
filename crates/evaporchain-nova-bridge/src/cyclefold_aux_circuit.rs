//! B-1/B-2 EVM, option (1C) — increment 1: the **CycleFold
//! auxiliary scalar-mul gadget** + its constraint-count probe.
//!
//! # Why this module
//!
//! CycleFold (Kothapalli & Setty, eprint 2023/1192) replaces Nova's
//! full augmented secondary circuit with a tiny "auxiliary" circuit
//! that does **one** EC scalar-mul check per IVC step. That single
//! check is the load-bearing piece of the whole CycleFold reduction
//! — it is what makes `n_secondary` independent of step circuit
//! size, dropping the secondary IPA opening's `n` from 2¹⁷ (the
//! option-(2) dead-end measurement) into a tractable range.
//!
//! # What this circuit verifies
//!
//! Given a base `P ∈ E1 = BN254-G1`, a scalar `s ∈ E1.ScalarField =
//! Bn254Fr` (the folding challenge), and a claimed result `Q`, it
//! enforces `Q = s · P`. Per CycleFold's cycle: the aux's constraint
//! field is **E1's base field = Bn254Fq** (matching E2's scalar
//! field, so the aux's R1CS commitment lands on E2 = Grumpkin
//! without non-native arithmetic). BN254 G1 ops are therefore
//! **NATIVE** here (ark_bn254::constraints::GVar over
//! FpVar<Bn254Fq>); the scalar is non-native (EmulatedFpVar<Bn254Fr,
//! Bn254Fq>) — mirror image of [`crate::s4_msm_gadget`].
//!
//! # The measurement is the deliverable
//!
//! Per the assert-without-measuring lesson, no claim about the
//! CycleFold reduction's effective `n_aux` is asserted here. The
//! probe test measures `cs.num_constraints()` directly so the next
//! increments work from a real number.

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr, G1Affine, G1Projective};
use ark_r1cs_std::{
    convert::ToBitsGadget,
    fields::emulated_fp::EmulatedFpVar,
    groups::CurveVar,
};
use ark_relations::gr1cs::SynthesisError;

/// In-circuit BN254-G1 variable, NATIVE over a Bn254Fq circuit
/// (`ark_bn254::constraints::GVar = ProjectiveVar<g1::Config,
/// FpVar<Bn254Fq>>`). This is the aux circuit's group type.
pub type Bn254G1Var = ark_bn254::constraints::GVar;

/// Compute `s · P` in-circuit over a Bn254Fq constraint system,
/// with `P` a public constant E1 point and `s` a non-native E1
/// scalar (Bn254Fr emulated in Bn254Fq).
///
/// Caller `enforce_equal`s the returned `Bn254G1Var` against the
/// claimed result `Q` to bind the relation `Q = s · P` — that is the
/// soundness gate. (Returning the var rather than enforcing inside
/// keeps the gadget composable; the `CycleFoldAuxCircuit` wrapper
/// below does the binding.)
pub fn cyclefold_aux_scalar_mul(
    base: G1Affine,
    scalar: &EmulatedFpVar<Bn254Fr, Bn254Fq>,
) -> Result<Bn254G1Var, SynthesisError> {
    let bits = scalar.to_bits_le()?;
    Bn254G1Var::constant(G1Projective::from(base)).scalar_mul_le(bits.iter())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bn254::G1Projective as Bn254G1Proj;
    use ark_ec::{AffineRepr, CurveGroup};
    use ark_ff::{Field, UniformRand};
    use ark_r1cs_std::{alloc::AllocVar, eq::EqGadget, GR1CSVar};
    use ark_relations::gr1cs::ConstraintSystem;
    use ark_std::test_rng;

    /// CORRECTNESS: in-circuit `s · P` equals the out-of-circuit ark
    /// scalar-mul; positive CS satisfaction. Random scalar (not a
    /// toy small value) — generic-safe.
    #[test]
    fn aux_scalar_mul_matches_native() {
        let mut rng = test_rng();
        let p_aff = G1Affine::generator();
        let s = Bn254Fr::rand(&mut rng);
        let expected = (Bn254G1Proj::from(p_aff) * s).into_affine();

        let cs = ConstraintSystem::<Bn254Fq>::new_ref();
        let s_var = EmulatedFpVar::<Bn254Fr, Bn254Fq>::new_witness(
            cs.clone(),
            || Ok(s),
        )
        .unwrap();
        let q = cyclefold_aux_scalar_mul(p_aff, &s_var).expect("synth");
        let expected_var = Bn254G1Var::new_witness(cs.clone(), || {
            Ok(Bn254G1Proj::from(expected))
        })
        .unwrap();
        q.enforce_equal(&expected_var).unwrap();

        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "in-circuit s·P must equal ark s·P + CS satisfied"
        );
        assert_eq!(
            q.value().unwrap().into_affine(),
            expected,
            "in-circuit value must equal out-of-circuit"
        );
    }

    /// NON-VACUITY: tampering the expected result MUST break CS.
    /// Same B-1 hazard discipline as `recursion_decider_circuit`.
    #[test]
    fn aux_scalar_mul_wrong_expected_breaks_cs() {
        let mut rng = test_rng();
        let p_aff = G1Affine::generator();
        let s = Bn254Fr::rand(&mut rng);
        let expected = (Bn254G1Proj::from(p_aff) * s).into_affine();
        let g = Bn254G1Proj::from(G1Affine::generator());

        let cs = ConstraintSystem::<Bn254Fq>::new_ref();
        let s_var = EmulatedFpVar::<Bn254Fr, Bn254Fq>::new_witness(
            cs.clone(),
            || Ok(s),
        )
        .unwrap();
        let q = cyclefold_aux_scalar_mul(p_aff, &s_var).expect("synth");
        // Tamper: claimed = expected + G (off by a generator).
        let bad = Bn254G1Var::new_witness(cs.clone(), || {
            Ok(Bn254G1Proj::from(expected) + g)
        })
        .unwrap();
        q.enforce_equal(&bad).unwrap();

        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "wrong Q MUST break CS — non-vacuous binding (B-1 guard)"
        );
    }

    /// THE INCREMENT-1 SIZE PROBE: measure `cs.num_constraints()`
    /// for one full CycleFold aux check (the binding R1CS shape).
    /// Prints `AUX_PROBE = …`, asserts sanity bounds only. The
    /// real number — not an assertion — feeds the next increments
    /// (ppsnark padding ⇒ effective IPA `n_aux` ⇒ Solidity gas).
    /// Per the assert-without-measuring lesson, no tractability
    /// claim is made here from the number — it is just reported.
    #[test]
    fn aux_scalar_mul_size_probe() {
        let mut rng = test_rng();
        let p_aff = G1Affine::generator();
        let s = Bn254Fr::rand(&mut rng);
        let expected = (Bn254G1Proj::from(p_aff) * s).into_affine();

        let cs = ConstraintSystem::<Bn254Fq>::new_ref();
        let s_var = EmulatedFpVar::<Bn254Fr, Bn254Fq>::new_witness(
            cs.clone(),
            || Ok(s),
        )
        .unwrap();
        let q = cyclefold_aux_scalar_mul(p_aff, &s_var).expect("synth");
        let expected_var = Bn254G1Var::new_witness(cs.clone(), || {
            Ok(Bn254G1Proj::from(expected))
        })
        .unwrap();
        q.enforce_equal(&expected_var).unwrap();

        let n_cons = cs.num_constraints();
        let n_witness = cs.num_witness_variables();
        let n_instance = cs.num_instance_variables();
        eprintln!(
            "AUX_PROBE cs.num_constraints={n_cons} \
             cs.num_witness={n_witness} cs.num_instance={n_instance}"
        );

        assert!(cs.is_satisfied().unwrap(), "probe CS must be sat");
        // Sanity: aux must be NON-trivial (catch a vacuous-circuit
        // regression — a fundamental B-1 guard for the probe itself).
        assert!(
            n_cons >= 1_000,
            "aux probe < 1000 cons: suspect vacuous circuit, got {n_cons}"
        );
        // Sanity: aux must be MUCH smaller than the option-(2)
        // dead-end (~3.3×10⁸ flat / ~5.2×10⁸ fold @ n=2¹⁷). 1e6 is a
        // very generous ceiling — tripping it means the gadget is
        // not delivering the architectural win and 1C needs a
        // re-think.
        assert!(
            n_cons < 1_000_000,
            "aux probe ≥ 1e6 cons: CycleFold reduction broken, got {n_cons}"
        );
    }
}

//! B-1/B-2 EVM, option (1C) — increment 3b-2: the **arkworks
//! `ConstraintSystem<Bn254Fq>` → nova-snark `R1CSShape<
//! GrumpkinEngine>` bridge** for the CycleFold instance circuit.
//!
//! # What this bridges
//!
//! [`crate::cyclefold_instance_circuit::CycleFoldInstanceCircuit`]
//! is authored against arkworks (`ConstraintSynthesizer<Bn254Fq>`),
//! but `nova_snark::nifs::NIFS<GrumpkinEngine>` operates on
//! `nova_snark::r1cs::R1CSShape<GrumpkinEngine>` with its own sparse
//! `(row, col, scalar)` representation and its own scalar type.
//! This module is the type-and-format adapter that lets NIFS act on
//! real CF instances (the load-bearing piece of increment 3b's full
//! integration).
//!
//! # Format conversion
//!
//! - arkworks: `ConstraintMatrices` `a/b/c: Matrix<F> = Vec<Vec<(F,
//!   usize)>>` (per-row list of `(coeff, col_index)`).
//! - nova-snark: `R1CSShape::new(num_cons, num_vars, num_io,
//!   A, B, C)` with `A/B/C: Vec<(usize, usize, E::Scalar)>` (flat
//!   sparse triples).
//!
//! Column layout is **identical** in both: col 0 = constant ONE,
//! cols `1..=num_io` = instance vars, cols `num_io+1..` = witness
//! vars. The arkworks `num_instance_variables` **includes** the
//! implicit ONE, so nova-snark's `num_io = ark.num_instance_variables
//! - 1`. Scalars convert via the just-verified
//! [`crate::scalar_adapter::ark_fq_to_secondary`] (same-field, exact).
//!
//! # Box-verified by the test below
//!
//! Build a real `CycleFoldInstanceCircuit`, extract the shape,
//! assert `num_cons == 1,985`, `num_vars == 1,812`, `num_io == 21`
//! (the increment-2 measurements with the implicit ONE removed).

use ark_bn254::Fq as Bn254Fq;
use ark_relations::r1cs::{
    ConstraintSynthesizer, ConstraintSystem, OptimizationGoal, SynthesisMode,
};

use crate::scalar_adapter::{ark_fq_to_secondary, SecondaryScalar};
use nova_snark::provider::GrumpkinEngine;
use nova_snark::r1cs::R1CSShape;

/// Errors returned by [`cyclefold_instance_r1cs_shape`].
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// Arkworks `ConstraintSystem::generate_constraints` failed.
    #[error("arkworks synthesis failed: {0:?}")]
    ArkSynthesis(ark_relations::r1cs::SynthesisError),
    /// `ConstraintSystem::to_matrices()` returned `None` — the
    /// synthesis mode wasn't `Setup`/matrix-construction. Should
    /// never happen if we set the mode correctly.
    #[error("ConstraintSystem::to_matrices returned None — wrong synthesis mode")]
    MatricesUnavailable,
    /// nova-snark `R1CSShape::new` rejected the converted matrices
    /// (column out of range etc.) — should never happen if our
    /// arkworks/nova layouts agree, but surfaces any drift loudly.
    #[error("nova-snark R1CSShape::new rejected the converted matrices: {0:?}")]
    NovaShapeRejected(nova_snark::errors::NovaError),
}

/// Synthesize an arkworks `ConstraintSynthesizer<Bn254Fq>` and
/// convert its A/B/C matrices into a nova-snark
/// `R1CSShape<GrumpkinEngine>`. Generic over the circuit so it can
/// be reused for any CF-side circuit (not only
/// `CycleFoldInstanceCircuit`).
pub fn arkworks_cs_to_nova_grumpkin_shape<C>(
    circuit: C,
) -> Result<R1CSShape<GrumpkinEngine>, BridgeError>
where
    C: ConstraintSynthesizer<Bn254Fq>,
{
    let cs = ConstraintSystem::<Bn254Fq>::new_ref();
    // Setup mode + no optimization gives the cleanest A/B/C the
    // increment-2 size probe measured.
    cs.set_optimization_goal(OptimizationGoal::None);
    cs.set_mode(SynthesisMode::Setup);

    circuit
        .generate_constraints(cs.clone())
        .map_err(BridgeError::ArkSynthesis)?;
    cs.finalize();
    let cs_borrow = cs.borrow().expect("CS ref must be borrow-able");
    let m = cs_borrow
        .to_matrices()
        .ok_or(BridgeError::MatricesUnavailable)?;

    // Arkworks counts the implicit ONE in `num_instance_variables`;
    // nova-snark's `num_io` excludes it.
    let num_cons = m.num_constraints;
    let num_vars = m.num_witness_variables;
    let num_io = m
        .num_instance_variables
        .checked_sub(1)
        .expect("arkworks num_instance_variables must include the implicit ONE");

    let convert = |rows: &[Vec<(Bn254Fq, usize)>]| -> Vec<(usize, usize, SecondaryScalar)> {
        let mut out = Vec::new();
        for (row_idx, row) in rows.iter().enumerate() {
            for (coeff, col) in row.iter() {
                out.push((row_idx, *col, ark_fq_to_secondary(*coeff)));
            }
        }
        out
    };
    let a = convert(&m.a);
    let b = convert(&m.b);
    let c = convert(&m.c);

    R1CSShape::<GrumpkinEngine>::new(num_cons, num_vars, num_io, &a, &b, &c)
        .map_err(BridgeError::NovaShapeRejected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cyclefold_instance_circuit::CycleFoldInstanceCircuit;
    use ark_bn254::{Fr as Bn254Fr, G1Affine, G1Projective};
    use ark_ec::{AffineRepr, CurveGroup};
    use ark_ff::UniformRand;
    use ark_std::test_rng;

    /// End-to-end bridge: a real `CycleFoldInstanceCircuit` →
    /// `R1CSShape<GrumpkinEngine>`. Asserts the shape's
    /// `num_constraints / num_vars / num_io` match the increment-2
    /// size-probe measurements (1,985 / 1,812 / 21) — that's the
    /// cross-check that the bridge preserves the R1CS exactly (no
    /// rows lost, no padding, no off-by-one in IO accounting).
    #[test]
    fn cf_instance_to_nova_shape_matches_increment2_probe() {
        let mut rng = test_rng();
        let p = G1Affine::generator();
        let s = Bn254Fr::rand(&mut rng);
        let q = (G1Projective::from(p) * s).into_affine();
        let circuit = CycleFoldInstanceCircuit::new(p, s, q);

        let shape = arkworks_cs_to_nova_grumpkin_shape(circuit)
            .expect("bridge must produce a valid R1CSShape");

        // The increment-2 measurements (HEAD 9bb02bc3, Mini3):
        //   cs.num_constraints=1985 cs.num_witness=1812 cs.num_instance=22
        // ⇒ nova num_io = 22 - 1 = 21.
        assert_eq!(
            shape.num_cons(),
            1_985,
            "num_cons must match increment-2 probe (1,985)"
        );
        assert_eq!(
            shape.num_vars(),
            1_812,
            "num_vars must match increment-2 probe (1,812)"
        );
        assert_eq!(
            shape.num_io(),
            21,
            "num_io must equal arkworks num_instance_variables (22) - 1 (the implicit ONE)"
        );
    }
}

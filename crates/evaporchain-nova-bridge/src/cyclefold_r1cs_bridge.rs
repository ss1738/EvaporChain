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
use ark_relations::gr1cs::{
    ConstraintSynthesizer, ConstraintSystem, OptimizationGoal, SynthesisMode,
};

use crate::scalar_adapter::{ark_fq_to_secondary, SecondaryScalar};
use nova_snark::provider::GrumpkinEngine;
use nova_snark::r1cs::{R1CSInstance, R1CSShape, R1CSWitness, SparseMatrix};
use nova_snark::traits::commitment::CommitmentEngineTrait;
use nova_snark::traits::Engine;

/// Errors returned by [`cyclefold_instance_r1cs_shape`].
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// Arkworks `ConstraintSystem::generate_constraints` failed.
    #[error("arkworks synthesis failed: {0:?}")]
    ArkSynthesis(ark_relations::gr1cs::SynthesisError),
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
    // ark-relations 0.6: to_matrices() returns Result<BTreeMap<Label,
    // Vec<Matrix>>>. For R1CS the predicate's matrices are [A, B, C].
    // Counts now come from the CS directly (were on R1CSMatrices in 0.5).
    let matrices_map = cs_borrow
        .to_matrices()
        .map_err(|_| BridgeError::MatricesUnavailable)?;
    let r1cs_mats = matrices_map
        .get(ark_relations::gr1cs::predicate::polynomial_constraint::R1CS_PREDICATE_LABEL)
        .ok_or(BridgeError::MatricesUnavailable)?;
    let m_a = &r1cs_mats[0];
    let m_b = &r1cs_mats[1];
    let m_c = &r1cs_mats[2];

    // Arkworks counts the implicit ONE in `num_instance_variables`;
    // nova-snark's `num_io` excludes it.
    let num_cons = cs_borrow.num_constraints();
    let num_vars = cs_borrow.num_witness_variables();
    let num_io = cs_borrow
        .num_instance_variables()
        .checked_sub(1)
        .expect("arkworks num_instance_variables must include the implicit ONE");

    // Column-layout remap: arkworks uses z = [ONE, X (num_io),
    // W (num_vars)] (col 0 = ONE, cols 1..=num_io = instance vars,
    // cols num_io+1.. = witness). nova-snark's `is_sat` builds
    // z = [W (num_vars), ONE, X (num_io)]. The shape's aggregate
    // dims (num_cons, num_vars, num_io) are layout-invariant, but
    // per-constraint column indices must be remapped or the R1CS
    // becomes nonsense (Az·Bz ≠ Cz for valid assignments — exactly
    // the increment-3b-3 first-run failure).
    let remap_col = |ark_col: usize| -> usize {
        if ark_col == 0 {
            num_vars // ONE → nova col num_vars
        } else if ark_col <= num_io {
            num_vars + ark_col // X[i-1] (ark) → nova col num_vars + ark_col
        } else {
            ark_col - num_io - 1 // W[k] (ark) → nova col k
        }
    };
    // SparseMatrix::new ASSERTS columns within each row are strictly
    // ascending; remap shuffles col positions so post-remap sort is
    // mandatory (the previous "sort the raw arkworks cols" sort no
    // longer suffices — sort the REMAPPED cols).
    let convert = |rows: &[Vec<(Bn254Fq, usize)>]| -> Vec<(usize, usize, SecondaryScalar)> {
        let mut out = Vec::new();
        for (row_idx, row) in rows.iter().enumerate() {
            let mut remapped: Vec<(usize, SecondaryScalar)> = row
                .iter()
                .map(|(coeff, ark_col)| (remap_col(*ark_col), ark_fq_to_secondary(*coeff)))
                .collect();
            remapped.sort_by_key(|(col, _)| *col);
            for (col, coeff) in remapped {
                out.push((row_idx, col, coeff));
            }
        }
        out
    };
    let a_triples = convert(m_a);
    let b_triples = convert(m_b);
    let c_triples = convert(m_c);

    // SparseMatrix's `cols` = total z-vector width = num_io + num_vars + 1
    // (the +1 is the implicit constant-ONE column at index 0).
    let cols = num_io + num_vars + 1;
    let a_sm = SparseMatrix::<SecondaryScalar>::new(&a_triples, num_cons, cols);
    let b_sm = SparseMatrix::<SecondaryScalar>::new(&b_triples, num_cons, cols);
    let c_sm = SparseMatrix::<SecondaryScalar>::new(&c_triples, num_cons, cols);

    R1CSShape::<GrumpkinEngine>::new(num_cons, num_vars, num_io, a_sm, b_sm, c_sm)
        .map_err(BridgeError::NovaShapeRejected)
}

/// CommitmentKey alias to keep call sites readable.
pub type CK =
    <<GrumpkinEngine as Engine>::CE as CommitmentEngineTrait<GrumpkinEngine>>::CommitmentKey;

/// Result of building a real, satisfied `R1CSInstance` + witness +
/// shape + commitment key for a CF-side arkworks circuit.
pub struct NovaGrumpkinR1CSArtifacts {
    pub shape: R1CSShape<GrumpkinEngine>,
    pub ck: CK,
    pub instance: R1CSInstance<GrumpkinEngine>,
    pub witness: R1CSWitness<GrumpkinEngine>,
}

/// Synthesize a CF-side arkworks `ConstraintSynthesizer<Bn254Fq>`
/// and build a **satisfied** nova-snark `(R1CSShape, R1CSInstance,
/// R1CSWitness)` triple over `GrumpkinEngine`, plus the
/// `CommitmentKey` used to commit the witness. Default synthesis
/// mode (`Prove { construct_matrices: true }`) so both matrices and
/// assignments are produced.
///
/// `ck_label` is the Pedersen `setup` domain tag (any stable
/// `&'static [u8]`; nova-snark uses `b"ck"` style tags). Reused
/// keys across folds must use the same label + a `num_vars ≥`
/// every folded instance's `num_vars`.
pub fn arkworks_cs_to_nova_grumpkin_satisfied_pair<C>(
    circuit: C,
    ck_label: &'static [u8],
) -> Result<NovaGrumpkinR1CSArtifacts, BridgeError>
where
    C: ConstraintSynthesizer<Bn254Fq>,
{
    let cs = ConstraintSystem::<Bn254Fq>::new_ref();
    cs.set_optimization_goal(OptimizationGoal::None);
    // Default mode = Prove { construct_matrices: true } — both
    // matrices AND assignments populated.

    circuit
        .generate_constraints(cs.clone())
        .map_err(BridgeError::ArkSynthesis)?;
    cs.finalize();
    let cs_borrow = cs.borrow().expect("CS ref must be borrow-able");
    // ark-relations 0.6 to_matrices: same shape as the helper above —
    // pull the R1CS predicate's [A, B, C], counts from cs directly.
    let matrices_map = cs_borrow
        .to_matrices()
        .map_err(|_| BridgeError::MatricesUnavailable)?;
    let r1cs_mats = matrices_map
        .get(ark_relations::gr1cs::predicate::polynomial_constraint::R1CS_PREDICATE_LABEL)
        .ok_or(BridgeError::MatricesUnavailable)?;
    let m_a = &r1cs_mats[0];
    let m_b = &r1cs_mats[1];
    let m_c = &r1cs_mats[2];

    let num_cons = cs_borrow.num_constraints();
    let num_vars = cs_borrow.num_witness_variables();
    let num_io = cs_borrow
        .num_instance_variables()
        .checked_sub(1)
        .expect("arkworks num_instance_variables must include the implicit ONE");

    // Same arkworks→nova column remap as in the shape-only path
    // (see comment in `arkworks_cs_to_nova_grumpkin_shape`).
    let remap_col = |ark_col: usize| -> usize {
        if ark_col == 0 {
            num_vars
        } else if ark_col <= num_io {
            num_vars + ark_col
        } else {
            ark_col - num_io - 1
        }
    };
    let convert = |rows: &[Vec<(Bn254Fq, usize)>]| -> Vec<(usize, usize, SecondaryScalar)> {
        let mut out = Vec::new();
        for (row_idx, row) in rows.iter().enumerate() {
            let mut remapped: Vec<(usize, SecondaryScalar)> = row
                .iter()
                .map(|(coeff, ark_col)| (remap_col(*ark_col), ark_fq_to_secondary(*coeff)))
                .collect();
            remapped.sort_by_key(|(col, _)| *col);
            for (col, coeff) in remapped {
                out.push((row_idx, col, coeff));
            }
        }
        out
    };
    let cols = num_io + num_vars + 1;
    let a_sm = SparseMatrix::<SecondaryScalar>::new(&convert(m_a), num_cons, cols);
    let b_sm = SparseMatrix::<SecondaryScalar>::new(&convert(m_b), num_cons, cols);
    let c_sm = SparseMatrix::<SecondaryScalar>::new(&convert(m_c), num_cons, cols);
    let shape = R1CSShape::<GrumpkinEngine>::new(num_cons, num_vars, num_io, a_sm, b_sm, c_sm)
        .map_err(BridgeError::NovaShapeRejected)?;

    // Extract assignments. arkworks puts the implicit ONE at
    // instance_assignment[0]; nova-snark's X excludes it.
    // ark-relations 0.6: these are methods returning Result<&[F]>.
    let w_nova: Vec<SecondaryScalar> = cs_borrow
        .witness_assignment()
        .map_err(|_| BridgeError::MatricesUnavailable)?
        .iter()
        .map(|f| ark_fq_to_secondary(*f))
        .collect();
    let x_nova: Vec<SecondaryScalar> = cs_borrow
        .instance_assignment()
        .map_err(|_| BridgeError::MatricesUnavailable)?
        .iter()
        .skip(1)
        .map(|f| ark_fq_to_secondary(*f))
        .collect();

    // Setup CK sized for the witness, commit, build instance.
    let ck = <<GrumpkinEngine as Engine>::CE as CommitmentEngineTrait<GrumpkinEngine>>::setup(
        ck_label, num_vars,
    )
    .map_err(BridgeError::NovaShapeRejected)?;
    let witness = R1CSWitness::<GrumpkinEngine>::new(&shape, &w_nova)
        .map_err(BridgeError::NovaShapeRejected)?;
    let comm_w = witness.commit(&ck);
    let instance = R1CSInstance::<GrumpkinEngine>::new(&shape, &comm_w, &x_nova)
        .map_err(BridgeError::NovaShapeRejected)?;

    Ok(NovaGrumpkinR1CSArtifacts {
        shape,
        ck,
        instance,
        witness,
    })
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

    /// 1C INCREMENT 3b-3 SOUNDNESS GATE: a real `CycleFoldInstance
    /// Circuit` synthesised through the full bridge produces a
    /// `(shape, R1CSInstance, R1CSWitness)` triple that nova-snark's
    /// own `R1CSShape::is_sat` accepts. This proves the end-to-end
    /// arkworks→nova-snark pipeline (shape + assignments + Pedersen
    /// commitment) is consistent; any bug in scalar conversion,
    /// matrix sort, IO accounting, or witness/instance construction
    /// breaks this. Required precondition for 3b-4's NIFS prove.
    #[test]
    fn cf_instance_through_bridge_is_satisfied_per_nova_is_sat() {
        let mut rng = test_rng();
        let p = G1Affine::generator();
        let s = Bn254Fr::rand(&mut rng);
        let q = (G1Projective::from(p) * s).into_affine();
        let circuit = CycleFoldInstanceCircuit::new(p, s, q);

        let art = arkworks_cs_to_nova_grumpkin_satisfied_pair(circuit, b"ev-cf-ck")
            .expect("bridge must produce a satisfied artifacts triple");

        // Sanity: artifact dims match shape.
        assert_eq!(art.shape.num_cons(), 1_985);
        assert_eq!(art.shape.num_vars(), 1_812);
        assert_eq!(art.shape.num_io(), 21);

        // THE GATE: nova-snark accepts this as a satisfied R1CS pair.
        art.shape
            .is_sat(&art.ck, &art.instance, &art.witness)
            .expect("nova-snark R1CSShape::is_sat must accept bridged CF instance");
    }

    /// 1C INCREMENT 3b-4 SOUNDNESS GATE: real
    /// `NIFS::<GrumpkinEngine>::prove` on TWO bridged CF instances
    /// produces a satisfied folded relaxed pair. Plus
    /// `NIFS::verify` ≡ `NIFS::prove` U cross-check (any mismatch
    /// ⇒ bug in our composition vs nova-snark NIFS semantics).
    ///
    /// `CE::setup(label, n)` is deterministic in `(label, n)` —
    /// two artifacts built with the same label + same shape's
    /// num_vars share an identical `ck`, so passing `art1.ck` for
    /// both prove (commitments) and verify (computations) is sound.
    #[test]
    fn nifs_prove_two_real_cf_instances_yields_satisfied_folded_pair() {
        use nova_snark::nova::nifs::NIFS;
        use nova_snark::r1cs::{RelaxedR1CSInstance, RelaxedR1CSWitness};
        use nova_snark::traits::ROConstants;

        let mut rng = test_rng();
        let make = |rng: &mut _| {
            let p = G1Affine::generator();
            let s = Bn254Fr::rand(rng);
            let q = (G1Projective::from(p) * s).into_affine();
            CycleFoldInstanceCircuit::new(p, s, q)
        };
        let art1 = arkworks_cs_to_nova_grumpkin_satisfied_pair(make(&mut rng), b"ev-cf-ck")
            .expect("art1 bridge");
        let art2 = arkworks_cs_to_nova_grumpkin_satisfied_pair(make(&mut rng), b"ev-cf-ck")
            .expect("art2 bridge");

        // Lift art1 → relaxed (running side).
        let u_running = RelaxedR1CSInstance::<GrumpkinEngine>::from_r1cs_instance(
            &art1.ck,
            &art1.shape,
            &art1.instance,
        );
        let w_running =
            RelaxedR1CSWitness::<GrumpkinEngine>::from_r1cs_witness(&art1.shape, &art1.witness);

        let ro_consts = ROConstants::<GrumpkinEngine>::default();
        let pp_digest = SecondaryScalar::from(0u64);

        let (nifs_proof, (u_folded, w_folded)) = NIFS::<GrumpkinEngine>::prove(
            &art1.ck,
            &ro_consts,
            &pp_digest,
            &art1.shape,
            &u_running,
            &w_running,
            &art2.instance,
            &art2.witness,
        )
        .expect("NIFS::prove must succeed on two bridged CF instances");

        // THE SOUNDNESS GATE: folded relaxed pair satisfies shape.
        art1.shape
            .is_sat_relaxed(&art1.ck, &u_folded, &w_folded)
            .expect("NIFS::prove output must be is_sat_relaxed-accepted");

        // Cross-check: prove's U ≡ verify's U (semantic agreement
        // of the prover and verifier paths of NIFS).
        let u_via_verify = nifs_proof
            .verify(&ro_consts, &pp_digest, &u_running, &art2.instance)
            .expect("NIFS::verify must succeed");
        assert_eq!(
            u_folded, u_via_verify,
            "NIFS::prove and NIFS::verify must produce the same folded U"
        );
    }
}

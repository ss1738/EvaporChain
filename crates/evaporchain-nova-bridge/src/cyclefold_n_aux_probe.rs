//! B-1/B-2 EVM, option (1C) — increment 5-α: **direct n_aux
//! measurement** for the CycleFold secondary.
//!
//! # What this pins
//!
//! Across the 1C arc the CF secondary's IPA opening size `n_aux`
//! has been *predicted*: increment 1 hinted ~8,192 from `S_comm.N
//! = next_pow2(2·num_vars) = next_pow2(2·1812)`; increment 2
//! refined to "≥ 2¹² = 4,096 (caveat `total_nz` may push higher)".
//! Per the assert-without-measuring lesson, predictions stand only
//! until they are *measured against a real proof*. This module
//! does that: build a satisfied `RelaxedR1CSInstance/Witness` for
//! [`crate::cyclefold_instance_circuit::CycleFoldInstanceCircuit`],
//! run `ppsnark::setup` + `prove`, serde-extract the resulting
//! proof, read `eval_arg.L_vec.len()` — that array length is
//! `log₂(n_aux)`, so `n_aux = 1 << L_vec.len()`.
//!
//! # What this is NOT
//!
//! Not a measure of the *full* IVC's secondary cost (the full IVC
//! needs the complete 4b primary augmented circuit, deferred). The
//! n_aux here is the secondary IPA opening size for a single CF
//! instance, which IS the dominant cost driver of the per-step CF
//! fold. So this number IS the load-bearing one for Solidity gas
//! prediction (increment 6) and Groth16 wrapper viability.

use ark_bn254::Fq as Bn254Fq;
use ark_relations::gr1cs::{
    ConstraintSynthesizer, ConstraintSystem, OptimizationGoal,
};

use crate::cyclefold_r1cs_bridge::BridgeError;
use crate::scalar_adapter::{ark_fq_to_secondary, SecondaryScalar};

use nova_snark::provider::GrumpkinEngine;
use nova_snark::r1cs::{
    R1CSInstance, R1CSShape, R1CSWitness, RelaxedR1CSInstance, RelaxedR1CSWitness,
    SparseMatrix,
};
use nova_snark::traits::commitment::CommitmentEngineTrait;
use nova_snark::traits::snark::RelaxedR1CSSNARKTrait;
use nova_snark::traits::Engine;

/// The ppsnark type used for the CF secondary measurement —
/// matches `recursive_snark_fixture::S2pp` (`spartan::ppsnark::
/// RelaxedR1CSSNARK<GrumpkinEngine, ipa_pc::EvaluationEngine>`).
pub type S2pp = nova_snark::spartan::ppsnark::RelaxedR1CSSNARK<
    GrumpkinEngine,
    nova_snark::provider::ipa_pc::EvaluationEngine<GrumpkinEngine>,
>;

/// Result of the n_aux measurement.
#[derive(Debug, Clone, Copy)]
pub struct NauxMeasurement {
    /// `eval_arg.L_vec.len()` from the serialised ppsnark proof.
    pub log_n_aux: usize,
    /// `1 << log_n_aux` — the actual IPA opening size for the CF
    /// secondary on this circuit.
    pub n_aux: usize,
    /// `shape.num_cons` — same R1CS the bridge measured at 1985 for
    /// `CycleFoldInstanceCircuit`.
    pub shape_num_cons: usize,
    /// `shape.num_vars`.
    pub shape_num_vars: usize,
}

/// Run a real ppsnark proof on a CF-side arkworks circuit and
/// report the secondary IPA `n_aux`. Heavy (ppsnark preprocessing
/// + Spartan prove); intended to be called from a `#[ignore]` test
/// on the box, not in the default CI loop.
pub fn measure_cf_secondary_n_aux<C: ConstraintSynthesizer<Bn254Fq>>(
    circuit: C,
    ck_label: &'static [u8],
) -> Result<NauxMeasurement, BridgeError> {
    // 1. Synthesise + extract matrices + assignments (inline mirror
    //    of `arkworks_cs_to_nova_grumpkin_satisfied_pair`, but we
    //    can't reuse that one because we need to size the CK to
    //    ppsnark's ck_floor, not just num_vars).
    let cs = ConstraintSystem::<Bn254Fq>::new_ref();
    cs.set_optimization_goal(OptimizationGoal::None);
    circuit
        .generate_constraints(cs.clone())
        .map_err(BridgeError::ArkSynthesis)?;
    cs.finalize();
    let cs_borrow = cs.borrow().expect("CS ref must be borrow-able");
    // ark-relations 0.6 to_matrices: Result<BTreeMap<Label, Vec<Matrix>>>.
    // R1CS predicate matrices = [A, B, C]; counts come from cs directly.
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
        .expect("arkworks num_instance_variables includes the implicit ONE");

    // Same column remap as the 3b-3 bridge (arkworks [ONE, X, W] →
    // nova [W, ONE, X]).
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
                .map(|(coeff, ark_col)| {
                    (remap_col(*ark_col), ark_fq_to_secondary(*coeff))
                })
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

    // ark-relations 0.6: witness_assignment / instance_assignment are
    // now methods returning Result<&[F]> (were public Vec fields).
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

    // 2. ppsnark requires a CK sized to its `ck_floor(&shape)`
    //    (the preprocessed sparse-matrix commitments need room).
    //    next_pow2(max(total_nz, 2·num_vars, num_cons)).
    let ck_floor_closure = S2pp::ck_floor();
    let ck_size = ck_floor_closure(&shape);
    let ck = <<GrumpkinEngine as Engine>::CE as CommitmentEngineTrait<
        GrumpkinEngine,
    >>::setup(ck_label, ck_size)
        .map_err(BridgeError::NovaShapeRejected)?;

    // 3. Build witness + commit (matching r_W internal) + instance.
    let witness = R1CSWitness::<GrumpkinEngine>::new(&shape, &w_nova)
        .map_err(BridgeError::NovaShapeRejected)?;
    let comm_w = witness.commit(&ck);
    let instance = R1CSInstance::<GrumpkinEngine>::new(&shape, &comm_w, &x_nova)
        .map_err(BridgeError::NovaShapeRejected)?;

    // 4. Sanity: basic R1CS is satisfied (cheap; catches a CK-size
    //    mismatch where the new larger CK gives a different
    //    commitment than the witness expects).
    shape
        .is_sat(&ck, &instance, &witness)
        .map_err(BridgeError::NovaShapeRejected)?;

    // 5. Lift to relaxed for ppsnark.
    let u_relaxed = RelaxedR1CSInstance::<GrumpkinEngine>::from_r1cs_instance(
        &ck, &shape, &instance,
    );
    let w_relaxed = RelaxedR1CSWitness::<GrumpkinEngine>::from_r1cs_witness(
        &shape, &witness,
    );
    shape
        .is_sat_relaxed(&ck, &u_relaxed, &w_relaxed)
        .map_err(BridgeError::NovaShapeRejected)?;

    // 6. ppsnark setup + prove.
    let (pk, _vk) = S2pp::setup(&ck, &shape).map_err(BridgeError::NovaShapeRejected)?;
    let proof = S2pp::prove(&ck, &pk, &shape, &u_relaxed, &w_relaxed)
        .map_err(BridgeError::NovaShapeRejected)?;

    // 7. Serde-extract eval_arg.L_vec.len() from the proof. This IS
    //    log₂(n_aux); n_aux = 1 << log_n_aux.
    let v = serde_json::to_value(&proof).expect("ppsnark proof to_value");
    let l_vec = v
        .pointer("/eval_arg/L_vec")
        .and_then(|x| x.as_array())
        .expect("proof.eval_arg.L_vec must be an array");
    let log_n_aux = l_vec.len();
    let n_aux = 1usize << log_n_aux;

    Ok(NauxMeasurement {
        log_n_aux,
        n_aux,
        shape_num_cons: num_cons,
        shape_num_vars: num_vars,
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

    /// 1C INCREMENT 5-α MEASUREMENT: run ppsnark on a real
    /// `CycleFoldInstanceCircuit` and pin the actual `n_aux`. Prints
    /// `N_AUX_MEASURED ...` so the flow can read the verdict;
    /// asserts a sanity ceiling well below the option-2 dead-end
    /// 2¹⁷ = 131,072 — if the measured n_aux IS ≥ 2¹⁷, the
    /// CycleFold reduction failed in absolute numbers and option
    /// 1C is itself a dead-end.
    /// `#[ignore]`: heavy (ppsnark preprocess + prove, ~tens of
    /// seconds on a Mini); intended for manual / opt-in box runs.
    #[test]
    #[ignore = "increment-5-α n_aux measurement: ppsnark prove on real CF instance (Mini, slow)"]
    fn cf_secondary_n_aux_measurement_real_proof() {
        let mut rng = test_rng();
        let p = G1Affine::generator();
        let s = Bn254Fr::rand(&mut rng);
        let q = (G1Projective::from(p) * s).into_affine();
        let circuit = CycleFoldInstanceCircuit::new(p, s, q);

        let m = measure_cf_secondary_n_aux(circuit, b"ev-cf-naux")
            .expect("ppsnark measurement must complete");
        eprintln!(
            "N_AUX_MEASURED log_n_aux={} n_aux={} shape_num_cons={} shape_num_vars={}",
            m.log_n_aux, m.n_aux, m.shape_num_cons, m.shape_num_vars
        );

        // Sanity: dimensions match the 3b-2 / increment-2 probe.
        assert_eq!(m.shape_num_cons, 1_985, "shape num_cons regression");
        assert_eq!(m.shape_num_vars, 1_812, "shape num_vars regression");

        // FALSIFIER: option-2 dead-end was n_aux = 2¹⁷ = 131_072
        // (for the FULL augmented-circuit secondary). CycleFold's
        // architectural reduction here MUST produce a meaningfully
        // smaller n_aux — < 2¹⁷ is the necessary condition for the
        // 1C architecture to be a real improvement. (Predicted ≥
        // 2¹² = 4,096; real value to be reported.)
        assert!(
            m.log_n_aux < 17,
            "FALSIFIER TRIPPED: log_n_aux={} ≥ 17 — 1C architecture \
             does NOT reduce secondary below the option-2 dead-end. \
             Re-think required.",
            m.log_n_aux
        );
    }
}

//! Audit B-1/B-2 **S4b**: in-circuit satisfiability of the
//! **secondary** RelaxedR1CS — the other half of full Nova
//! soundness (Section 3 does only the primary, native-field).
//!
//! Nova's `is_sat_relaxed` requires BOTH accumulators satisfied. The
//! secondary R1CS is over Grumpkin's scalar field = BN254 **Fq**,
//! non-native to the BN254-Fr circuit. This gadget is the
//! non-native mirror of `section3_gadget::enforce_primary_relaxed_
//! r1cs_sat`: for every row `i`, enforce
//!   `(Az)_i · (Bz)_i == u · (Cz)_i + E_i`
//! with `z = [W, u, X[0], X[1]]`, **every operation in
//! `EmulatedFpVar<Fq, Fr>`**.
//!
//! This module proves the gadget logic in isolation on a tiny
//! hand-built instance (satisfied passes; an unsatisfied row makes
//! the CS unsatisfiable). Extracting the real secondary A/B/C/W/E
//! from a fixture is the next S4b sub-unit (a `section3_witness`-
//! class extraction, secondary side). Cost note: non-native makes
//! every `coeff·z` a real emulated mul (unlike native, where a
//! constant coeff is a free LC) — the documented S4b constraint
//! blow-up; the isolated proof stays tiny on purpose.

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr};
use ark_r1cs_std::{
    eq::EqGadget,
    fields::{emulated_fp::EmulatedFpVar, FieldVar},
};
use ark_relations::r1cs::SynthesisError;

/// Non-native scalar var over the secondary R1CS field (BN254 Fq),
/// emulated inside the BN254-Fr circuit.
pub type NnFq = EmulatedFpVar<Bn254Fq, Bn254Fr>;

/// One sparse row as `(col, coeff)` pairs (coeff is a constant
/// secondary-field element).
pub type SparseRow = Vec<(usize, Bn254Fq)>;

/// Σ over the row of `coeff · z[col]`, all emulated. Constant coeffs
/// are NOT free here (non-native) — each is an emulated multiply.
fn sparse_lc_nn(rows: &[SparseRow], z: &[NnFq], row: usize) -> Result<NnFq, SynthesisError> {
    let mut acc = NnFq::constant(Bn254Fq::from(0u64));
    if let Some(entries) = rows.get(row) {
        for &(col, coeff) in entries {
            let term = &z[col] * NnFq::constant(coeff);
            acc = &acc + &term;
        }
    }
    Ok(acc)
}

/// Enforce secondary RelaxedR1CS satisfiability in-circuit (S4b core).
///
/// `a/b/c_rows` are pre-bucketed by row (same shape as
/// `section3_gadget::bucket_by_row`, secondary field). `x` are the
/// two public-IO scalars. All inputs are emulated `Fq` vars.
#[allow(clippy::too_many_arguments)]
pub fn enforce_secondary_relaxed_r1cs_sat_nn(
    w: &[NnFq],
    e: &[NnFq],
    u: &NnFq,
    x: &[NnFq],
    a_rows: &[SparseRow],
    b_rows: &[SparseRow],
    c_rows: &[SparseRow],
    num_cons: usize,
) -> Result<(), SynthesisError> {
    // z = [W, u, X[0], X[1]]
    let mut z: Vec<NnFq> = Vec::with_capacity(w.len() + 1 + x.len());
    z.extend(w.iter().cloned());
    z.push(u.clone());
    z.extend(x.iter().cloned());

    for row in 0..num_cons {
        let az = sparse_lc_nn(a_rows, &z, row)?;
        let bz = sparse_lc_nn(b_rows, &z, row)?;
        let cz = sparse_lc_nn(c_rows, &z, row)?;
        let lhs = &az * &bz;
        let rhs = &(u * &cz) + &e[row];
        lhs.enforce_equal(&rhs)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_r1cs_std::alloc::AllocVar;
    use ark_relations::r1cs::ConstraintSystem;

    /// Tiny hand-built secondary instance: num_vars=1, x=[0,0],
    /// z=[w0,u,0,0]. One row: A=[(0,1)]→Az=w0; B=[(0,1)]→Bz=w0;
    /// C=[(0,3)]→Cz=3·w0; u=1, E=0. Satisfied iff w0² == 3·w0.
    /// Returns whether the constraint system is satisfied.
    fn build_is_satisfied(w0: u64) -> bool {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let mk = |v: u64| NnFq::new_witness(cs.clone(), || Ok(Bn254Fq::from(v))).unwrap();
        let w = vec![mk(w0)];
        let e = vec![mk(0)];
        let u = mk(1);
        let x = vec![mk(0), mk(0)];
        let a = vec![vec![(0usize, Bn254Fq::from(1u64))]];
        let b = vec![vec![(0usize, Bn254Fq::from(1u64))]];
        let c = vec![vec![(0usize, Bn254Fq::from(3u64))]];
        enforce_secondary_relaxed_r1cs_sat_nn(&w, &e, &u, &x, &a, &b, &c, 1)
            .expect("synthesize secondary R1CS gadget");
        cs.is_satisfied().expect("is_satisfied")
    }

    /// THE S4b PRIMITIVE PROOF: a correct secondary R1CS witness
    /// satisfies the CS; an incorrect one makes it unsatisfiable.
    #[test]
    fn secondary_relaxed_r1cs_nn_sat_and_adversarial() {
        // Satisfied: w0 = 3 → 3·3 == 1·9 + 0  (9 == 9).
        assert!(
            build_is_satisfied(3),
            "correct secondary R1CS witness must satisfy the CS"
        );
        // Adversarial: w0 = 4 → Az·Bz = 16, u·Cz+E = 12 → 16 != 12.
        assert!(
            !build_is_satisfied(4),
            "an unsatisfied secondary R1CS row must make the CS UNSATISFIABLE"
        );
    }
}

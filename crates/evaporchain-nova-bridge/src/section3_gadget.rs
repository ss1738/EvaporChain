//! In-circuit enforcement of the primary RelaxedR1CS satisfiability check.
//!
//! `enforce_primary_relaxed_r1cs_sat` synthesises the Section 3 gate:
//! for each row `i` of the primary R1CS, enforce
//!   `(Az)_i * (Bz)_i == u * (Cz)_i + E_i`
//! where `z = [W, u, X[0], X[1]]` and A, B, C are CIRCUIT CONSTANTS
//! (from `PublicParams`).
//!
//! # Constraint cost
//!
//! - `num_cons` multiplication gates (one per row: `Az_i * Bz_i`).
//! - Linear combinations for `Az_i`, `Bz_i`, `Cz_i` are free in R1CS.
//! - Total witness scalars: `num_vars` (W) + `num_cons` (E) + 1 (u) + 2 (X).
//!
//! For `TrivialIncrementCircuit`'s augmented primary:
//! `num_cons ≈ 10 003`, `num_vars ≈ 9 995` → ~10 k mult gates.

use crate::section3_witness::{Section3Witness, SparseTriple};
use ark_bn254::Fr as Bn254Fr;
use ark_r1cs_std::{
    alloc::AllocVar,
    eq::EqGadget,
    fields::fp::FpVar,
};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

/// Enforce primary RelaxedR1CS satisfiability in-circuit.
///
/// Allocates `W`, `E`, `u` as private witnesses; uses `X` passed in as
/// already-allocated public input vars.  The A, B, C matrices from `s3`
/// are baked in as circuit constants (constant-coefficient linear
/// combinations, zero extra mult gates).
///
/// Returns `Ok(())` on success; `SynthesisError` if synthesis fails.
pub fn enforce_primary_relaxed_r1cs_sat(
    cs: ConstraintSystemRef<Bn254Fr>,
    s3: &Section3Witness,
    x_vars: &[FpVar<Bn254Fr>],
) -> Result<(), SynthesisError> {
    // ── Allocate witnesses ────────────────────────────────────────────────
    let w_vars: Vec<FpVar<Bn254Fr>> = s3
        .w_primary
        .iter()
        .map(|&val| FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(val)))
        .collect::<Result<_, _>>()?;

    let e_vars: Vec<FpVar<Bn254Fr>> = s3
        .e_primary
        .iter()
        .map(|&val| FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(val)))
        .collect::<Result<_, _>>()?;

    let u_var = FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(s3.u_primary))?;

    // ── Build z = [W, u, X[0], X[1]] ─────────────────────────────────────
    let mut z_vars: Vec<FpVar<Bn254Fr>> = Vec::with_capacity(w_vars.len() + 1 + x_vars.len());
    z_vars.extend(w_vars.iter().cloned());
    z_vars.push(u_var.clone());
    for xv in x_vars {
        z_vars.push(xv.clone());
    }

    // ── For each row: enforce (Az)_i * (Bz)_i == u * (Cz)_i + E_i ───────
    #[allow(clippy::needless_range_loop)]
    for row in 0..s3.num_cons {
        let az_i = sparse_lc(&s3.a_primary, &z_vars, row)?;
        let bz_i = sparse_lc(&s3.b_primary, &z_vars, row)?;
        let cz_i = sparse_lc(&s3.c_primary, &z_vars, row)?;

        let lhs = &az_i * &bz_i;
        let rhs = &u_var * &cz_i + &e_vars[row];
        lhs.enforce_equal(&rhs)?;
    }

    Ok(())
}

/// Compute one row of a sparse matrix-vector product as a `FpVar` linear
/// combination.  Constant coefficients → no extra mult gates.
fn sparse_lc(
    m: &SparseTriple,
    z: &[FpVar<Bn254Fr>],
    row: usize,
) -> Result<FpVar<Bn254Fr>, SynthesisError> {
    let mut acc = FpVar::<Bn254Fr>::Constant(Bn254Fr::from(0u64));
    for &(r, col, ref coeff) in &m.entries {
        if r == row {
            acc += &z[col] * *coeff;
        }
    }
    Ok(acc)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::section3_witness::Section3Witness;
    use ark_relations::r1cs::ConstraintSystem;

    /// A trivially-satisfied circuit: zero A/B/C, zero W/E, u=1.
    /// All rows reduce to 0*0 == 1*0 + 0, which is 0 == 0. CS must satisfy.
    #[test]
    fn trivial_zero_witness_circuit_satisfies() {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let s3 = Section3Witness::trivial_zero(3, 5);

        // Allocate x_vars (2 zero public inputs)
        let x_vars: Vec<FpVar<Bn254Fr>> = s3
            .x_primary
            .iter()
            .map(|&x| {
                FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(x))
            })
            .collect::<Result<_, _>>()
            .expect("alloc x_vars");

        enforce_primary_relaxed_r1cs_sat(cs.clone(), &s3, &x_vars)
            .expect("Section 3 gadget must synthesize");
        assert!(
            cs.is_satisfied().expect("is_satisfied check"),
            "trivial zero witness must produce a satisfied CS"
        );
    }

    /// Single-constraint R1CS: a*b = c, one non-zero entry each.
    /// Witness: W=[a, b], u=1, X=[0,0], E=[0].
    /// Constraint row: Az = a, Bz = b, Cz = c where c = a*b.
    #[test]
    fn single_constraint_satisfied() {
        use crate::section3_witness::SparseTriple;
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();

        let a_val = Bn254Fr::from(3u64);
        let b_val = Bn254Fr::from(5u64);
        let c_val = a_val * b_val; // 15

        // z = [W[0]=a, W[1]=b, u=1, X[0]=0, X[1]=0]  (len 5)
        // Az[0] = 1*z[0] = a
        // Bz[0] = 1*z[1] = b
        // Cz[0] = 1*z[2_unused] -> let's put c in E instead
        // Simpler: use C -> z[3] = X[0] = 0 and E[0] = a*b
        let num_cols = 5;
        let s3 = Section3Witness {
            w_primary: vec![a_val, b_val],
            e_primary: vec![c_val], // E[0] = a*b so that lhs==rhs: a*b == 0*Cz + E[0]
            u_primary: Bn254Fr::from(1u64),
            x_primary: [Bn254Fr::from(0u64); 2],
            a_primary: SparseTriple {
                num_rows: 1,
                num_cols,
                entries: vec![(0, 0, Bn254Fr::from(1u64))], // row0 col0: +1
            },
            b_primary: SparseTriple {
                num_rows: 1,
                num_cols,
                entries: vec![(0, 1, Bn254Fr::from(1u64))], // row0 col1: +1
            },
            c_primary: SparseTriple {
                num_rows: 1,
                num_cols,
                entries: vec![], // Cz = 0
            },
            num_cons: 1,
            num_vars: 2,
            num_io: 2,
        };

        // Verify natively first
        s3.validate_rows_native().expect("native check must pass");

        let x_vars: Vec<FpVar<Bn254Fr>> = s3
            .x_primary
            .iter()
            .map(|&x| FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(x)))
            .collect::<Result<_, _>>()
            .expect("alloc x");

        enforce_primary_relaxed_r1cs_sat(cs.clone(), &s3, &x_vars)
            .expect("Section 3 gadget synthesize");
        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "single-constraint R1CS must satisfy"
        );
    }
}

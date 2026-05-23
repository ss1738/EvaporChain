//! B-1/B-2 EVM, option (1C) — increment (c)-2d: **n_aux scaling
//! probe** with synthetic small R1CS shapes.
//!
//! # Purpose
//!
//! Tests the (R5) "smaller n_aux via deeper recursion" feasibility
//! claim from `B1_B2_AUDIT_DOSSIER.md`. The dossier projects
//! reducing n_aux from 16,384 → ~1,024 (16×) to bring the BN254-
//! precompile Pippenger MSM gas from ~319M into L2 single-tx range
//! (~20M).
//!
//! The PHYSICAL question (R5) hinges on: does `n_aux` actually
//! scale linearly with secondary R1CS `total_nz`, or is there a
//! hidden ppsnark floor that blocks reduction below some threshold?
//!
//! Per the existing measurement (`cyclefold_n_aux_probe`, HEAD
//! `da3735a1`): n_aux = next_pow2(max(total_nz, 2·num_vars,
//! num_cons)). At the real CycleFold instance (1,985 cons,
//! total_nz ≈ 15,880) → n_aux = 2¹⁴ = 16,384.
//!
//! This probe runs ppsnark on synthetic R1CS shapes of decreasing
//! size and reports the observed n_aux. Falsifier: if the smallest
//! shape (say 8 cons, tens of nonzeros) still has n_aux ≥ 4,096,
//! ppsnark has a hidden setup-side floor and (R5) cannot drop n_aux
//! by 16× via constraint reduction alone.
//!
//! ## What this does NOT prove
//!
//! Even if scaling holds: shrinking the *actual* CycleFold
//! secondary from 1,985 cons → 125 cons requires per-bit folding
//! granularity (currently 254-bit scalar-mul in one shot). That's
//! a multi-week circuit-architecture rewrite + a ~254× prover-side
//! work multiplier. This probe only tests the n_aux side of the
//! claim, not the architectural-cost side.

use ark_bn254::Fq as Bn254Fq;
use ark_ff::Field;
use ark_relations::{
    lc,
    gr1cs::{
        ConstraintSynthesizer, ConstraintSystemRef, LinearCombination,
        SynthesisError, Variable,
    },
};

use crate::cyclefold_n_aux_probe::measure_cf_secondary_n_aux;
use crate::cyclefold_r1cs_bridge::BridgeError;

/// A trivially-satisfied parametric-size R1CS:
///   - allocates `chain_len` witness variables `w_0, ..., w_{n-1}`
///   - enforces `w_{i+1} = w_i · scalar` (one multiplication per step)
///   - public input is the final value `w_{n-1}` (claim of the chain)
///
/// This shape is intentionally simple so we can dial `chain_len` and
/// observe how n_aux scales with the resulting R1CS dimensions.
#[derive(Clone, Debug)]
pub struct TrivialChainCircuit {
    pub chain_len: usize,
    /// Initial witness value (input to the chain).
    pub w0: Bn254Fq,
    /// Multiplicative scalar used at each step.
    pub scalar: Bn254Fq,
}

impl TrivialChainCircuit {
    pub fn new(chain_len: usize, w0: Bn254Fq, scalar: Bn254Fq) -> Self {
        Self { chain_len, w0, scalar }
    }
}

impl ConstraintSynthesizer<Bn254Fq> for TrivialChainCircuit {
    fn generate_constraints(
        self,
        cs: ConstraintSystemRef<Bn254Fq>,
    ) -> Result<(), SynthesisError> {
        let n = self.chain_len;
        if n == 0 {
            return Ok(());
        }
        // Allocate witness chain.
        let mut cur_val = self.w0;
        let mut cur_var: Variable = cs.new_witness_variable(|| Ok(cur_val))?;
        let scalar_var = cs.new_witness_variable(|| Ok(self.scalar))?;

        for _i in 0..(n - 1) {
            let next_val = cur_val * self.scalar;
            let next_var = cs.new_witness_variable(|| Ok(next_val))?;
            // Constraint: cur · scalar = next.
            // ark-relations 0.6: enforce_constraint took 3 LCs in 0.5;
            // in 0.6 it's predicate-label + boxed-closure-iter. For
            // plain R1CS use enforce_r1cs_constraint(a, b, c) — takes
            // 3 LC-producing closures, same semantics.
            cs.enforce_r1cs_constraint(
                || LinearCombination::from(cur_var),
                || LinearCombination::from(scalar_var),
                || LinearCombination::from(next_var),
            )?;
            cur_val = next_val;
            cur_var = next_var;
        }

        // Public-input pin: the final value (claim).
        let claim_var = cs.new_input_variable(|| Ok(cur_val))?;
        cs.enforce_r1cs_constraint(
            || LinearCombination::from(cur_var),
            || lc!() + (Bn254Fq::ONE, Variable::One),
            || LinearCombination::from(claim_var),
        )?;

        Ok(())
    }
}

/// Convenience: build the synthetic circuit + run the ppsnark probe.
pub fn measure_synthetic_n_aux(
    chain_len: usize,
    ck_label: &'static [u8],
) -> Result<crate::cyclefold_n_aux_probe::NauxMeasurement, BridgeError> {
    let circuit = TrivialChainCircuit::new(
        chain_len,
        Bn254Fq::from(7u64),
        Bn254Fq::from(3u64),
    );
    measure_cf_secondary_n_aux(circuit, ck_label)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (c)-2d FALSIFIER: scan synthetic shapes of decreasing size and
    /// report the observed n_aux at each. If n_aux drops linearly
    /// with chain length, (R5) scaling claim is feasible (the
    /// architectural cost of shrinking the secondary stays separate).
    /// If n_aux floors at some k regardless of small shape, ppsnark
    /// has hidden setup-side overhead and (R5) cannot deliver 16×
    /// reduction via this path.
    #[test]
    #[ignore = "(c)-2d n_aux scaling probe: heavy ppsnark on multiple shapes"]
    fn n_aux_scales_with_synthetic_shape() {
        // Scan shapes: small, medium, large.
        let chain_lens = [8usize, 32, 128, 512, 2048];
        let mut results = Vec::new();
        for &n in &chain_lens {
            let m = measure_synthetic_n_aux(n, b"ev-naux-scale")
                .expect("synthetic ppsnark probe");
            println!(
                "N_AUX_SCALE chain_len={} n_aux={} log_n_aux={} \
                 shape_num_cons={} shape_num_vars={}",
                n, m.n_aux, m.log_n_aux, m.shape_num_cons, m.shape_num_vars
            );
            results.push((n, m));
        }
        // Sanity: at smallest shape (8 cons), n_aux must be < 1,024
        // for (R5) to be feasible. If smallest shape still has
        // n_aux ≥ 1,024, ppsnark floor blocks (R5) regardless of
        // architectural CycleFold redesign.
        let smallest = &results[0].1;
        println!(
            "FALSIFIER_CHECK smallest_n_aux={} threshold=1024 R5_feasible={}",
            smallest.n_aux,
            smallest.n_aux < 1024
        );
        // Report all results in JSON-ish form for downstream parsing.
        print!("N_AUX_SCALING_TABLE [");
        for (i, (n, m)) in results.iter().enumerate() {
            if i > 0 { print!(","); }
            print!(
                "{{\"chain_len\":{},\"n_aux\":{},\"log_n_aux\":{},\"cons\":{},\"vars\":{}}}",
                n, m.n_aux, m.log_n_aux, m.shape_num_cons, m.shape_num_vars
            );
        }
        println!("]");
    }
}

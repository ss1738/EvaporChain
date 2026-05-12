//! Phase 2.2-section-2 prep — empirical R1CS-constraint budget for a
//! single Poseidon absorb-and-squeeze invocation using arkworks'
//! [`PoseidonSpongeVar`].
//!
//! # Why this module
//!
//! Section 2 of the verifier circuit re-hashes the nova-snark
//! transcript inside R1CS. The constraint count is the single
//! biggest unknown driving the trusted-setup `2^N` ceiling: if one
//! Poseidon costs >2^14 constraints and we need *two* of them
//! (primary + secondary hashers), we're already pushing 2^15. Add
//! Section 3 (RelaxedR1CS satisfiability) on top and the ceiling
//! decision (2^18 vs 2^20 Powers-of-Tau ceremony) flips.
//!
//! This module gives an EMPIRICAL number for arkworks' default
//! Poseidon gadget cost, today. The actual chain-side gadget will
//! use neptune-aligned constants (see PR #65 `poseidon_transcript`
//! for the alignment spec), but the underlying permutation cost is
//! determined by `(state_size, full_rounds, partial_rounds, alpha)`
//! which are the same shape parameters between arkworks and neptune.
//! Arkworks' default config therefore gives a credible ballpark.
//!
//! # What's NOT here
//!
//! - Real Section 2 gadget. This is just the cost probe.
//! - Neptune constant equivalence. That's the BESPOKE port.
//! - Multi-hash batching optimization. Section 2 needs two hashes;
//!   merging their permutations could save some constraints but
//!   not many.

use ark_bn254::Fr as Bn254Fr;
use ark_crypto_primitives::sponge::poseidon::constraints::PoseidonSpongeVar;
use ark_crypto_primitives::sponge::poseidon::traits::find_poseidon_ark_and_mds;
use ark_crypto_primitives::sponge::poseidon::PoseidonConfig;
use ark_crypto_primitives::sponge::constraints::CryptographicSpongeVar;
use ark_ff::PrimeField;
use ark_r1cs_std::alloc::AllocVar;
use ark_r1cs_std::fields::fp::FpVar;
use ark_relations::r1cs::{ConstraintSystem, ConstraintSystemRef, SynthesisError};

/// Build an `(8, 60, alpha=5, rate=2, capacity=1)` Poseidon config
/// over BN254 Fr. These are *generic* security parameters in the
/// arkworks-default style — not the neptune-aligned constants that
/// Section 2's actual gadget will need.
///
/// `find_poseidon_ark_and_mds` generates ARK + MDS deterministically
/// from the field characteristic + structural parameters, so this
/// is reproducible across runs.
pub fn arkworks_default_config_for_bn254() -> PoseidonConfig<Bn254Fr> {
    let full_rounds = 8usize;
    let partial_rounds = 60usize;
    let alpha = 5u64;
    let rate = 2usize;
    let capacity = 1usize;
    let prime_bits = Bn254Fr::MODULUS_BIT_SIZE as u64;
    let (ark, mds) = find_poseidon_ark_and_mds::<Bn254Fr>(
        prime_bits,
        rate,
        full_rounds as u64,
        partial_rounds as u64,
        0u64,
    );
    PoseidonConfig::new(full_rounds, partial_rounds, alpha, mds, ark, rate, capacity)
}

/// Constraint-count breakdown for a single Poseidon absorb→squeeze
/// inside R1CS, plus the per-field-element absorb counts on either
/// side. Section 2's full primary-side hash absorbs 6 elements
/// (digest + num_steps + z0 + zi + RelaxedR1CS-instance fields +
/// ri_primary) at `z_arity=1`; the secondary side has the same
/// shape with two zero sentinels in place of z0/zi.
#[derive(Clone, Copy, Debug)]
pub struct PoseidonBudget {
    /// `cs.num_instance_variables()` after running the budget circuit.
    pub instance_variables: usize,
    /// `cs.num_witness_variables()` after running the budget circuit.
    pub witness_variables: usize,
    /// `cs.num_constraints()` after running the budget circuit —
    /// the empirical constraint cost of one absorb-and-squeeze.
    pub constraints: usize,
    /// Number of absorbs the budget circuit ran.
    pub absorbs: usize,
}

/// Run a fresh `ConstraintSystem`, allocate `num_absorbs` public-input
/// scalars, absorb them all into a `PoseidonSpongeVar`, and squeeze
/// one element out. Returns the post-run shape.
///
/// `num_absorbs = 6` matches Section 2's primary-side absorb count
/// for `z_arity=1`. Picking `num_absorbs` higher than that
/// over-estimates (more absorb-batch permutations); picking lower
/// under-estimates. The arkworks default sponge auto-permutes when
/// the absorb buffer exceeds `rate`, so the per-permutation count
/// is what dominates as `num_absorbs` grows.
pub fn measure_budget(num_absorbs: usize) -> Result<PoseidonBudget, SynthesisError> {
    let cs: ConstraintSystemRef<Bn254Fr> = ConstraintSystem::<Bn254Fr>::new_ref();
    let config = arkworks_default_config_for_bn254();
    let mut sponge = PoseidonSpongeVar::<Bn254Fr>::new(cs.clone(), &config);

    let inputs: Vec<FpVar<Bn254Fr>> = (0..num_absorbs)
        .map(|i| {
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(i as u64 + 1)))
        })
        .collect::<Result<Vec<_>, _>>()?;

    for input in &inputs {
        sponge.absorb(input)?;
    }
    let _squeeze = sponge.squeeze_field_elements(1)?;

    Ok(PoseidonBudget {
        instance_variables: cs.num_instance_variables(),
        witness_variables: cs.num_witness_variables(),
        constraints: cs.num_constraints(),
        absorbs: num_absorbs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Empirical Poseidon-permutation cost report for arity-3
    /// state (rate=2, capacity=1) at the absorb count Section 2's
    /// primary hasher will need. Pin the numbers as documentation
    /// — they will shift as ark-crypto-primitives' Poseidon gadget
    /// evolves and the test fires when that happens.
    ///
    /// **What good numbers look like.** A single Poseidon-128
    /// permutation at `(state_size=3, full_rounds=8,
    /// partial_rounds=60, alpha=5)` typically costs O(200-400)
    /// R1CS constraints in arkworks. Absorbing 6 elements into a
    /// rate-2 sponge triggers 3 permutations, so we expect
    /// somewhere around O(600-1200). Squeezing 1 element triggers
    /// no additional permutations (it's just a state read).
    #[test]
    fn poseidon_absorb_6_squeeze_1_budget() {
        let budget = measure_budget(6).expect("budget measure");
        // Pin: must be within the arkworks-Poseidon ballpark.
        // If this fires LOW, ark-crypto-primitives may have
        // optimized the gadget. If it fires HIGH, regression.
        assert!(
            (200..=10_000).contains(&budget.constraints),
            "Poseidon budget out of expected range: {} constraints",
            budget.constraints
        );
        assert_eq!(budget.absorbs, 6);
        eprintln!(
            "poseidon_budget: absorbs=6 → instance_vars={} witness_vars={} constraints={}",
            budget.instance_variables, budget.witness_variables, budget.constraints
        );
    }

    /// Confirm the cost scales roughly linearly with absorbs once
    /// absorbs exceeds the rate (each rate-many absorbs triggers
    /// one permutation). Catches a regression where every absorb
    /// might suddenly trigger a permutation.
    #[test]
    fn poseidon_cost_grows_with_absorbs() {
        let b3 = measure_budget(3).expect("3 absorbs");
        let b6 = measure_budget(6).expect("6 absorbs");
        let b12 = measure_budget(12).expect("12 absorbs");

        eprintln!("poseidon_budget: 3→{} 6→{} 12→{} constraints",
                  b3.constraints, b6.constraints, b12.constraints);

        // Roughly: doubling absorbs should not more than quadruple
        // constraints (allows for a fixed setup overhead + per-permutation
        // cost). Loose bound — tightens the regression net without
        // making this brittle.
        assert!(b6.constraints >= b3.constraints, "6-absorb must cost ≥ 3-absorb");
        assert!(b12.constraints >= b6.constraints, "12-absorb must cost ≥ 6-absorb");
        assert!(
            b12.constraints <= 4 * b6.constraints,
            "12-absorb must not blow up >4× over 6-absorb"
        );
    }

    /// Section 2 needs TWO Poseidon hashes (primary + secondary)
    /// per verifier circuit invocation. This test projects the
    /// total Section-2 cost from the single-hash budget. Useful
    /// for documenting the 2^N ceremony pick.
    #[test]
    fn section_2_constraint_projection() {
        // Primary side: 6 absorbs (digest, num_steps, z0[0],
        // zi[0], RelaxedR1CS-instance expansion, ri_primary)
        // for z_arity=1. The instance expansion is multi-scalar
        // (~4-8 entries depending on encoding); use 6 as an
        // optimistic lower bound and ~12 as upper.
        //
        // Secondary side: same shape (two zero sentinels in place
        // of z0/zi, instance + ri swap).
        let single_low = measure_budget(6).expect("6 absorbs");
        let single_high = measure_budget(12).expect("12 absorbs");

        let section_2_low = 2 * single_low.constraints;
        let section_2_high = 2 * single_high.constraints;

        eprintln!(
            "section_2_projection: low={} high={} constraints (×2 hashers)",
            section_2_low, section_2_high
        );

        // Document the budget against the Powers-of-Tau ceiling.
        // 2^18 = 262_144 constraints. Section 2 alone should be
        // O(few × 10³) — well under the ceiling. Section 3 is
        // the unknown.
        assert!(
            section_2_high < 1 << 18,
            "Section 2 projection {} must fit in 2^18 ceremony",
            section_2_high
        );
    }
}

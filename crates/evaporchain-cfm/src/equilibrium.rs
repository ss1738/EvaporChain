//! `cfm_equilibrium` — Boltzmann-reweight a mempool distribution.
//!
//! Given a mempool histogram `ρ_mempool(f)` over a finite set of fee
//! buckets `fees`, and an inverse temperature `β` in millibits, the
//! equilibrium distribution is
//!
//! ```text
//!   p_eq(f_i) ∝ exp(−β · f_i) · ρ_mempool(f_i)
//! ```
//!
//! Renormalised to a proper [`Distribution`] (sums to
//! `FIXED_POINT_SCALE`). Boundary behaviour:
//!
//! - `β = 0`: equilibrium = mempool (no reweighting).
//! - `β → ∞`: equilibrium concentrates on the lowest-fee bucket whose
//!   `ρ_mempool` is non-zero.
//! - All weights zero (e.g. extreme β with no zero-fee bucket): error,
//!   the equilibrium is undefined.

use thiserror::Error;

use crate::weight::boltzmann_weight;
use evaporchain_sanov_slashing::{Distribution, DistributionError};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EquilibriumError {
    #[error("mempool pmf and fees alphabets differ: |ρ|={pmf_len}, |fees|={fees_len}")]
    AlphabetMismatch { pmf_len: usize, fees_len: usize },
    #[error(
        "all reweighted bins collapsed to zero — β too large for this fee range, equilibrium undefined"
    )]
    AllZero,
    #[error("distribution build failed: {0}")]
    DistributionBuild(#[from] DistributionError),
}

/// Build the CFM equilibrium distribution from a mempool pmf and the
/// per-bucket fee values.
///
/// Both `mempool_pmf` and `fees` are length-N vectors, with `pmf[i]`
/// the (fixed-point) probability of fee bucket `fees[i]`. The result
/// is a renormalised [`Distribution`] over the same N-bucket alphabet.
pub fn cfm_equilibrium(
    mempool_pmf: &[u64],
    fees: &[u64],
    beta_mb: u64,
) -> Result<Distribution, EquilibriumError> {
    if mempool_pmf.len() != fees.len() {
        return Err(EquilibriumError::AlphabetMismatch {
            pmf_len: mempool_pmf.len(),
            fees_len: fees.len(),
        });
    }
    // Reweight: w_i = ρ_i × Boltzmann(f_i, β). u128 intermediate to
    // avoid overflow at extreme magnitudes.
    let raw_weights: Vec<u128> = mempool_pmf
        .iter()
        .zip(fees.iter())
        .map(|(&p, &f)| (p as u128).saturating_mul(boltzmann_weight(f, beta_mb) as u128))
        .collect();
    let total: u128 = raw_weights.iter().sum();
    if total == 0 {
        return Err(EquilibriumError::AllZero);
    }
    // Renormalise into u64 counts that sum to FIXED_POINT_SCALE.
    // Use the same residual-to-largest-bucket trick as
    // Distribution::from_counts so the floor-division residual lands
    // somewhere meaningful.
    let scale = evaporchain_sanov_slashing::FIXED_POINT_SCALE as u128;
    let mut pmf: Vec<u64> = raw_weights
        .iter()
        .map(|&w| (w * scale / total) as u64)
        .collect();
    let assigned: u64 = pmf.iter().fold(0u64, |a, b| a.saturating_add(*b));
    if assigned < evaporchain_sanov_slashing::FIXED_POINT_SCALE {
        let residual = evaporchain_sanov_slashing::FIXED_POINT_SCALE - assigned;
        let max_idx = pmf
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| **v)
            .map(|(i, _)| i)
            .unwrap_or(0);
        pmf[max_idx] = pmf[max_idx].saturating_add(residual);
    }
    Distribution::new(pmf).map_err(EquilibriumError::DistributionBuild)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_sanov_slashing::FIXED_POINT_SCALE;

    #[test]
    fn beta_zero_yields_input_distribution() {
        let mempool_pmf = vec![400_000, 600_000];
        let fees = vec![1, 2];
        let eq = cfm_equilibrium(&mempool_pmf, &fees, 0).unwrap();
        // β = 0 means weights are all MAX_WEIGHT → renormalise to mempool.
        assert_eq!(eq.pmf, vec![400_000, 600_000]);
    }

    #[test]
    fn alphabet_mismatch_rejected() {
        let err = cfm_equilibrium(&[FIXED_POINT_SCALE], &[1, 2], 0).unwrap_err();
        assert!(matches!(err, EquilibriumError::AlphabetMismatch { .. }));
    }

    #[test]
    fn high_beta_concentrates_on_lowest_fee() {
        // Uniform mempool, fee buckets {0, 8, 16}.
        // β = 1000 → weight(0)=MAX, weight(8)=MAX>>8, weight(16)=MAX>>16.
        // Equilibrium is dominated by the f=0 bucket.
        let pmf = vec![333_333, 333_333, 333_334];
        let fees = vec![0, 8, 16];
        let eq = cfm_equilibrium(&pmf, &fees, 1_000).unwrap();
        assert!(eq.pmf[0] > eq.pmf[1]);
        assert!(eq.pmf[1] > eq.pmf[2]);
        // f=0 bucket should hold most of the mass.
        assert!(eq.pmf[0] > FIXED_POINT_SCALE * 2 / 3);
    }

    #[test]
    fn equilibrium_sums_to_fixed_point_scale() {
        let pmf = vec![100_000, 200_000, 300_000, 400_000];
        let fees = vec![1, 2, 3, 4];
        let eq = cfm_equilibrium(&pmf, &fees, 500).unwrap();
        let sum: u64 = eq.pmf.iter().sum();
        assert_eq!(sum, FIXED_POINT_SCALE);
    }

    #[test]
    fn all_zero_pmf_errs() {
        let pmf = vec![0u64, 0u64];
        let fees = vec![1, 2];
        let err = cfm_equilibrium(&pmf, &fees, 0).unwrap_err();
        assert!(matches!(err, EquilibriumError::AllZero));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use evaporchain_sanov_slashing::FIXED_POINT_SCALE;
    use proptest::prelude::*;

    fn arb_pmf2() -> impl Strategy<Value = Vec<u64>> {
        (1u64..FIXED_POINT_SCALE).prop_map(|p| vec![p, FIXED_POINT_SCALE - p])
    }

    proptest! {
        /// Property: CFM equilibrium always sums to FIXED_POINT_SCALE
        /// when it produces a result.
        ///
        /// For extreme β·fee combinations, all weights collapse to zero
        /// and the equilibrium is genuinely undefined — the substrate
        /// returns `AllZero` and we accept that as correct behavior
        /// rather than asserting a normalised result.
        #[test]
        fn equilibrium_always_normalises(
            pmf in arb_pmf2(),
            fees in proptest::collection::vec(0u64..1_000_000, 2..=2),
            beta_mb in 0u64..10_000,
        ) {
            match cfm_equilibrium(&pmf, &fees, beta_mb) {
                Ok(eq) => {
                    let sum: u64 = eq.pmf.iter().sum();
                    prop_assert_eq!(sum, FIXED_POINT_SCALE);
                }
                Err(EquilibriumError::AllZero) => {
                    // Acceptable: extreme β collapses every weight to 0.
                }
                Err(other) => prop_assert!(false, "unexpected error: {other}"),
            }
        }

        /// Property: at β = 0, equilibrium equals the input pmf
        /// (modulo the residual rebalance — exact when input itself
        /// already sums to FIXED_POINT_SCALE).
        #[test]
        fn beta_zero_is_identity(pmf in arb_pmf2()) {
            let fees = vec![1u64, 2u64];
            let eq = cfm_equilibrium(&pmf, &fees, 0).unwrap();
            prop_assert_eq!(eq.pmf, pmf);
        }
    }
}

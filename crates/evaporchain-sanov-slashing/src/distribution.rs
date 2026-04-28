//! `Distribution` — a probability distribution over a finite outcome
//! alphabet, stored as fixed-point integers that must sum to
//! `FIXED_POINT_SCALE` (= 1.0). All chain code is integer-deterministic;
//! we never touch floats.
//!
//! For a 2-outcome example (block-produced vs block-missed) over the
//! last 1024 slots: a validator that produced 1000 of 1024 has
//! `Q = (1000 * SCALE / 1024, 24 * SCALE / 1024)`. Honest behaviour
//! is `P = (1023 * SCALE / 1024, 1 * SCALE / 1024)` (target ≈ 0.1%
//! miss rate).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Fixed-point scale: every probability is stored as `floor(p × SCALE)`.
/// 1_000_000 = 6 significant decimal digits, comfortably inside `u64`
/// even for distributions with hundreds of thousands of outcomes.
pub const FIXED_POINT_SCALE: u64 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Distribution {
    /// `pmf[i]` is `floor(p_i × FIXED_POINT_SCALE)`.
    pub pmf: Vec<u64>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DistributionError {
    #[error("pmf sums to {got}, expected exactly {expected}")]
    BadSum { got: u64, expected: u64 },
    #[error("pmf is empty — at least one outcome required")]
    Empty,
    #[error("alphabet mismatch: |Q|={q_len} != |P|={p_len}")]
    AlphabetMismatch { q_len: usize, p_len: usize },
}

impl Distribution {
    /// Construct after validating that the pmf sums to
    /// `FIXED_POINT_SCALE` exactly. Use `from_counts` for the common
    /// "I have n_i observations, build the empirical distribution" path.
    pub fn new(pmf: Vec<u64>) -> Result<Self, DistributionError> {
        if pmf.is_empty() {
            return Err(DistributionError::Empty);
        }
        let sum: u64 = pmf.iter().fold(0u64, |a, b| a.saturating_add(*b));
        if sum != FIXED_POINT_SCALE {
            return Err(DistributionError::BadSum {
                got: sum,
                expected: FIXED_POINT_SCALE,
            });
        }
        Ok(Self { pmf })
    }

    /// Empirical distribution from raw counts. Renormalises so the
    /// fixed-point pmf sums to exactly `FIXED_POINT_SCALE` — any
    /// floor-division residual is absorbed into the largest bucket.
    pub fn from_counts(counts: &[u64]) -> Result<Self, DistributionError> {
        if counts.is_empty() {
            return Err(DistributionError::Empty);
        }
        let total: u128 = counts.iter().map(|&c| c as u128).sum();
        if total == 0 {
            return Err(DistributionError::BadSum {
                got: 0,
                expected: FIXED_POINT_SCALE,
            });
        }
        let scale = FIXED_POINT_SCALE as u128;
        let mut pmf: Vec<u64> = counts
            .iter()
            .map(|&c| ((c as u128 * scale) / total) as u64)
            .collect();
        let assigned: u64 = pmf.iter().fold(0u64, |a, b| a.saturating_add(*b));
        if assigned < FIXED_POINT_SCALE {
            // Push the residual into the largest bucket — keeps the sum
            // exact and biases away from rounding-down zero outcomes.
            let residual = FIXED_POINT_SCALE - assigned;
            let max_idx = pmf
                .iter()
                .enumerate()
                .max_by_key(|(_, v)| **v)
                .map(|(i, _)| i)
                .unwrap_or(0);
            pmf[max_idx] = pmf[max_idx].saturating_add(residual);
        }
        Self::new(pmf)
    }

    pub fn alphabet_size(&self) -> usize {
        self.pmf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_exact_pmf_round_trip() {
        let d = Distribution::new(vec![400_000, 600_000]).unwrap();
        assert_eq!(d.pmf, vec![400_000, 600_000]);
    }

    #[test]
    fn empty_pmf_rejected() {
        assert!(matches!(
            Distribution::new(vec![]).unwrap_err(),
            DistributionError::Empty
        ));
    }

    #[test]
    fn bad_sum_rejected() {
        assert!(matches!(
            Distribution::new(vec![400_000, 400_000]).unwrap_err(),
            DistributionError::BadSum { .. }
        ));
    }

    #[test]
    fn from_counts_basic() {
        let d = Distribution::from_counts(&[3, 1]).unwrap();
        // 3/4 = 750_000, 1/4 = 250_000 — exact, no residual.
        assert_eq!(d.pmf, vec![750_000, 250_000]);
    }

    #[test]
    fn from_counts_residual_goes_to_largest() {
        // 1024 slots, 1000 produced + 24 missed → 976_562, 23_437 with
        // residual 1 → goes to bucket 0.
        let d = Distribution::from_counts(&[1000, 24]).unwrap();
        let sum: u64 = d.pmf.iter().sum();
        assert_eq!(sum, FIXED_POINT_SCALE);
        // Bucket 0 is the largest count, gets the residual.
        assert!(d.pmf[0] > d.pmf[1]);
    }

    #[test]
    fn from_counts_zero_total_rejected() {
        assert!(matches!(
            Distribution::from_counts(&[0, 0]).unwrap_err(),
            DistributionError::BadSum { .. }
        ));
    }
}

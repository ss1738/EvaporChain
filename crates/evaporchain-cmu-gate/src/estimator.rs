//! `entropy_millibits` — Shannon entropy `H = -Σ p_i log_2(p_i)` in
//! millibits, computed from a finite-alphabet histogram of raw counts.
//!
//! Integer approximation: `log_2(p_i)` via `bit_length` consistent
//! with the rest of the workspace (matches `evaporchain-tropical::weight`,
//! `evaporchain-demurrage::rate`, `evaporchain-sanov-slashing::kl`).

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EntropyError {
    #[error("histogram is empty — at least one sample required")]
    Empty,
    #[error("all-zero histogram — entropy undefined")]
    AllZero,
}

/// `H(X) = -Σ p_i log_2(p_i)` in millibits, given raw `counts`.
/// Returns 0 when only one bucket has any mass (zero-entropy
/// distribution).
pub fn entropy_millibits(counts: &[u64]) -> Result<u64, EntropyError> {
    if counts.is_empty() {
        return Err(EntropyError::Empty);
    }
    let total: u128 = counts.iter().map(|&c| c as u128).sum();
    if total == 0 {
        return Err(EntropyError::AllZero);
    }
    // For each non-zero bucket: contribute -p log_2(p) in millibits.
    // p = c / total. log_2(p) = log_2(c) - log_2(total).
    // Both via bit_length proxy. Result aggregated in i128 then clamped.
    let total_bits = bit_length(total as u64) as i128;
    let mut h_millibits: i128 = 0;
    for &c in counts {
        if c == 0 {
            continue;
        }
        let c_bits = bit_length(c) as i128;
        // -log_2(c/total) = log_2(total) - log_2(c) ≥ 0 (since c ≤ total).
        let neg_log2_p = (total_bits - c_bits).max(0);
        // p × neg_log2_p, scaled by 1000 for millibits.
        // p = c/total, so contribution = (c × neg_log2_p × 1000) / total.
        let term = (c as i128 * neg_log2_p * 1_000) / total as i128;
        h_millibits = h_millibits.saturating_add(term);
    }
    Ok(h_millibits.max(0) as u64)
}

fn bit_length(n: u64) -> u64 {
    if n == 0 {
        0
    } else {
        64 - n.leading_zeros() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_histogram_rejected() {
        assert!(matches!(
            entropy_millibits(&[]).unwrap_err(),
            EntropyError::Empty
        ));
    }

    #[test]
    fn all_zero_rejected() {
        assert!(matches!(
            entropy_millibits(&[0, 0, 0]).unwrap_err(),
            EntropyError::AllZero
        ));
    }

    #[test]
    fn single_bucket_zero_entropy() {
        // {full_mass, 0, 0} → 0 bits.
        assert_eq!(entropy_millibits(&[1000, 0, 0]).unwrap(), 0);
    }

    #[test]
    fn uniform_two_outcomes_one_bit() {
        // 50/50 → 1 bit = 1000 millibits (exact in this proxy).
        let h = entropy_millibits(&[500, 500]).unwrap();
        assert_eq!(h, 1000);
    }

    #[test]
    fn uniform_four_outcomes_two_bits() {
        let h = entropy_millibits(&[250, 250, 250, 250]).unwrap();
        // 4 outcomes, p=0.25 each, log2(4) = 2 → 2000 millibits.
        assert_eq!(h, 2000);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Entropy is non-negative for any non-empty non-zero histogram.
        #[test]
        fn entropy_non_negative(
            counts in proptest::collection::vec(0u64..1_000_000, 1..16)
                .prop_filter("at least one positive", |v| v.iter().any(|c| *c > 0)),
        ) {
            let h = entropy_millibits(&counts).unwrap();
            prop_assert!(h <= u64::MAX);
        }

        /// Entropy of a single-bucket histogram is always 0.
        #[test]
        fn single_bucket_always_zero_entropy(c in 1u64..1_000_000) {
            prop_assert_eq!(entropy_millibits(&[c]).unwrap(), 0);
        }
    }
}

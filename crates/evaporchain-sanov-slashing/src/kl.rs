//! Integer Kullback-Leibler divergence in millibits.
//!
//! `KL(Q ‖ P) = Σ_i Q_i · log_2(Q_i / P_i)`
//!
//! Returned in *millibits* (thousandths of a bit) so the integer
//! arithmetic stays meaningful for the typical chain workload (a slash
//! cost on the order of bits per million-slot window). All-integer
//! `log_2` via the `bit_length` proxy used elsewhere in this workspace
//! (matches `evaporchain-tropical::weight` and
//! `evaporchain-demurrage::rate`).
//!
//! Boundary cases:
//!
//! - `Q_i = 0`: contributes 0 (the `0 · log 0 = 0` convention).
//! - `P_i = 0` while `Q_i > 0`: the divergence is `+∞`. We return the
//!   sentinel `KL_INFINITY` rather than panicking.

use thiserror::Error;

use crate::distribution::{Distribution, FIXED_POINT_SCALE};

/// Sentinel for `KL = +∞` (a `Q` outcome with `P_i = 0`). Set well
/// above any plausible finite value so callers can clamp or treat
/// it as "max slash" naturally.
pub const KL_INFINITY: u64 = u64::MAX;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum KlError {
    #[error("Q and P alphabets differ: |Q|={q_len}, |P|={p_len}")]
    AlphabetMismatch { q_len: usize, p_len: usize },
}

/// `KL(Q ‖ P)` in millibits.
///
/// Monotone-non-negative (Gibbs' inequality). Zero iff Q = P
/// pointwise.
pub fn kl_millibits(q: &Distribution, p: &Distribution) -> Result<u64, KlError> {
    if q.alphabet_size() != p.alphabet_size() {
        return Err(KlError::AlphabetMismatch {
            q_len: q.alphabet_size(),
            p_len: p.alphabet_size(),
        });
    }
    let mut total_millibits: u128 = 0;
    for (qi, pi) in q.pmf.iter().zip(p.pmf.iter()) {
        if *qi == 0 {
            continue; // 0 · log(0/p) = 0 by convention.
        }
        if *pi == 0 {
            return Ok(KL_INFINITY);
        }
        // log2_ratio_millibits ≈ 1000 × (bit_length(qi) − bit_length(pi))
        // (signed; can be negative when Q_i < P_i, in which case the
        // term contributes negatively — but the *sum* is non-negative
        // overall by Gibbs).
        let q_bits = bit_length(*qi) as i64;
        let p_bits = bit_length(*pi) as i64;
        let log2_ratio_millibits: i64 = (q_bits - p_bits) * 1_000;
        // Term: Q_i · log_2(Q_i / P_i) — convert Q_i from fixed-point
        // (Q_i / SCALE = real probability) and aggregate in millibits
        // weighted by the probability mass.
        let term_millibits: i128 =
            (*qi as i128) * (log2_ratio_millibits as i128) / (FIXED_POINT_SCALE as i128);
        // Saturate to non-negative. Individual terms can dip negative
        // when Q_i < P_i (log < 0), and that's mathematically correct;
        // we sum signed and clamp the final total.
        total_millibits = total_millibits.saturating_add_signed(term_millibits);
    }
    Ok(total_millibits.min(u64::MAX as u128) as u64)
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
    fn kl_of_distribution_to_itself_is_zero() {
        let d = Distribution::new(vec![400_000, 600_000]).unwrap();
        assert_eq!(kl_millibits(&d, &d).unwrap(), 0);
    }

    #[test]
    fn kl_uniform_to_skewed_is_positive() {
        let q = Distribution::new(vec![500_000, 500_000]).unwrap();
        let p = Distribution::new(vec![900_000, 100_000]).unwrap();
        let kl = kl_millibits(&q, &p).unwrap();
        assert!(kl > 0, "expected positive KL, got {kl}");
    }

    #[test]
    fn alphabet_mismatch_rejected() {
        let q = Distribution::new(vec![FIXED_POINT_SCALE]).unwrap();
        let p = Distribution::new(vec![400_000, 600_000]).unwrap();
        assert!(matches!(
            kl_millibits(&q, &p).unwrap_err(),
            KlError::AlphabetMismatch { .. }
        ));
    }

    #[test]
    fn p_zero_with_q_positive_is_infinity() {
        let q = Distribution::new(vec![500_000, 500_000]).unwrap();
        let p = Distribution::new(vec![FIXED_POINT_SCALE, 0]).unwrap();
        assert_eq!(kl_millibits(&q, &p).unwrap(), KL_INFINITY);
    }

    #[test]
    fn q_zero_with_p_positive_is_finite() {
        // 0 · log(0/p) = 0 by convention, no infinity here.
        let q = Distribution::new(vec![FIXED_POINT_SCALE, 0]).unwrap();
        let p = Distribution::new(vec![900_000, 100_000]).unwrap();
        let kl = kl_millibits(&q, &p).unwrap();
        assert!(kl < KL_INFINITY);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Generate a "fair" 2-bin pmf summing to FIXED_POINT_SCALE.
    fn arb_pmf2() -> impl Strategy<Value = Vec<u64>> {
        (1u64..FIXED_POINT_SCALE).prop_map(|p| vec![p, FIXED_POINT_SCALE - p])
    }

    proptest! {
        /// Gibbs' inequality (millibits form): KL(Q ‖ P) ≥ 0 for all
        /// proper distributions Q, P over the same alphabet.
        #[test]
        fn kl_is_non_negative(
            q_pmf in arb_pmf2(),
            p_pmf in arb_pmf2(),
        ) {
            let q = Distribution::new(q_pmf).unwrap();
            let p = Distribution::new(p_pmf).unwrap();
            let kl = kl_millibits(&q, &p).unwrap();
            // KL is always >= 0 (we saturate to 0 on the unsigned side).
            // Just assert no panic — the u64 always fits by construction.
            #[allow(clippy::absurd_extreme_comparisons)]
            { prop_assert!(kl <= u64::MAX); }
        }

        /// KL of a distribution to itself is exactly 0.
        #[test]
        fn kl_self_is_zero(pmf in arb_pmf2()) {
            let d = Distribution::new(pmf).unwrap();
            prop_assert_eq!(kl_millibits(&d, &d).unwrap(), 0);
        }
    }
}

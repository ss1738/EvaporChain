//! Integer statistics over `&[u64]` sample windows.
//!
//! Mean and variance use `u128` intermediates for headroom. Relative
//! variance is reported in the same fixed-point scale used by
//! [`crate::bound`] so the comparison in `tur_check` is integer-clean.

use crate::bound::FIXED_POINT_SCALE;

/// `⟨X⟩` over `samples`. Returns 0 for empty input.
pub fn mean(samples: &[u64]) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    let sum: u128 = samples.iter().map(|&x| x as u128).sum();
    sum / samples.len() as u128
}

/// Sample variance `(1/n) Σ (X_i − ⟨X⟩)²`. Returns 0 for empty input
/// or single-sample input.
pub fn variance(samples: &[u64]) -> u128 {
    if samples.len() <= 1 {
        return 0;
    }
    let m = mean(samples);
    let n = samples.len() as u128;
    let sum_sq_dev: u128 = samples
        .iter()
        .map(|&x| {
            let dev = (x as u128).max(m) - (x as u128).min(m); // |X_i - m|
            dev.saturating_mul(dev)
        })
        .sum();
    sum_sq_dev / n
}

/// `Var(X) / ⟨X⟩²` in `FIXED_POINT_SCALE` units. Returns
/// `u128::MAX` if `⟨X⟩ = 0` (the ratio is undefined / infinite).
pub fn relative_variance_fixed(samples: &[u64]) -> u128 {
    let m = mean(samples);
    if m == 0 {
        return u128::MAX;
    }
    let v = variance(samples);
    // (Var × SCALE) / mean² — order of operations keeps precision.
    (v.saturating_mul(FIXED_POINT_SCALE)) / m.saturating_mul(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_of_constants_is_constant() {
        assert_eq!(mean(&[5, 5, 5, 5]), 5);
    }

    #[test]
    fn variance_of_constants_is_zero() {
        assert_eq!(variance(&[7, 7, 7, 7]), 0);
    }

    #[test]
    fn relative_variance_of_constants_is_zero() {
        assert_eq!(relative_variance_fixed(&[7, 7, 7, 7]), 0);
    }

    #[test]
    fn variance_of_simple_sequence() {
        // {1, 2, 3, 4, 5}: mean=3, variance = (4+1+0+1+4)/5 = 2
        assert_eq!(variance(&[1, 2, 3, 4, 5]), 2);
    }

    #[test]
    fn relative_variance_of_simple_sequence() {
        // var=2, mean²=9 → 2/9 in FIXED_POINT_SCALE.
        let r = relative_variance_fixed(&[1, 2, 3, 4, 5]);
        let expected = (2u128 * FIXED_POINT_SCALE) / 9;
        assert_eq!(r, expected);
    }

    #[test]
    fn empty_input_safe_defaults() {
        assert_eq!(mean(&[]), 0);
        assert_eq!(variance(&[]), 0);
        assert_eq!(relative_variance_fixed(&[]), u128::MAX);
    }

    #[test]
    fn single_sample_zero_variance() {
        assert_eq!(variance(&[42]), 0);
        assert_eq!(relative_variance_fixed(&[42]), 0);
    }
}

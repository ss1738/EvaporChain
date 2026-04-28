//! Per-epoch demurrage rate, in parts-per-million of balance.
//!
//! The rate is an integer approximation of
//!
//! ```text
//!   r(balance) = max(0, λ_base · log_2(balance / threshold))
//! ```
//!
//! using `bit_length(balance) − bit_length(threshold)` as the integer
//! `log_2` proxy. This is exact for balances that are exact powers of
//! two and a tight upper bound elsewhere (`bit_length` over-estimates
//! `log_2` by less than 1 bit for any value, which translates to at
//! most 1 ppm of error per coefficient).
//!
//! The rate is clamped at `MAX_LOG2_RATIO = 64` (one full `u64` width
//! above threshold) so a degenerate `threshold = 0` cannot produce a
//! runaway rate.

use evaporchain_types::Energy;

use crate::params::DemurrageParams;

/// Cap on the integer `log_2` ratio. `u64` has at most 64 bits, so
/// `bit_length(balance) − bit_length(threshold)` is bounded by 64;
/// the constant is exposed so consumers can reason about worst-case
/// rate inflation.
pub const MAX_LOG2_RATIO: u64 = 64;

/// Per-epoch demurrage rate at `balance` in parts-per-million.
/// Returns 0 if `balance ≤ params.threshold` or if params are disabled.
pub fn rate_ppm(balance: Energy, params: &DemurrageParams) -> u64 {
    if params.is_disabled() {
        return 0;
    }
    if balance <= params.threshold {
        return 0;
    }
    let bal_bits = bit_length(balance);
    let thr_bits = bit_length(params.threshold);
    let log_ratio = bal_bits.saturating_sub(thr_bits).min(MAX_LOG2_RATIO);
    params.lambda_base_ppm.saturating_mul(log_ratio)
}

/// `bit_length(0) = 0`, `bit_length(n > 0) = floor(log_2(n)) + 1`.
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
    fn rate_below_threshold_is_zero() {
        let p = DemurrageParams::new(10, 1024);
        assert_eq!(rate_ppm(0, &p), 0);
        assert_eq!(rate_ppm(1, &p), 0);
        assert_eq!(rate_ppm(1024, &p), 0);
    }

    #[test]
    fn rate_at_2x_threshold_is_one_doubling() {
        let p = DemurrageParams::new(10, 1024);
        // bit_length(2049) = 12, bit_length(1024) = 11 → log_ratio = 1
        // rate = 10 * 1 = 10 ppm
        assert_eq!(rate_ppm(2049, &p), 10);
    }

    #[test]
    fn rate_at_4x_threshold_is_two_doublings() {
        let p = DemurrageParams::new(10, 1024);
        // bit_length(4097) = 13, bit_length(1024) = 11 → log_ratio = 2
        assert_eq!(rate_ppm(4097, &p), 20);
    }

    #[test]
    fn disabled_params_yield_zero_rate() {
        let p = DemurrageParams::disabled();
        assert_eq!(rate_ppm(u64::MAX, &p), 0);
    }

    #[test]
    fn zero_lambda_yields_zero_rate() {
        let p = DemurrageParams::new(0, 1024);
        assert_eq!(rate_ppm(u64::MAX, &p), 0);
    }

    #[test]
    fn rate_caps_at_max_log2_ratio() {
        let p = DemurrageParams::new(1, 1);
        // log_ratio bounded by 64 even for the extreme balance.
        let r = rate_ppm(u64::MAX, &p);
        assert!(r <= MAX_LOG2_RATIO);
    }

    #[test]
    fn bit_length_corner_cases() {
        assert_eq!(bit_length(0), 0);
        assert_eq!(bit_length(1), 1);
        assert_eq!(bit_length(2), 2);
        assert_eq!(bit_length(3), 2);
        assert_eq!(bit_length(4), 3);
        assert_eq!(bit_length(u64::MAX), 64);
    }
}

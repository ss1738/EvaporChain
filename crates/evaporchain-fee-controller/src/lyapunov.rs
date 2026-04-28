//! Lyapunov function `V(E) = ½ (E − E*)²`.
//!
//! Returned in `u128` to keep the squaring chain-safe — `u64` energies
//! squared overflow at energy ≈ 4.3 × 10⁹, well within plausible chain
//! workloads.

use evaporchain_types::Energy;

/// `V(E) = ½ (E − E*)²`.
pub fn lyapunov_value(energy: Energy, target: Energy) -> u128 {
    let diff = signed_diff(energy, target).unsigned_abs() as u128;
    diff.saturating_mul(diff) / 2
}

/// Signed `E − E*` as `i128` (chosen for headroom under squaring;
/// `i64` would work for any realistic energy range but `i128` keeps
/// the kernel-internal arithmetic safe).
pub fn signed_diff(energy: Energy, target: Energy) -> i128 {
    energy as i128 - target as i128
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_target_is_zero() {
        assert_eq!(lyapunov_value(1_000, 1_000), 0);
    }

    #[test]
    fn known_values() {
        // diff = 10, V = 50
        assert_eq!(lyapunov_value(110, 100), 50);
        // diff = -10, V = 50 (squaring kills the sign)
        assert_eq!(lyapunov_value(90, 100), 50);
    }

    #[test]
    fn lyapunov_is_symmetric_around_target() {
        for delta in 1u64..1000 {
            let above = lyapunov_value(1000 + delta, 1000);
            let below = lyapunov_value(1000 - delta, 1000);
            assert_eq!(above, below);
        }
    }

    #[test]
    fn signed_diff_above_and_below() {
        assert_eq!(signed_diff(110, 100), 10);
        assert_eq!(signed_diff(90, 100), -10);
        assert_eq!(signed_diff(100, 100), 0);
    }
}

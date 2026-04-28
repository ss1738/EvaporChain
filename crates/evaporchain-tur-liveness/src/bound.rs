//! TUR bound `2/Σ` in fixed-point.
//!
//! `Σ` is the entropy production of the chain over the sample window —
//! callers compute it from the chain's accounting (fees + slashing +
//! demurrage) and pass it in. The bound is then `2/Σ` reported in the
//! same fixed-point scale [`FIXED_POINT_SCALE`] used by
//! [`crate::stats::relative_variance_fixed`] so the comparison in
//! [`crate::check::tur_check`] is integer-clean.

/// Fixed-point scale: 2^32 ≈ 10 significant decimal digits, plenty for
/// the chain-grade ratios this crate manipulates.
pub const FIXED_POINT_BITS: u32 = 32;
pub const FIXED_POINT_SCALE: u128 = 1u128 << FIXED_POINT_BITS;

/// `2 / Σ` in `FIXED_POINT_SCALE` units. Returns `u128::MAX` for
/// `Σ = 0` (the bound is then *infinite* — every relative variance is
/// allowed; no useful liveness assertion can be made).
pub fn tur_bound_fixed(sigma: u64) -> u128 {
    if sigma == 0 {
        return u128::MAX;
    }
    (2u128 * FIXED_POINT_SCALE) / sigma as u128
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tur_bound_inverse_in_sigma() {
        let a = tur_bound_fixed(2);
        let b = tur_bound_fixed(4);
        assert!(a > b, "bound shrinks as Σ grows (more entropy = tighter bound)");
        // Numeric: 2/2 vs 2/4.
        assert_eq!(a, FIXED_POINT_SCALE);
        assert_eq!(b, FIXED_POINT_SCALE / 2);
    }

    #[test]
    fn zero_sigma_is_infinite_bound() {
        assert_eq!(tur_bound_fixed(0), u128::MAX);
    }
}

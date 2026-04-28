//! Quadratic AMM pricing for one epoch of rent.
//!
//! `rent_rate(used, capacity, base) = base × (used + 1)² / capacity²`
//!
//! - Empty namespace pays the floor `base / capacity²` per epoch (the
//!   "+1" keeps the marginal cost positive even at zero utilisation
//!   so squatting on capacity has a price).
//! - Full namespace pays `base × (capacity + 1)² / capacity² ≈ base`.
//! - `capacity = 0` is rejected as undefined (a zero-capacity namespace
//!   should never have a slot to rent).

use thiserror::Error;

use evaporchain_types::Energy;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PricingError {
    #[error("capacity must be > 0 to price rent")]
    ZeroCapacity,
}

/// Rent rate in `Energy` units per epoch per slot.
///
/// `u128` intermediate to keep the squaring chain-safe; saturating to
/// `Energy::MAX` if the result somehow overflows in the unlikely
/// extreme.
pub fn rent_rate(used: u64, capacity: u64, base: Energy) -> Result<Energy, PricingError> {
    if capacity == 0 {
        return Err(PricingError::ZeroCapacity);
    }
    let numerator = (used as u128 + 1).saturating_mul(used as u128 + 1);
    let denominator = (capacity as u128).saturating_mul(capacity as u128);
    let scaled = (base as u128).saturating_mul(numerator) / denominator;
    Ok(scaled.min(Energy::MAX as u128) as Energy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_capacity_rejected() {
        assert_eq!(rent_rate(0, 0, 1_000).unwrap_err(), PricingError::ZeroCapacity);
    }

    #[test]
    fn empty_namespace_pays_floor() {
        // base = 100, capacity = 10, used = 0 → 100 * 1 / 100 = 1
        assert_eq!(rent_rate(0, 10, 100).unwrap(), 1);
    }

    #[test]
    fn full_namespace_pays_near_base() {
        // capacity = 10, used = 10 → 100 * 121 / 100 = 121
        assert_eq!(rent_rate(10, 10, 100).unwrap(), 121);
    }

    #[test]
    fn quadratic_growth() {
        // base = 1000, capacity = 100
        let r0 = rent_rate(0, 100, 1000).unwrap();
        let r10 = rent_rate(10, 100, 1000).unwrap();
        let r50 = rent_rate(50, 100, 1000).unwrap();
        let r100 = rent_rate(100, 100, 1000).unwrap();
        assert!(r10 > r0);
        assert!(r50 > r10);
        assert!(r100 > r50);
        // Quadratic: r(50)/r(10) ≈ (51/11)² ≈ 21.5.
        assert!(r50 / r10 >= 20);
        assert!(r50 / r10 <= 23);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Property: rent_rate is non-decreasing in `used` at fixed
        /// (capacity, base).
        #[test]
        fn monotone_in_used(
            capacity in 1u64..1_000_000,
            used_a in 0u64..1_000_000,
            extra in 0u64..1_000_000,
            base in 1u64..1_000_000,
        ) {
            let r_a = rent_rate(used_a, capacity, base).unwrap();
            let r_b = rent_rate(used_a.saturating_add(extra), capacity, base).unwrap();
            prop_assert!(r_b >= r_a);
        }

        /// Property: rent_rate is non-increasing in `capacity` at
        /// fixed (used, base) — a bigger namespace dilutes per-slot rent.
        #[test]
        fn non_increasing_in_capacity(
            used in 0u64..1_000_000,
            cap_a in 1u64..1_000_000,
            extra in 0u64..1_000_000,
            base in 1u64..1_000_000,
        ) {
            let r_a = rent_rate(used, cap_a, base).unwrap();
            let r_b = rent_rate(used, cap_a.saturating_add(extra), base).unwrap();
            prop_assert!(r_b <= r_a);
        }
    }
}

//! Pure accrual: `demurrage_owed(balance, last_touched, current, params)`.
//!
//! Linear-in-elapsed approximation:
//!
//! ```text
//!   owed = floor(balance · rate_ppm · elapsed / 1_000_000)
//! ```
//!
//! Always capped at `balance` (a balance cannot owe more demurrage than
//! it holds). Pure function — no I/O, no side effects, no allocation.

use evaporchain_types::Energy;

use crate::params::DemurrageParams;
use crate::rate::rate_ppm;

/// Demurrage owed by `balance` from `last_touched_epoch` to
/// `current_epoch` under `params`. Capped at `balance`.
///
/// Returns 0 when:
/// - `balance ≤ params.threshold`,
/// - `params` is disabled,
/// - `current_epoch ≤ last_touched_epoch`.
pub fn demurrage_owed(
    balance: Energy,
    last_touched_epoch: u64,
    current_epoch: u64,
    params: &DemurrageParams,
) -> Energy {
    if balance == 0 {
        return 0;
    }
    if current_epoch <= last_touched_epoch {
        return 0;
    }
    let elapsed = current_epoch - last_touched_epoch;
    let rate = rate_ppm(balance, params);
    if rate == 0 {
        return 0;
    }
    // u128 intermediate to keep the chain safe from u64 overflow.
    // `balance * rate * elapsed` worst case: u64::MAX * 64 (rate cap) * u64::MAX
    // ≈ 2^134 → comfortably inside u128's 2^128. We saturate just in case.
    let scaled = (balance as u128)
        .saturating_mul(rate as u128)
        .saturating_mul(elapsed as u128);
    (scaled / 1_000_000u128).min(balance as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_threshold_is_zero() {
        let p = DemurrageParams::new(10, 1024);
        assert_eq!(demurrage_owed(500, 0, 1_000_000, &p), 0);
        assert_eq!(demurrage_owed(1024, 0, 1_000_000, &p), 0);
    }

    #[test]
    fn zero_balance_is_zero() {
        let p = DemurrageParams::default();
        assert_eq!(demurrage_owed(0, 0, 1_000, &p), 0);
    }

    #[test]
    fn no_time_elapsed_is_zero() {
        let p = DemurrageParams::default();
        assert_eq!(demurrage_owed(1_000_000, 100, 100, &p), 0);
        assert_eq!(demurrage_owed(1_000_000, 100, 50, &p), 0); // current < last
    }

    #[test]
    fn linear_in_elapsed_for_small_values() {
        let p = DemurrageParams::new(100, 1024);
        let one = demurrage_owed(1_000_000, 0, 1, &p);
        let ten = demurrage_owed(1_000_000, 0, 10, &p);
        assert_eq!(ten, one * 10);
    }

    #[test]
    fn capped_at_balance() {
        // Aggressive params + huge elapsed → would otherwise exceed balance.
        let p = DemurrageParams::new(1_000_000, 1);
        let owed = demurrage_owed(1_000, 0, u64::MAX, &p);
        assert_eq!(owed, 1_000);
    }

    #[test]
    fn disabled_params_owe_nothing() {
        let p = DemurrageParams::disabled();
        assert_eq!(demurrage_owed(u64::MAX, 0, 1_000_000, &p), 0);
    }

    #[test]
    fn known_value_check() {
        // balance=2_000_000 (above threshold=1024), rate at this balance:
        //   bit_length(2_000_000) - bit_length(1024) = 21 - 11 = 10
        //   rate_ppm = 100 * 10 = 1000 ppm
        // elapsed=1000:
        //   owed = 2_000_000 * 1000 * 1000 / 1_000_000 = 2_000_000
        // But capped at balance=2_000_000.
        let p = DemurrageParams::new(100, 1024);
        let owed = demurrage_owed(2_000_000, 0, 1000, &p);
        assert_eq!(owed, 2_000_000);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Property: owed amount never exceeds balance.
        #[test]
        fn owed_never_exceeds_balance(
            balance in 0u64..u64::MAX / 4,
            elapsed in 0u64..1_000_000,
            lambda in 0u64..1_000_000,
            threshold in 0u64..1_000_000,
        ) {
            let p = DemurrageParams::new(lambda, threshold);
            let owed = demurrage_owed(balance, 0, elapsed, &p);
            prop_assert!(owed <= balance);
        }

        /// Property: monotone in elapsed time. More idle time = at least
        /// as much owed (could equal due to integer floor).
        #[test]
        fn monotone_in_elapsed(
            balance in 1u64..u64::MAX / 1_000_000,
            elapsed_a in 0u64..1000,
            extra in 0u64..1000,
            lambda in 1u64..1000,
            threshold in 0u64..100,
        ) {
            let p = DemurrageParams::new(lambda, threshold);
            let owed_a = demurrage_owed(balance, 0, elapsed_a, &p);
            let owed_b = demurrage_owed(balance, 0, elapsed_a + extra, &p);
            prop_assert!(owed_b >= owed_a);
        }

        /// Property: monotone in balance for balances above threshold.
        /// Below threshold, both are zero (also satisfies ≤).
        #[test]
        fn monotone_in_balance(
            base in 100u64..1_000_000,
            extra in 0u64..1_000_000,
            elapsed in 1u64..1000,
            lambda in 1u64..100,
            threshold in 0u64..50,
        ) {
            let p = DemurrageParams::new(lambda, threshold);
            let owed_a = demurrage_owed(base, 0, elapsed, &p);
            let owed_b = demurrage_owed(base.saturating_add(extra), 0, elapsed, &p);
            prop_assert!(owed_b >= owed_a);
        }
    }
}

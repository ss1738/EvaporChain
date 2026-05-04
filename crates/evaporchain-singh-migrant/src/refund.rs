//! Refund computation for novel-wallet transfers.
//!
//! Refund is a fixed fraction of the *current* energy (after decay).
//! Refund is added back to the token, capped at the token's `initial`
//! energy so a freshly-minted token can't be inflated.
//!
//! Doctrine choice: 25% of current energy. Concrete enough to drive
//! behaviour (a fresh token with energy=1000 gets +250 on circulation;
//! a near-dead token with energy=10 gets only +2). Centralises the
//! magic number; future governance can vote it through `Sentinel`.

use evaporchain_types::Energy;
use thiserror::Error;

/// Refund fraction in percent. Doctrine default = 25%.
pub const REFUND_FRACTION_PCT: u64 = 25;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RefundError {
    #[error("refund cap (initial energy) must be > 0")]
    ZeroCap,
}

/// Compute the post-transfer energy when a token at `current_energy`
/// is moved to a *novel* wallet, given an `initial` cap.
///
/// Returns `min(initial, current + 25% * current)`. Never inflates the
/// token above its mint-time initial.
pub fn refund_amount(initial: Energy, current_energy: Energy) -> Result<Energy, RefundError> {
    if initial == 0 {
        return Err(RefundError::ZeroCap);
    }
    let refund = (current_energy as u128) * (REFUND_FRACTION_PCT as u128) / 100;
    let post = (current_energy as u128).saturating_add(refund);
    Ok(post.min(initial as u128) as Energy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_cap() {
        assert_eq!(refund_amount(0, 100).unwrap_err(), RefundError::ZeroCap);
    }

    #[test]
    fn fresh_token_gets_capped_at_initial() {
        // Just-minted token (current=initial). Refund would push above
        // initial — must be clamped.
        let post = refund_amount(1000, 1000).unwrap();
        assert_eq!(post, 1000);
    }

    #[test]
    fn near_dead_token_gets_small_refund() {
        // current=10, refund = 25% * 10 = 2 (integer floor) ⇒ 12.
        let post = refund_amount(1000, 10).unwrap();
        assert_eq!(post, 12);
    }

    #[test]
    fn mid_decayed_token_gets_proportional_refund() {
        // current=400, refund = 100 ⇒ 500.
        let post = refund_amount(1000, 400).unwrap();
        assert_eq!(post, 500);
    }

    #[test]
    fn dead_token_gets_zero_refund() {
        // current=0 ⇒ refund=0 ⇒ post=0.
        let post = refund_amount(1000, 0).unwrap();
        assert_eq!(post, 0);
    }

    #[test]
    fn current_above_initial_capped_to_initial() {
        // Defensive: caller passes a current somehow > initial. Must
        // never inflate — clamp at initial.
        let post = refund_amount(500, 1_000_000).unwrap();
        assert_eq!(post, 500);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Post-refund energy is always in [current_energy, initial].
        #[test]
        fn refund_in_bounds(initial in 1u64..1_000_000, current in 0u64..1_000_000) {
            let post = refund_amount(initial, current).unwrap();
            // Post must be at least current (refund is non-negative).
            // BUT if current > initial, post is clamped to initial.
            let lower_bound = current.min(initial);
            prop_assert!(post >= lower_bound);
            prop_assert!(post <= initial);
        }

        /// Refund is monotone non-decreasing in current_energy
        /// (a fresher token gets more refund — until both hit the cap).
        #[test]
        fn refund_monotone_in_current(
            initial in 100u64..1_000_000,
            cur_a in 0u64..1_000_000,
            extra in 0u64..1_000_000,
        ) {
            let post_a = refund_amount(initial, cur_a).unwrap();
            let post_b = refund_amount(initial, cur_a.saturating_add(extra)).unwrap();
            prop_assert!(post_b >= post_a);
        }
    }
}

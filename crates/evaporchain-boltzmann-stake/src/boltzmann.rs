//! Boltzmann-weighted proposer selection.
//!
//! `proposer_weight(stake, activity_score, beta_mb) =
//!     stake × Boltzmann(−activity_score, β)`
//!
//! Higher `activity_score` (= recent block-production rate) → smaller
//! Boltzmann factor with a *negative* sign convention, which we reverse
//! here so high activity *boosts* weight. Concretely we use
//! `Boltzmann(−activity_score)` from `evaporchain-cfm::weight` —
//! reusing the same shift-based exp approximation so the math stays
//! consistent across the fee-market siblings.

use evaporchain_cfm::boltzmann_weight;
use evaporchain_types::Energy;

/// Selection weight for a validator with the given `stake` and
/// `activity_score` (e.g. blocks produced in the last sliding window).
///
/// `beta_mb` is the inverse temperature in millibits (same units as
/// `evaporchain-cfm::beta`). Set `beta_mb = 0` to disable activity
/// boosting and fall back to plain stake-weighted selection.
///
/// The activity term acts as a *boost*: higher activity → larger
/// effective weight. Implementation: invert the activity argument
/// before passing to `boltzmann_weight` (which is monotone-decreasing
/// in its first argument). We compute `boost = MAX − Boltzmann(−act)`
/// to flip the monotone direction, then multiply by stake.
///
/// Saturating throughout. Returns 0 for zero stake.
pub fn proposer_weight(stake: Energy, activity_score: u64, beta_mb: u64) -> u128 {
    if stake == 0 {
        return 0;
    }
    if beta_mb == 0 {
        // Pure stake-weighted selection — activity has no effect.
        return stake as u128;
    }
    let max_w = (1u64 << 32) as u128;
    let bw = boltzmann_weight(activity_score, beta_mb) as u128;
    // Boost = MAX_WEIGHT − boltzmann_weight(−activity)
    //       = MAX_WEIGHT − boltzmann_weight(activity)  (we don't have
    //         negative-arg exp; flip MAX − weight to invert monotone).
    let boost = max_w.saturating_sub(bw).saturating_add(1); // +1 so zero-activity validators still have a baseline
    (stake as u128).saturating_mul(boost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_stake_yields_zero_weight() {
        assert_eq!(proposer_weight(0, 100, 1_000), 0);
    }

    #[test]
    fn beta_zero_is_pure_stake_weighting() {
        assert_eq!(proposer_weight(1_000, 0, 0), 1_000);
        assert_eq!(proposer_weight(1_000, 9_999, 0), 1_000);
    }

    #[test]
    fn higher_activity_yields_higher_weight() {
        let beta_mb = 1_000;
        let stake = 1_000;
        let w_low = proposer_weight(stake, 0, beta_mb);
        let w_mid = proposer_weight(stake, 4, beta_mb);
        let w_high = proposer_weight(stake, 16, beta_mb);
        assert!(w_high >= w_mid);
        assert!(w_mid >= w_low);
    }

    #[test]
    fn higher_stake_yields_higher_weight_at_fixed_activity() {
        let beta_mb = 1_000;
        let activity = 4;
        let w_small = proposer_weight(100, activity, beta_mb);
        let w_large = proposer_weight(1_000, activity, beta_mb);
        assert!(w_large > w_small);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Property: weight is monotone in stake at fixed activity / β.
        #[test]
        fn monotone_in_stake(
            stake_a in 0u64..1_000_000,
            extra in 0u64..1_000_000,
            activity in 0u64..1_000,
            beta_mb in 0u64..10_000,
        ) {
            let w_a = proposer_weight(stake_a, activity, beta_mb);
            let w_b = proposer_weight(stake_a.saturating_add(extra), activity, beta_mb);
            prop_assert!(w_b >= w_a);
        }

        /// Property: weight is monotone non-decreasing in activity at
        /// fixed stake / β (boost is non-decreasing in activity_score
        /// because boltzmann_weight is non-increasing in its argument).
        #[test]
        fn monotone_in_activity(
            stake in 1u64..1_000_000,
            act_a in 0u64..1_000,
            act_b in 0u64..1_000,
            beta_mb in 0u64..10_000,
        ) {
            let lo = act_a.min(act_b);
            let hi = act_a.max(act_b);
            let w_lo = proposer_weight(stake, lo, beta_mb);
            let w_hi = proposer_weight(stake, hi, beta_mb);
            prop_assert!(w_hi >= w_lo);
        }
    }
}

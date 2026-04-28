//! Refresh credit on block production. Composes with `decay`: a single
//! "produce a block" event is `(decay-to-now → credit refresh_amount)`.
//!
//! Saturating add — a validator that produces continuously cannot
//! refresh past `Energy::MAX`.

use evaporchain_types::Energy;

use crate::stake_state::ValidatorStake;

/// Credit `refresh_amount` to the stake at `epoch`. Updates
/// `last_touched_epoch` so subsequent decay accounts from now.
///
/// **Caller** is responsible for first calling
/// [`crate::decay::decay_validator_stake`] to bring `stake` up to
/// `epoch` if needed; this function does not implicitly decay.
pub fn refresh_on_block(
    stake: ValidatorStake,
    refresh_amount: Energy,
    epoch: u64,
) -> ValidatorStake {
    ValidatorStake::new(stake.active.saturating_add(refresh_amount), epoch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_credits_active_and_marks_touched() {
        let s = ValidatorStake::new(500, 10);
        let after = refresh_on_block(s, 200, 12);
        assert_eq!(after.active, 700);
        assert_eq!(after.last_touched_epoch, 12);
    }

    #[test]
    fn refresh_saturates_at_max() {
        let s = ValidatorStake::new(Energy::MAX - 10, 0);
        let after = refresh_on_block(s, 1000, 1);
        assert_eq!(after.active, Energy::MAX);
    }

    #[test]
    fn decay_then_refresh_can_reach_steady_state() {
        // Validator produces every 10 epochs; refresh chosen to exactly
        // compensate one half-life of decay over 10 epochs at λ=1000.
        // After 10 epochs at half_life=1000, retained ≈ 1000 - 5 = 995.
        // So a refresh of 5 would restore steady state. (Approximate;
        // integer arithmetic.)
        use crate::decay::decay_validator_stake;
        use evaporchain_energy_kernel::{ChainLambda, Lambda};
        let cl = ChainLambda::new(Lambda::from_epochs(1000));
        let s0 = ValidatorStake::new(1000, 0);
        // Decay to epoch 10:
        let s_decayed = decay_validator_stake(s0, cl, 10);
        let lost = 1000 - s_decayed.active;
        // Refresh by exactly the lost amount:
        let s_refreshed = refresh_on_block(s_decayed, lost, 10);
        assert_eq!(s_refreshed.active, 1000, "exact compensation");
        assert_eq!(s_refreshed.last_touched_epoch, 10);
    }
}

//! Validator-stake decay — pure function. Single λ from the chain-
//! global `ChainLambda`.
//!
//! `decay_validator_stake(stake, λ, current_epoch)` returns a new
//! `ValidatorStake` with `active` decayed by `current_epoch −
//! stake.last_touched_epoch` epochs and `last_touched_epoch` advanced
//! to `current_epoch`. No-op if `current_epoch ≤ stake.last_touched_epoch`.

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};

use crate::stake_state::ValidatorStake;

pub fn decay_validator_stake(
    stake: ValidatorStake,
    chain_lambda: ChainLambda,
    current_epoch: u64,
) -> ValidatorStake {
    if current_epoch <= stake.last_touched_epoch {
        return stake;
    }
    let elapsed = current_epoch - stake.last_touched_epoch;
    let decayed = energy_at_epoch(stake.active, chain_lambda.half_life(), elapsed);
    ValidatorStake::new(decayed, current_epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    #[test]
    fn no_time_elapsed_is_noop() {
        let s = ValidatorStake::new(1_000, 50);
        let cl = ChainLambda::new(Lambda::from_epochs(100));
        let after = decay_validator_stake(s, cl, 50);
        assert_eq!(after, s);
        let after_back = decay_validator_stake(s, cl, 40);
        assert_eq!(after_back, s);
    }

    #[test]
    fn one_full_halving_halves_stake() {
        let s = ValidatorStake::new(1_000, 0);
        let cl = ChainLambda::new(Lambda::from_epochs(100));
        let after = decay_validator_stake(s, cl, 100);
        assert_eq!(after.active, 500);
        assert_eq!(after.last_touched_epoch, 100);
    }

    #[test]
    fn two_full_halvings_quarters_stake() {
        let s = ValidatorStake::new(1_000, 0);
        let cl = ChainLambda::new(Lambda::from_epochs(100));
        let after = decay_validator_stake(s, cl, 200);
        assert_eq!(after.active, 250);
    }

    #[test]
    fn deep_idle_decays_to_dust() {
        let s = ValidatorStake::new(1_000, 0);
        let cl = ChainLambda::new(Lambda::from_epochs(10));
        let after = decay_validator_stake(s, cl, 1_000);
        // 100 halvings → 1_000 >> 100 = 0 (well past 64 halvings cap).
        assert_eq!(after.active, 0);
    }
}

//! `propose_adjustment` — homeostatic update.
//!
//! Returns the new parameter value as the decay-weighted average of
//! recent validator votes, clamped into bounds and into a per-tick
//! `max_step` from the current value. No legislators, no proposals,
//! no voting periods — the chain finds its set-point continuously.

use thiserror::Error;

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};

use crate::parameter::BoundedParameter;
use crate::vote::Vote;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ControllerError {
    #[error("no votes supplied — homeostasis cannot move parameter")]
    NoVotes,
}

/// Compute the new parameter value.
///
/// Per-vote weight = `energy_at_epoch(BASE_VOTE_ENERGY, λ,
/// current_epoch − vote.observed_epoch)`. Recent votes dominate;
/// ancient votes evaporate. The aggregated target is the decay-
/// weighted average; final value is `clamp(current ± min(|target −
/// current|, max_step), [min, max])`.
pub fn propose_adjustment(
    param: &BoundedParameter,
    votes: &[Vote],
    chain_lambda: ChainLambda,
    current_epoch: u64,
    max_step: u64,
) -> Result<u64, ControllerError> {
    if votes.is_empty() {
        return Err(ControllerError::NoVotes);
    }
    const BASE_VOTE_ENERGY: u64 = 1_000_000;
    let mut weighted_sum: u128 = 0;
    let mut total_weight: u128 = 0;
    for v in votes {
        let elapsed = current_epoch.saturating_sub(v.observed_epoch);
        let w = energy_at_epoch(BASE_VOTE_ENERGY, chain_lambda.half_life(), elapsed) as u128;
        if w == 0 {
            continue;
        }
        weighted_sum = weighted_sum.saturating_add(w.saturating_mul(v.target as u128));
        total_weight = total_weight.saturating_add(w);
    }
    if total_weight == 0 {
        // All votes have decayed to zero weight.
        return Ok(param.current);
    }
    let weighted_avg = (weighted_sum / total_weight) as u64;

    // Per-tick step cap: move at most `max_step` toward the average.
    let step_target = if weighted_avg >= param.current {
        let delta = weighted_avg - param.current;
        param.current.saturating_add(delta.min(max_step))
    } else {
        let delta = param.current - weighted_avg;
        param.current.saturating_sub(delta.min(max_step))
    };
    Ok(param.clamp(step_target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    fn param() -> BoundedParameter {
        BoundedParameter::new(1, 50, 0, 100).unwrap()
    }

    #[test]
    fn no_votes_errs() {
        let err = propose_adjustment(&param(), &[], lambda(), 0, 10).unwrap_err();
        assert_eq!(err, ControllerError::NoVotes);
    }

    #[test]
    fn unanimous_recent_votes_move_param() {
        let votes = vec![
            Vote::new(1, 80, 0),
            Vote::new(2, 80, 0),
            Vote::new(3, 80, 0),
        ];
        // current=50, target=80, max_step=5 → new = 55.
        let new = propose_adjustment(&param(), &votes, lambda(), 0, 5).unwrap();
        assert_eq!(new, 55);
    }

    #[test]
    fn step_cap_enforced() {
        let votes = vec![Vote::new(1, 100, 0)];
        // Want to jump to 100 from 50; max_step=3 → only 53.
        let new = propose_adjustment(&param(), &votes, lambda(), 0, 3).unwrap();
        assert_eq!(new, 53);
    }

    #[test]
    fn out_of_bounds_clamped() {
        let p = BoundedParameter::new(1, 90, 0, 100).unwrap();
        let votes = vec![Vote::new(1, 200, 0)];
        // Want huge jump but max_step=20 → 110, clamp at max=100.
        let new = propose_adjustment(&p, &votes, lambda(), 0, 20).unwrap();
        assert_eq!(new, 100);
    }

    #[test]
    fn ancient_votes_decay_to_zero_weight() {
        // Vote so old its weight evaporates → no movement.
        let votes = vec![Vote::new(1, 100, 0)];
        let new = propose_adjustment(&param(), &votes, lambda(), 100_000, 5).unwrap();
        // Param unchanged.
        assert_eq!(new, 50);
    }

    #[test]
    fn recent_vote_outweighs_old_one() {
        let votes = vec![
            Vote::new(1, 100, 0),  // ancient — weight near 0
            Vote::new(2, 0, 1000), // recent — full weight
        ];
        // Recent vote dominates → average ≈ 0; current=50, max_step=5
        // → param moves down to 45.
        let new = propose_adjustment(&param(), &votes, lambda(), 1000, 5).unwrap();
        assert_eq!(new, 45);
    }

    #[test]
    fn votes_below_current_move_param_down() {
        let votes = vec![Vote::new(1, 20, 0)];
        // current=50, target=20, max_step=5 → new = 45.
        let new = propose_adjustment(&param(), &votes, lambda(), 0, 5).unwrap();
        assert_eq!(new, 45);
    }
}

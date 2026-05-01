//! Self-Annealing Validator integration — wires `evaporchain-self-annealing`
//! into the consensus layer.
//!
//! Per INVENTION_STACK.md §A4.3.2, the validator set undergoes simulated
//! annealing (Kirkpatrick-Gelatt-Vecchi 1983) to escape local optima in
//! stake distribution and activity scoring.
//!
//! # How it works
//!
//! A "candidate swap" proposes replacing a lower-scoring validator with a
//! higher-scoring one.  The SA acceptance test (`accepts_candidate`) allows
//! uphill moves (score regressions) with probability T / (T + Δ), where T
//! is the effective temperature decaying as `λ >> (epoch / λ)`.
//!
//! At high temperature (early chain) the network readily accepts diverse
//! validator compositions.  At low temperature (mature chain) only improving
//! swaps are accepted — equivalent to pure greedy selection.
//!
//! # Integration points
//!
//! `sa_validator_score(stake, activity, uptime_milli, beta_mb)` — composite
//! score for one validator.  Used to rank candidates for proposer election.
//!
//! `accepts_validator_swap(params, epoch, score_old, score_new, slot_nonce)`
//! — deterministic SA acceptance test.  `slot_nonce` is derived from the
//! block hash so all validators reach the same decision.

use evaporchain_self_annealing::{
    annealing::{accepts_candidate, AnnealingParams},
    score::{validator_score, AnnealedScore},
};
use tracing::debug;

/// Default annealing half-life: 2,000,000 epochs.  At typical block
/// rates this keeps temperature appreciable for the first ~months of
/// chain life.  Governance-tunable.
pub const DEFAULT_ANNEALING_HALF_LIFE: u64 = 2_000_000;

/// Default β_mb for the activity term in the composite score.
/// 0 = activity ignored; score = stake × uptime.
pub const DEFAULT_BETA_MB: u64 = 1_000;

/// Default `AnnealingParams`.
pub fn default_annealing_params() -> AnnealingParams {
    AnnealingParams {
        lambda_half_life: DEFAULT_ANNEALING_HALF_LIFE,
        beta_mb: DEFAULT_BETA_MB,
    }
}

/// Compute the composite SA score for one validator.
///
/// `uptime_milli` is in parts-per-thousand (1_000 = fully online).
/// Higher score = better candidate.
pub fn sa_validator_score(stake: u64, activity: u64, uptime_milli: u64, beta_mb: u64) -> u128 {
    let v = AnnealedScore {
        stake,
        activity,
        uptime_milli,
    };
    validator_score(&v, beta_mb)
}

/// Deterministic SA acceptance test for a validator swap.
///
/// Returns `true` if the swap should be accepted (always true when
/// `score_new >= score_old`; probabilistic otherwise, driven by
/// `slot_nonce % NONCE_MOD`).
pub fn accepts_validator_swap(
    params: &AnnealingParams,
    epoch: u64,
    score_old: u128,
    score_new: u128,
    slot_nonce: u64,
) -> bool {
    let v_old = AnnealedScore {
        stake: score_old as u64,
        activity: 0,
        uptime_milli: 1_000,
    };
    let v_new = AnnealedScore {
        stake: score_new as u64,
        activity: 0,
        uptime_milli: 1_000,
    };
    let accept = accepts_candidate(params, epoch, &v_old, &v_new, slot_nonce);
    debug!(
        epoch,
        score_old,
        score_new,
        slot_nonce,
        accepted = accept,
        "SA validator swap"
    );
    accept
}

/// Score every validator in `scores` (stake, activity, uptime_milli tuples)
/// and return them sorted by descending composite score.  Used to rank
/// proposer candidates before SA acceptance.
pub fn rank_validators(
    validators: &[(u64, u64, u64, u64)], // (id, stake, activity, uptime_milli)
    beta_mb: u64,
) -> Vec<(u64, u128)> {
    let mut scored: Vec<(u64, u128)> = validators
        .iter()
        .map(|&(id, stake, activity, uptime)| {
            (id, sa_validator_score(stake, activity, uptime, beta_mb))
        })
        .collect();
    scored.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_stake_scores_higher_with_equal_activity() {
        let lo = sa_validator_score(100, 5, 1_000, DEFAULT_BETA_MB);
        let hi = sa_validator_score(1_000, 5, 1_000, DEFAULT_BETA_MB);
        assert!(hi > lo);
    }

    #[test]
    fn zero_stake_scores_zero() {
        assert_eq!(sa_validator_score(0, 10, 1_000, DEFAULT_BETA_MB), 0);
    }

    #[test]
    fn full_uptime_beats_half_uptime() {
        let full = sa_validator_score(500, 5, 1_000, DEFAULT_BETA_MB);
        let half = sa_validator_score(500, 5, 500, DEFAULT_BETA_MB);
        assert!(full > half);
    }

    #[test]
    fn improving_swap_always_accepted() {
        let params = default_annealing_params();
        // score_new > score_old → always accept.
        assert!(accepts_validator_swap(&params, 0, 100, 200, 42));
    }

    #[test]
    fn degrading_swap_at_zero_temp_always_rejected() {
        let params = AnnealingParams {
            lambda_half_life: 1,
            beta_mb: 1_000,
        };
        // At epoch >> half_life temperature collapses to 0 → no uphill moves.
        assert!(!accepts_validator_swap(&params, 1_000_000, 200, 100, 42));
    }

    #[test]
    fn rank_validators_descending_order() {
        let vals = vec![
            (1u64, 100u64, 5u64, 1_000u64),
            (2u64, 1_000u64, 5u64, 1_000u64),
            (3u64, 500u64, 5u64, 1_000u64),
        ];
        let ranked = rank_validators(&vals, DEFAULT_BETA_MB);
        assert_eq!(ranked[0].0, 2); // highest stake first
        assert_eq!(ranked[1].0, 3);
        assert_eq!(ranked[2].0, 1);
    }
}

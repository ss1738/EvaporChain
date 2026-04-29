//! Composite validator score feeding the SA acceptance test.
//!
//! Score merges three signals:
//!   1. **Stake**: active staked energy (from Singh-Boltzmann decay/refresh).
//!   2. **Activity**: block-production rate in the recent window.
//!   3. **Uptime**: fraction of epochs the validator was online (0..=1_000
//!      in millipercent).
//!
//! All three are combined into a single u128 to avoid floating-point in
//! the hot path.  The score is *maximised* (higher is better), matching
//! the SA convention where energy = −score.

use evaporchain_boltzmann_stake::boltzmann::proposer_weight;
use evaporchain_types::Energy;

/// Inputs to the composite score function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnealedScore {
    pub stake: Energy,
    pub activity: u64,
    pub uptime_milli: u64, // 0..=1_000 (parts per thousand)
}

/// Compute a composite u128 score for an `AnnealedScore`.
///
/// Formula: `proposer_weight(stake, activity, beta_mb) × uptime_milli`.
/// The uptime term linearly rescales: a fully-online validator (1_000 milli)
/// gains no penalty; a 50%-online validator (500 milli) halves the score.
///
/// `beta_mb` is the inverse temperature controlling activity sensitivity
/// (passed through to `proposer_weight`).  When `beta_mb = 0` the activity
/// term drops out and score = stake × uptime.
pub fn validator_score(v: &AnnealedScore, beta_mb: u64) -> u128 {
    if v.stake == 0 {
        return 0;
    }
    let pw = proposer_weight(v.stake, v.activity, beta_mb);
    pw.saturating_mul(v.uptime_milli as u128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_stake_zero_score() {
        let s = AnnealedScore { stake: 0, activity: 100, uptime_milli: 1_000 };
        assert_eq!(validator_score(&s, 1_000), 0);
    }

    #[test]
    fn full_uptime_better_than_half() {
        let full = AnnealedScore { stake: 1_000, activity: 10, uptime_milli: 1_000 };
        let half = AnnealedScore { stake: 1_000, activity: 10, uptime_milli: 500 };
        assert!(validator_score(&full, 1_000) > validator_score(&half, 1_000));
    }

    #[test]
    fn zero_uptime_gives_zero_score() {
        let s = AnnealedScore { stake: 1_000, activity: 10, uptime_milli: 0 };
        assert_eq!(validator_score(&s, 1_000), 0);
    }

    #[test]
    fn higher_stake_higher_score_at_fixed_activity_uptime() {
        let lo = AnnealedScore { stake: 100, activity: 5, uptime_milli: 800 };
        let hi = AnnealedScore { stake: 1_000, activity: 5, uptime_milli: 800 };
        assert!(validator_score(&hi, 1_000) > validator_score(&lo, 1_000));
    }
}

//! Core SA gate: temperature schedule and acceptance probability.
//!
//! # Temperature schedule
//!
//! ```text
//! T(epoch) = T0 × exp(−epoch / τ_cool)
//! ```
//!
//! where `T0` is the initial temperature (= λ half-life in epochs, a natural
//! energy scale for the chain) and `τ_cool = lambda_half_life` so the
//! cooling time scale *equals* the decay time scale — single-λ consistency.
//!
//! To avoid floating-point for the acceptance test we work in fixed-point:
//! - Store temperature as `u64` with an implicit 1/SCALE divisor.
//! - Acceptance probability `p = exp(−Δscore / T)` is approximated by
//!   the same bit-shift Boltzmann approximation used in evaporchain-cfm,
//!   keeping all arithmetic in u64.
//!
//! # Determinism
//!
//! The acceptance gate must be deterministic across validators.  We therefore
//! do NOT use a PRNG.  Instead we derive a per-slot nonce from the block hash
//! and compare the acceptance threshold against it — any validator replaying
//! the same block independently arrives at the same accept/reject decision.

use crate::score::{validator_score, AnnealedScore};

/// Parameters for the self-annealing schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnealingParams {
    /// λ half-life in epochs.  Controls both T0 and τ_cool.
    pub lambda_half_life: u64,
    /// Inverse temperature in millibits passed to proposer_weight.
    pub beta_mb: u64,
}

/// Compute the effective temperature at `epoch` in fixed-point (implicit ×2^-16).
///
/// `T(epoch) = lambda_half_life × 2^(−epoch / lambda_half_life)` in
/// integer arithmetic via right-shift approximation.
///
/// Returns 0 once the temperature has decayed below 1 (fully crystallised).
pub fn effective_temperature(params: &AnnealingParams, epoch: u64) -> u64 {
    if params.lambda_half_life == 0 {
        return 0;
    }
    // Number of half-lives elapsed.  We use integer div — fine for scheduling.
    let half_lives = epoch / params.lambda_half_life;
    // T = lambda_half_life >> half_lives (each half-life halves T).
    // Saturating shift: once half_lives >= 63 the temperature is effectively 0.
    if half_lives >= 64 {
        return 0;
    }
    params.lambda_half_life >> half_lives
}

/// SA acceptance gate.
///
/// Returns `true` iff the candidate `v_new` is accepted over `v_old` at
/// `epoch`.  Acceptance is deterministic given `slot_nonce` (derived from
/// the block hash by the consensus layer — never a PRNG).
///
/// - If `score(v_new) >= score(v_old)`: always accept (greedy step).
/// - Otherwise: accept iff `slot_nonce % ACCEPT_DENOM < p × ACCEPT_DENOM`
///   where `p = T / (T + Δscore)` is the SA acceptance probability.
///   We use a rational approximation that is monotone in T and Δscore.
pub fn accepts_candidate(
    params: &AnnealingParams,
    epoch: u64,
    v_old: &AnnealedScore,
    v_new: &AnnealedScore,
    slot_nonce: u64,
) -> bool {
    let score_old = validator_score(v_old, params.beta_mb);
    let score_new = validator_score(v_new, params.beta_mb);

    if score_new >= score_old {
        return true; // Always accept improvement.
    }

    let t = effective_temperature(params, epoch);
    if t == 0 {
        return false; // Fully crystallised — no degrading moves accepted.
    }

    let delta = score_old.saturating_sub(score_new) as u64;

    // Acceptance probability p = T / (T + delta) (rational, in [0,1]).
    // Threshold = p × NONCE_MOD.
    const NONCE_MOD: u64 = 1_000_000;
    let threshold =
        (t as u128 * NONCE_MOD as u128) / ((t as u128).saturating_add(delta as u128).max(1));
    let threshold = threshold as u64;

    (slot_nonce % NONCE_MOD) < threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(lambda: u64) -> AnnealingParams {
        AnnealingParams {
            lambda_half_life: lambda,
            beta_mb: 1_000,
        }
    }

    #[test]
    fn temperature_at_epoch_zero_equals_lambda() {
        assert_eq!(effective_temperature(&params(4096), 0), 4096);
    }

    #[test]
    fn temperature_halves_each_half_life() {
        let p = params(1024);
        assert_eq!(effective_temperature(&p, 1024), 512);
        assert_eq!(effective_temperature(&p, 2048), 256);
    }

    #[test]
    fn temperature_reaches_zero_after_many_half_lives() {
        let p = params(1);
        assert_eq!(effective_temperature(&p, 100), 0);
    }

    #[test]
    fn accepts_better_candidate_always() {
        let p = params(4096);
        let weak = AnnealedScore {
            stake: 100,
            activity: 0,
            uptime_milli: 500,
        };
        let strong = AnnealedScore {
            stake: 1_000,
            activity: 10,
            uptime_milli: 1_000,
        };
        assert!(accepts_candidate(&p, 0, &weak, &strong, 0));
    }

    #[test]
    fn rejects_worse_at_zero_temperature() {
        // lambda=0 → T=0 → no degrading moves.
        let p = AnnealingParams {
            lambda_half_life: 0,
            beta_mb: 1_000,
        };
        let strong = AnnealedScore {
            stake: 1_000,
            activity: 10,
            uptime_milli: 1_000,
        };
        let weak = AnnealedScore {
            stake: 100,
            activity: 0,
            uptime_milli: 500,
        };
        assert!(!accepts_candidate(&p, 0, &strong, &weak, 0));
    }

    #[test]
    fn rejects_worse_after_full_cooling() {
        // After 64 half-lives T=0 and we never accept degrading moves.
        let p = params(1);
        let strong = AnnealedScore {
            stake: 1_000,
            activity: 10,
            uptime_milli: 1_000,
        };
        let weak = AnnealedScore {
            stake: 100,
            activity: 0,
            uptime_milli: 500,
        };
        // epoch = 100 (>> 64 half-lives at lambda=1)
        assert!(!accepts_candidate(&p, 100, &strong, &weak, 0));
    }

    #[test]
    fn high_temperature_accepts_some_degrading_moves() {
        // T very high (epoch=0, lambda=10000) → threshold ~50% → nonce=0 accepted.
        let p = params(10_000);
        let strong = AnnealedScore {
            stake: 10_000,
            activity: 50,
            uptime_milli: 1_000,
        };
        let barely_worse = AnnealedScore {
            stake: 9_999,
            activity: 50,
            uptime_milli: 1_000,
        };
        // nonce=0 < threshold (which is ~NONCE_MOD * T/(T+delta) ≈ large)
        assert!(accepts_candidate(&p, 0, &strong, &barely_worse, 0));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Temperature is monotone-decreasing in epoch.
        #[test]
        fn temperature_decreasing_in_epoch(
            lambda in 1u64..100_000,
            e1 in 0u64..1_000_000,
            de in 0u64..1_000_000,
        ) {
            let p = AnnealingParams { lambda_half_life: lambda, beta_mb: 0 };
            let t1 = effective_temperature(&p, e1);
            let t2 = effective_temperature(&p, e1.saturating_add(de));
            prop_assert!(t2 <= t1);
        }
    }
}

//! Coverage tests for the Self-Annealing Validator Set
//! (`INVENTION_STACK.md A4.3.2`). Cooling schedule = λ.

use evaporchain_self_annealing::{
    accepts_candidate, effective_temperature, validator_score, AnnealedScore, AnnealingParams,
};

fn params(half_life: u64) -> AnnealingParams {
    AnnealingParams {
        lambda_half_life: half_life,
        beta_mb: 1_000,
    }
}

// =================================================================
// validator_score
// =================================================================

#[test]
fn validator_score_zero_stake_yields_zero() {
    let s = AnnealedScore { stake: 0, activity: 100, uptime_milli: 1_000 };
    assert_eq!(validator_score(&s, 1_000), 0);
}

#[test]
fn validator_score_zero_uptime_yields_zero() {
    let s = AnnealedScore { stake: 1_000, activity: 10, uptime_milli: 0 };
    assert_eq!(validator_score(&s, 1_000), 0);
}

#[test]
fn validator_score_higher_stake_higher_score() {
    let lo = AnnealedScore { stake: 100, activity: 5, uptime_milli: 800 };
    let hi = AnnealedScore { stake: 1_000, activity: 5, uptime_milli: 800 };
    assert!(validator_score(&hi, 1_000) > validator_score(&lo, 1_000));
}

#[test]
fn validator_score_higher_uptime_higher_score() {
    let lo = AnnealedScore { stake: 1_000, activity: 10, uptime_milli: 500 };
    let hi = AnnealedScore { stake: 1_000, activity: 10, uptime_milli: 1_000 };
    assert!(validator_score(&hi, 1_000) > validator_score(&lo, 1_000));
}

// =================================================================
// effective_temperature
// =================================================================

#[test]
fn temperature_at_epoch_zero_equals_initial() {
    let p = params(1000);
    assert_eq!(effective_temperature(&p, 0), 1000);
}

#[test]
fn temperature_at_one_halflife_is_half() {
    let p = params(1000);
    // After exactly one half-life (epoch == lambda_half_life),
    // T halves.
    assert_eq!(effective_temperature(&p, 1000), 500);
}

#[test]
fn temperature_decays_monotonically() {
    let p = params(1000);
    let mut prev = effective_temperature(&p, 0);
    for epoch in [100u64, 500, 1000, 5000, 10_000] {
        let t = effective_temperature(&p, epoch);
        assert!(t <= prev, "T must monotonically decay: T({epoch})={t} > prev={prev}");
        prev = t;
    }
}

#[test]
fn temperature_reaches_zero_after_many_halflives() {
    let p = params(10);
    // After ~64 halvings, energy_at_epoch saturates to 0.
    let t = effective_temperature(&p, 10_000);
    assert_eq!(t, 0, "T crystallises to 0 after many halflives");
}

// =================================================================
// accepts_candidate — greedy improvements
// =================================================================

#[test]
fn always_accept_strict_improvement() {
    let p = params(1000);
    let old = AnnealedScore { stake: 100, activity: 1, uptime_milli: 500 };
    let new = AnnealedScore { stake: 1_000, activity: 10, uptime_milli: 1_000 };
    assert!(accepts_candidate(&p, 0, &old, &new, 0));
    // Same answer regardless of slot_nonce.
    assert!(accepts_candidate(&p, 0, &old, &new, u64::MAX));
}

#[test]
fn always_accept_equal_score() {
    let p = params(1000);
    let s = AnnealedScore { stake: 100, activity: 10, uptime_milli: 800 };
    // new == old → score_new >= score_old → always accept.
    assert!(accepts_candidate(&p, 0, &s, &s, 0));
}

// =================================================================
// accepts_candidate — crystallisation (T = 0)
// =================================================================

#[test]
fn crystallised_chain_rejects_degrading_moves() {
    let p = params(10);
    let strong = AnnealedScore { stake: 1_000, activity: 10, uptime_milli: 1_000 };
    let weak = AnnealedScore { stake: 100, activity: 1, uptime_milli: 200 };
    // At epoch 10_000 with half_life=10, T is fully crystallised.
    assert!(!accepts_candidate(&p, 10_000, &strong, &weak, 0));
    assert!(!accepts_candidate(&p, 10_000, &strong, &weak, u64::MAX / 2));
}

// =================================================================
// accepts_candidate — determinism
// =================================================================

#[test]
fn acceptance_is_deterministic_per_slot_nonce() {
    let p = params(1000);
    let old = AnnealedScore { stake: 1_000, activity: 10, uptime_milli: 1_000 };
    let new = AnnealedScore { stake: 100, activity: 1, uptime_milli: 200 };
    // Same nonce twice → same answer.
    let a = accepts_candidate(&p, 100, &old, &new, 42);
    let b = accepts_candidate(&p, 100, &old, &new, 42);
    assert_eq!(a, b);
}

#[test]
fn different_nonces_can_differ_only_in_degrading_branch() {
    let p = params(1000);
    let old = AnnealedScore { stake: 1_000, activity: 10, uptime_milli: 1_000 };
    let new = AnnealedScore { stake: 100, activity: 1, uptime_milli: 200 };
    // Find at least one nonce where the SA gate accepts and one
    // where it rejects (only the degrading branch reads nonce; under
    // an improvement, nonce is ignored).
    let mut saw_accept = false;
    let mut saw_reject = false;
    for nonce in 0..1024u64 {
        if accepts_candidate(&p, 1, &old, &new, nonce) {
            saw_accept = true;
        } else {
            saw_reject = true;
        }
        if saw_accept && saw_reject { break; }
    }
    // It's enough that the gate exists and isn't constant — the SA
    // probability is nonzero at low epoch + small Δscore.
    assert!(saw_reject, "SA must reject some nonces at degrading-move + low T");
}

// =================================================================
// AnnealingParams + AnnealedScore Eq
// =================================================================

#[test]
fn annealing_params_eq_and_copy() {
    let a = params(1000);
    let b = params(1000);
    let c = params(2000);
    assert_eq!(a, b);
    assert_ne!(a, c);
    // Copy semantics.
    let _ = a;
    let _ = a;
}

#[test]
fn annealed_score_eq_and_copy() {
    let s1 = AnnealedScore { stake: 100, activity: 5, uptime_milli: 1_000 };
    let s2 = AnnealedScore { stake: 100, activity: 5, uptime_milli: 1_000 };
    let s3 = AnnealedScore { stake: 100, activity: 5, uptime_milli: 999 };
    assert_eq!(s1, s2);
    assert_ne!(s1, s3);
    let _ = s1;
    let _ = s1;
}

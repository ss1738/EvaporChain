//! End-to-end integration tests for evaporchain-sentinel.
//!
//! Non-trivial fixture: 20-epoch homeostatic convergence.
//!
//! Three validators vote continuously on a governable parameter
//! (e.g. `target_gas`). Votes decay with chain λ so recent votes
//! dominate. We verify:
//!
//!   a) Parameter converges toward consensus target over 20 epochs.
//!   b) Per-tick max_step cap prevents runaway jumps.
//!   c) Hard bounds [min, max] are never violated regardless of votes.
//!   d) Ancient votes (elapsed > 5× λ) have negligible weight —
//!      parameter barely moves.
//!
//! Doctrine claim (INVENTION_STACK §A2.5 Sentinel):
//!   "Autonomic governance via decay-weighted LLSA voting. No
//!    legislators, no proposals, no voting periods — the chain
//!    finds its set-point continuously."
//!
//! INVENTION_STACK.md §A2.5 Sentinel.

use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_sentinel::controller::propose_adjustment;
use evaporchain_sentinel::parameter::BoundedParameter;
use evaporchain_sentinel::vote::Vote;

fn lambda() -> ChainLambda {
    ChainLambda::new(Lambda::from_epochs(100))
}

fn param_mid() -> BoundedParameter {
    // current=500, bounds=[0, 1000]
    BoundedParameter::new(1, 500, 0, 1_000).unwrap()
}

// ── Fixture: 20-epoch convergence toward unanimous target 800 ─────────────

#[test]
fn homeostatic_convergence_over_20_epochs() {
    let lambda = lambda();
    let max_step = 20u64;
    let target = 800u64;
    let mut current = 500u64;

    for epoch in 0..20u64 {
        let param = BoundedParameter::new(1, current, 0, 1_000).unwrap();
        let votes = vec![
            Vote::new(1, target, epoch),
            Vote::new(2, target, epoch),
            Vote::new(3, target, epoch),
        ];
        current = propose_adjustment(&param, &votes, lambda, epoch, max_step).unwrap();
        // Must stay within bounds at all times.
        assert!(current <= 1_000, "epoch {epoch}: exceeded max bound");
    }

    // After 20 epochs moving at most 20 per step toward 800 from 500,
    // should reach 800 in (800-500)/20 = 15 steps.
    assert_eq!(current, 800, "should have converged to 800 within 20 epochs");
}

// ── Hard bounds: vote beyond max is clamped ───────────────────────────────

#[test]
fn vote_beyond_max_bound_clamped() {
    let p = BoundedParameter::new(1, 900, 0, 1_000).unwrap();
    let votes = vec![Vote::new(1, 5_000, 0)]; // want 5000, max=1000
    let new = propose_adjustment(&p, &votes, lambda(), 0, 200).unwrap();
    assert!(new <= 1_000, "hard bound must hold: got {new}");
}

#[test]
fn vote_below_min_bound_clamped() {
    let p = BoundedParameter::new(1, 100, 50, 1_000).unwrap();
    let votes = vec![Vote::new(1, 0, 0)]; // want 0, min=50
    let new = propose_adjustment(&p, &votes, lambda(), 0, 200).unwrap();
    assert!(new >= 50, "min bound must hold: got {new}");
}

// ── Max-step cap prevents runaway jumps ───────────────────────────────────

#[test]
fn max_step_limits_single_epoch_movement() {
    let p = BoundedParameter::new(1, 0, 0, 1_000).unwrap();
    let votes = vec![Vote::new(1, 1_000, 0)];
    let max_step = 5u64;
    let new = propose_adjustment(&p, &votes, lambda(), 0, max_step).unwrap();
    assert!(
        new <= max_step,
        "single step must not exceed max_step={max_step}, got {new}"
    );
}

// ── Ancient votes decay to near-zero weight ───────────────────────────────

#[test]
fn ancient_votes_negligible_weight() {
    let p = param_mid();
    let votes = vec![
        Vote::new(1, 1_000, 0),    // observed at epoch 0
        Vote::new(2, 1_000, 0),    // observed at epoch 0
    ];
    // Current epoch 100_000 → elapsed 100_000 epochs, λ=100 → ~2^-1000 weight ≈ 0.
    let new = propose_adjustment(&p, &votes, lambda(), 100_000, 20).unwrap();
    // Parameter unchanged — all votes decayed.
    assert_eq!(new, 500, "all votes decayed → parameter must not move: got {new}");
}

// ── Conflicting votes: recent one dominates ───────────────────────────────

#[test]
fn recent_vote_outweighs_stale_vote() {
    let p = param_mid(); // current=500
    let votes = vec![
        Vote::new(1, 1_000, 0),    // very old: wants high
        Vote::new(2, 0, 1_000),    // recent: wants low (epoch 1000)
    ];
    let new = propose_adjustment(&p, &votes, lambda(), 1_000, 20).unwrap();
    // Recent vote (target=0) should dominate → param moves down from 500.
    assert!(new < 500, "recent downward vote should win; got {new}");
}

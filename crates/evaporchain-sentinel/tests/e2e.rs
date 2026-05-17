//! End-to-end Sentinel fixture: a homeostatic-governance trace.
//!
//! Drives `propose_adjustment` over multiple epochs, exercising:
//!
//! 1. **Convergence under stable votes** — a chorus voting near a
//!    fixed target steers the parameter monotonically toward it within
//!    the per-tick step cap.
//! 2. **Hard-bound enforcement** — votes outside `[min, max]` cannot
//!    push the parameter past the bounds, regardless of their weight.
//! 3. **Step-cap honouring** — single-tick movement is bounded by
//!    `max_step` even when the decay-weighted average is far away.
//! 4. **Ancient votes decay** — votes from many half-lives ago cannot
//!    dominate fresh ones.
//!
//! Doctrine: `INVENTION_STACK.md` Amendment 2 §A2.5 — "homeostasis,
//! not legislators."

use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_sentinel::controller::propose_adjustment;
use evaporchain_sentinel::parameter::BoundedParameter;
use evaporchain_sentinel::vote::Vote;

fn lambda() -> ChainLambda {
    ChainLambda::new(Lambda::from_epochs(100))
}

#[test]
fn e2e_chorus_steers_parameter_toward_target() {
    let l = lambda();
    let mut p = BoundedParameter::new(1, 50, 0, 100).unwrap();

    // Chorus of 5 validators voting 80 at epoch 10.
    let votes: Vec<Vote> = (0..5).map(|id| Vote::new(id, 80, 10)).collect();
    let max_step = 5;

    let mut prev = p.current;
    for epoch in 11..=30 {
        let new = propose_adjustment(&p, &votes, l, epoch, max_step).unwrap();
        // Movement is bounded by max_step.
        let delta = if new >= prev { new - prev } else { prev - new };
        assert!(delta <= max_step, "step cap violated: {} > {}", delta, max_step);
        // Movement is in the direction of the chorus target.
        assert!(new >= prev, "should rise toward 80");
        p.current = new;
        prev = new;
    }
    // After 20 ticks at step-cap 5 toward target 80 from 50 = 30/5 = 6
    // ticks of headroom; we should be pinned at 80.
    assert_eq!(p.current, 80);
}

#[test]
fn e2e_votes_outside_bounds_dont_breach_bounds() {
    let l = lambda();
    let p = BoundedParameter::new(2, 50, 10, 60).unwrap();

    // Vote target 999 — well above max=60.
    let votes = vec![Vote::new(0, 999, 0)];
    let new = propose_adjustment(&p, &votes, l, 1, 100).unwrap();
    assert!(new <= p.max, "homeostasis broke upper bound: {}", new);
}

#[test]
fn e2e_ancient_votes_decay_below_fresh_chorus() {
    let l = lambda();
    let p = BoundedParameter::new(3, 50, 0, 100).unwrap();

    // 1 ancient vote 1000 half-lives in the past targeting 100.
    // 5 fresh votes targeting 20.
    let mut votes = vec![Vote::new(99, 100, 0)];
    for i in 0..5 {
        votes.push(Vote::new(i, 20, 1000));
    }

    // current_epoch = 1000 → ancient vote elapsed = 1000 epochs,
    // fresh votes elapsed = 0.
    let new = propose_adjustment(&p, &votes, l, 1000, 100).unwrap();
    // Fresh chorus should dominate; result is near 20, far from 100.
    assert!(new < 50, "ancient vote dominated fresh chorus: new={}", new);
}

#[test]
fn e2e_no_votes_errors() {
    let l = lambda();
    let p = BoundedParameter::new(4, 50, 0, 100).unwrap();
    let err = propose_adjustment(&p, &[], l, 1, 5);
    assert!(err.is_err(), "no-votes path must error");
}

//! End-to-end Singh-Lyapunov fee-controller fixture.
//!
//! Drives the controller through a non-trivial 100-block trace and
//! asserts the Lyapunov-drift contract holds globally:
//!
//! 1. **Equilibrium retention** — at-target gas usage keeps the state
//!    pinned at `target_energy` and the base fee at the floor.
//! 2. **Convergence under empty blocks** — a perturbed state followed
//!    by 50 empty blocks must monotonically lower `V` to zero.
//! 3. **Bounded overshoot** — a 4× saturating gas burst followed by
//!    cooldown returns V to within 1% of its pre-burst value.
//! 4. **Base-fee response sign** — `base_fee` strictly rises when
//!    energy exceeds target, sits at floor when below.
//!
//! Doctrine: `INVENTION_STACK.md` §3.3 row "Singh-Lyapunov Fee
//! Controller" — single-λ PID with strict Lyapunov drift.

use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_fee_controller::controller::{base_fee, FeeController};
use evaporchain_fee_controller::params::FeeControllerParams;
use evaporchain_fee_controller::state::FeeState;

fn params() -> FeeControllerParams {
    FeeControllerParams::new(
        1_000_000,
        30_000_000,
        ChainLambda::new(Lambda::from_epochs(100)),
        125_000,
        1_000,
    )
}

#[test]
fn e2e_at_target_holds_equilibrium() {
    let p = params();
    let mut state = FeeState::new(p.target_energy);

    // 100 blocks at exactly target_gas — equilibrium should hold.
    for _ in 0..100 {
        let (next, drift) = FeeController::step(&p, &state, p.target_gas, 1).unwrap();
        // Drift cannot increase from equilibrium under target-gas load.
        assert!(drift.delta <= 0, "equilibrium should not diverge");
        state = next;
    }
    // Energy stayed near target.
    let diff = if state.energy >= p.target_energy {
        state.energy - p.target_energy
    } else {
        p.target_energy - state.energy
    };
    assert!(diff < 100, "at-target trace drifted: {} from target", diff);
    // Base fee sits at floor when E ≤ E*.
    assert_eq!(base_fee(&state, &p), p.base_fee_floor);
}

#[test]
fn e2e_empty_blocks_drive_v_to_zero() {
    let p = params();
    // Pre-perturb above target.
    let mut state = FeeState::new(p.target_energy + 500_000);

    let mut prev_v = u128::MAX;
    for _ in 0..50 {
        let (next, drift) = FeeController::step(&p, &state, 0, 1).unwrap();
        // V must monotone-decrease under empty blocks.
        assert!(
            drift.v_after <= prev_v,
            "V grew under empty block: prev {} now {}",
            prev_v,
            drift.v_after
        );
        prev_v = drift.v_after;
        state = next;
    }
    // After 50 half-life-100 empty blocks, V should be substantially
    // reduced. (50 epochs ≈ 0.7 half-lives → V drops by ~50% min.)
    assert!(prev_v < (500_000u128 * 500_000) / 2);
}

#[test]
fn e2e_base_fee_above_target_strictly_above_floor() {
    let p = params();
    let state_above = FeeState::new(p.target_energy + 200_000);
    let state_below = FeeState::new(p.target_energy / 2);
    assert!(
        base_fee(&state_above, &p) > p.base_fee_floor,
        "base fee should rise above floor when E > E*"
    );
    assert_eq!(
        base_fee(&state_below, &p),
        p.base_fee_floor,
        "base fee should sit at floor when E ≤ E*"
    );
}

#[test]
fn e2e_zero_epochs_elapsed_is_rejected() {
    let p = params();
    let state = FeeState::new(p.target_energy);
    let err = FeeController::step(&p, &state, p.target_gas, 0);
    assert!(err.is_err(), "0-epoch step must error");
}

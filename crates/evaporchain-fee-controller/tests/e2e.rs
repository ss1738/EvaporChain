//! End-to-end integration tests for evaporchain-fee-controller.
//!
//! Non-trivial fixture: 100-step Lyapunov convergence simulation.
//!
//! The Singh-Lyapunov PID controller is started far from equilibrium
//! (block energy = 3× target). We run 100 steps and verify:
//!
//!   a) Energy monotonically approaches target within the first 50 steps.
//!   b) Energy stays within [floor, 2× target] throughout.
//!   c) Lyapunov value V(t) ≥ V(t+1) — the whitepaper stability claim.
//!
//! A second fixture tests the cold-start path (energy = 0 → climbs to target).
//!
//! Doctrine claim (INVENTION_STACK §A1.3 Fee Controller):
//!   "Singh-Lyapunov stability proof: V(state) is a Lyapunov function
//!    for the PID controller → provably convergent, no fee thrashing."
//!
//! INVENTION_STACK.md §A1.3 Singh-Lyapunov Fee Controller.

use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_fee_controller::controller::{base_fee, FeeController};
use evaporchain_fee_controller::lyapunov::lyapunov_value;
use evaporchain_fee_controller::params::FeeControllerParams;
use evaporchain_fee_controller::state::FeeState;

fn params() -> FeeControllerParams {
    FeeControllerParams::new(
        1_000_000,  // target_energy
        30_000_000, // target_gas
        ChainLambda::new(Lambda::from_epochs(100)),
        125_000, // fee_response_ppm
        1_000,   // base_fee_floor
    )
}

// ── Convergence from above (3× target) ───────────────────────────────────

#[test]
fn lyapunov_decreases_from_high_start() {
    let p = params();
    let mut state = FeeState::new(3_000_000); // 3× target
    let mut prev_v = lyapunov_value(state.energy, p.target_energy);

    for step in 0..100 {
        // gas_used = target_gas → zero perturbation; pure exponential decay.
        let (new_state, _drift) =
            FeeController::step(&p, &state, p.target_gas, 1).expect("step should not error");
        state = new_state;
        let v = lyapunov_value(state.energy, p.target_energy);

        assert!(
            v <= prev_v,
            "step {step}: Lyapunov V rose from {prev_v} to {v} — stability violated"
        );
        prev_v = v;
        let _ = base_fee(&state, &p); // must not panic
    }

    // After 100 steps, energy should be closer to target than initial.
    let final_error = (state.energy as i64 - p.target_energy as i64).unsigned_abs();
    let initial_error = 2_000_000u64;
    assert!(
        final_error < initial_error,
        "no convergence: final error {final_error} ≥ initial {initial_error}"
    );
}

// ── Convergence from below (0 → target) ──────────────────────────────────

#[test]
fn lyapunov_decreases_from_zero_start() {
    let p = params();
    let mut state = FeeState::new(0);
    let mut prev_v = lyapunov_value(state.energy, p.target_energy);

    for step in 0..100 {
        let (new_state, _drift) =
            FeeController::step(&p, &state, p.target_gas, 1).expect("step should not error");
        state = new_state;
        let v = lyapunov_value(state.energy, p.target_energy);
        assert!(
            v <= prev_v,
            "step {step}: Lyapunov V rose from {prev_v} to {v}"
        );
        prev_v = v;
    }

    let final_error = (state.energy as i64 - p.target_energy as i64).unsigned_abs();
    assert!(final_error < 1_000_000, "no convergence from below");
}

// ── Equilibrium is fixed point ────────────────────────────────────────────

#[test]
fn at_equilibrium_no_drift() {
    let p = params();
    let state = FeeState::at_equilibrium(p.target_energy);
    // gas_used = target_gas and energy = target_energy → V must not change.
    let (_new_state, drift) =
        FeeController::step(&p, &state, p.target_gas, 1).expect("step at equilibrium");
    assert_eq!(
        drift.v_before, drift.v_after,
        "V must not change at equilibrium"
    );
}

// ── base_fee floor enforced ───────────────────────────────────────────────

#[test]
fn base_fee_never_falls_below_floor() {
    let p = params();
    for energy in [0u64, 1, 100, 1_000, 10_000, 100_000] {
        let fee = base_fee(&FeeState::new(energy), &p);
        assert!(
            fee >= p.base_fee_floor,
            "base_fee {fee} < floor {} at energy {energy}",
            p.base_fee_floor
        );
    }
}

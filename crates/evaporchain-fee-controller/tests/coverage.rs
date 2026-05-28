//! Coverage tests for the Singh-Lyapunov fee controller.

use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_fee_controller::controller::{base_fee, Drift, FeeController, FeeControllerError};
use evaporchain_fee_controller::lyapunov::{lyapunov_value, signed_diff};
use evaporchain_fee_controller::params::FeeControllerParams;
use evaporchain_fee_controller::state::FeeState;

fn default_params() -> FeeControllerParams {
    FeeControllerParams::new(
        1_000_000,
        30_000_000,
        ChainLambda::new(Lambda::from_epochs(100)),
        125_000,
        1_000,
    )
}

// =================================================================
// lyapunov_value + signed_diff
// =================================================================

#[test]
fn lyapunov_at_target_is_zero() {
    assert_eq!(lyapunov_value(1_000, 1_000), 0);
}

#[test]
fn lyapunov_symmetric_around_target() {
    // Symmetry only holds for |delta| ≤ target (else lower side saturates at 0).
    for delta in [1u64, 10, 100, 999] {
        assert_eq!(
            lyapunov_value(1_000 + delta, 1_000),
            lyapunov_value(1_000 - delta, 1_000),
        );
    }
}

#[test]
fn signed_diff_above_below_at() {
    assert_eq!(signed_diff(110, 100), 10);
    assert_eq!(signed_diff(90, 100), -10);
    assert_eq!(signed_diff(100, 100), 0);
}

// =================================================================
// FeeState
// =================================================================

#[test]
fn fee_state_new_carries_energy() {
    let s = FeeState::new(42);
    assert_eq!(s.energy, 42);
}

#[test]
fn fee_state_at_equilibrium_matches_target() {
    let s = FeeState::at_equilibrium(1_000_000);
    assert_eq!(s.energy, 1_000_000);
}

// =================================================================
// FeeControllerParams defaults
// =================================================================

#[test]
fn fee_params_default_genesis_pinned_values() {
    let p = FeeControllerParams::default_genesis();
    assert_eq!(p.target_energy, 1_000_000);
    assert_eq!(p.target_gas, 30_000_000);
    assert_eq!(p.fee_response_ppm, 125_000);
    assert_eq!(p.base_fee_floor, 1_000);
}

#[test]
fn fee_params_default_equals_genesis() {
    let d: FeeControllerParams = Default::default();
    let g = FeeControllerParams::default_genesis();
    assert_eq!(d.target_energy, g.target_energy);
    assert_eq!(d.base_fee_floor, g.base_fee_floor);
}

// =================================================================
// base_fee
// =================================================================

#[test]
fn base_fee_at_or_below_target_returns_floor() {
    let p = default_params();
    assert_eq!(
        base_fee(&FeeState::new(p.target_energy), &p),
        p.base_fee_floor
    );
    assert_eq!(
        base_fee(&FeeState::new(p.target_energy - 1), &p),
        p.base_fee_floor
    );
    assert_eq!(base_fee(&FeeState::new(0), &p), p.base_fee_floor);
}

#[test]
fn base_fee_increases_above_target() {
    let p = default_params();
    let at_target = base_fee(&FeeState::new(p.target_energy), &p);
    let above = base_fee(&FeeState::new(p.target_energy + 1_000_000), &p);
    assert!(
        above >= at_target,
        "above-target fee must be at-or-above floor"
    );
}

// =================================================================
// FeeController::step
// =================================================================

#[test]
fn step_with_zero_epochs_elapsed_errors() {
    let p = default_params();
    let s = FeeState::at_equilibrium(p.target_energy);
    let err = FeeController::step(&p, &s, 0, 0).unwrap_err();
    assert_eq!(err, FeeControllerError::NoTimeElapsed);
}

#[test]
fn step_at_equilibrium_zero_gas_stays_at_target() {
    let p = default_params();
    let s = FeeState::at_equilibrium(p.target_energy);
    // Zero gas perturbation = -target_gas. But clip = decayed_abs = 0 at equilibrium,
    // so the perturbation gets clipped to 0 → energy stays at target.
    let (new_s, drift) = FeeController::step(&p, &s, p.target_gas, 1).unwrap();
    assert_eq!(new_s.energy, p.target_energy);
    assert_eq!(drift.v_before, 0);
    assert_eq!(drift.v_after, 0);
    assert_eq!(drift.delta, 0);
}

#[test]
fn step_decays_displacement_toward_target() {
    let p = default_params();
    // Start displaced above target.
    let s = FeeState::new(p.target_energy + 10_000);
    let (new_s, drift) = FeeController::step(&p, &s, p.target_gas, 100).unwrap();
    // After one half-life, |diff| should be ~ halved; energy converges
    // toward target.
    assert!(new_s.energy < s.energy);
    assert!(
        drift.delta <= 0,
        "Lyapunov V must not grow at perturbation=0 (target_gas)"
    );
}

#[test]
fn step_clips_perturbation_to_decayed_diff() {
    // At target, any large gas_used can't push energy beyond the
    // decay floor (which is 0 at equilibrium). The clip enforces
    // |perturbation| ≤ decayed_abs.
    let p = default_params();
    let s = FeeState::at_equilibrium(p.target_energy);
    let (new_s, _) = FeeController::step(&p, &s, u64::MAX / 2, 1).unwrap();
    // Cannot have exploded; clip held.
    assert_eq!(new_s.energy, p.target_energy);
}

// =================================================================
// Drift struct
// =================================================================

#[test]
fn drift_eq_and_copy() {
    let a = Drift {
        v_before: 100,
        v_after: 50,
        delta: -50,
    };
    let b = Drift {
        v_before: 100,
        v_after: 50,
        delta: -50,
    };
    let c = Drift {
        v_before: 100,
        v_after: 60,
        delta: -40,
    };
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// =================================================================
// Serde
// =================================================================

#[test]
fn fee_state_serde_round_trips() {
    let s = FeeState::new(42_000);
    let json = serde_json::to_string(&s).unwrap();
    let back: FeeState = serde_json::from_str(&json).unwrap();
    assert_eq!(back, s);
}

#[test]
fn drift_serde_round_trips() {
    let d = Drift {
        v_before: 1_000,
        v_after: 500,
        delta: -500,
    };
    let json = serde_json::to_string(&d).unwrap();
    let back: Drift = serde_json::from_str(&json).unwrap();
    assert_eq!(back, d);
}

// =================================================================
// Error ergonomics
// =================================================================

#[test]
fn controller_error_displays() {
    let s = FeeControllerError::NoTimeElapsed.to_string();
    assert!(s.contains("epoch") || s.contains("time"));
}

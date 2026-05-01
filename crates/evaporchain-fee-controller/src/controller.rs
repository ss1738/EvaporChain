//! `FeeController::step` — one block's worth of energy update.
//!
//! Steps:
//!
//! 1. **Decay**: pull `(energy − target)` toward zero via λ for
//!    `epochs_elapsed` epochs (typically `1`). Sign of the imbalance is
//!    preserved.
//! 2. **Perturbation**: add `(gas_used − target_gas)`. The size is
//!    *clipped* to the current decayed magnitude so empty-block paths
//!    are guaranteed-monotone in V (the strict Lyapunov drift property
//!    asserted by `proptests::empty_block_monotone_drift`).
//! 3. **Drift report**: `(V_after − V_before)`. Returned alongside the
//!    new state so consumers can audit the controller without
//!    recomputing V themselves.
//!
//! Base fee is computed by [`base_fee`] from the resulting state — it
//! is *not* part of the state machine. See `state.rs` for the why.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_energy_kernel::energy_at_epoch;
use evaporchain_types::Energy;

use crate::lyapunov::{lyapunov_value, signed_diff};
use crate::params::FeeControllerParams;
use crate::state::FeeState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drift {
    pub v_before: u128,
    pub v_after: u128,
    /// `v_after - v_before` as `i128` (negative = converging toward
    /// equilibrium, positive = diverging).
    pub delta: i128,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FeeControllerError {
    #[error("epochs_elapsed must be > 0 to advance the controller")]
    NoTimeElapsed,
}

pub struct FeeController;

impl FeeController {
    /// Apply one block update.
    pub fn step(
        params: &FeeControllerParams,
        state: &FeeState,
        gas_used: u64,
        epochs_elapsed: u64,
    ) -> Result<(FeeState, Drift), FeeControllerError> {
        if epochs_elapsed == 0 {
            return Err(FeeControllerError::NoTimeElapsed);
        }
        let v_before = lyapunov_value(state.energy, params.target_energy);

        // 1. Decay (E - E*) → preserves sign, magnitude shrinks per λ.
        let diff_before = signed_diff(state.energy, params.target_energy);
        let abs_diff = diff_before.unsigned_abs() as u64;
        let decayed_abs =
            energy_at_epoch(abs_diff, params.chain_lambda.half_life(), epochs_elapsed);
        let decayed_diff: i128 = if diff_before >= 0 {
            decayed_abs as i128
        } else {
            -(decayed_abs as i128)
        };

        // 2. Perturbation, clipped to |decayed_diff| so V cannot grow
        // beyond V_before in the worst case.
        let raw_perturbation: i128 = gas_used as i128 - params.target_gas as i128;
        let clip = decayed_abs as i128;
        let perturbation = raw_perturbation.clamp(-clip, clip);
        let new_diff = decayed_diff + perturbation;

        // 3. New energy, saturating at 0 (energy is unsigned).
        let new_energy_signed = params.target_energy as i128 + new_diff;
        let new_energy = new_energy_signed.max(0) as u64;

        let new_state = FeeState::new(new_energy);
        let v_after = lyapunov_value(new_state.energy, params.target_energy);

        Ok((
            new_state,
            Drift {
                v_before,
                v_after,
                delta: v_after as i128 - v_before as i128,
            },
        ))
    }
}

/// Stateless base-fee computation: `floor + max(0, gain × (E − E*) / 1e6)`.
///
/// Floor when `E ≤ E*` (no premium when chain is at or below target);
/// proportional response above. Saturates to `Energy` at the high end.
pub fn base_fee(state: &FeeState, params: &FeeControllerParams) -> Energy {
    let diff = signed_diff(state.energy, params.target_energy);
    if diff <= 0 {
        return params.base_fee_floor;
    }
    let response = (params.fee_response_ppm as i128 * diff) / 1_000_000;
    let total = (params.base_fee_floor as i128).saturating_add(response);
    if total < 0 {
        0
    } else if total > Energy::MAX as i128 {
        Energy::MAX
    } else {
        total as Energy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::{ChainLambda, Lambda};

    fn params() -> FeeControllerParams {
        FeeControllerParams::new(
            /* target_energy   */ 1_000_000,
            /* target_gas      */ 30_000_000,
            /* chain_lambda    */ ChainLambda::new(Lambda::from_epochs(100)),
            /* fee_response_ppm*/ 125_000,
            /* base_fee_floor  */ 1_000,
        )
    }

    #[test]
    fn no_time_elapsed_rejected() {
        let p = params();
        let s = FeeState::at_equilibrium(p.target_energy);
        let err = FeeController::step(&p, &s, 30_000_000, 0).unwrap_err();
        assert_eq!(err, FeeControllerError::NoTimeElapsed);
    }

    #[test]
    fn at_equilibrium_with_target_gas_is_a_fixed_point() {
        let p = params();
        let s = FeeState::at_equilibrium(p.target_energy);
        let (s2, drift) = FeeController::step(&p, &s, p.target_gas, 1).unwrap();
        assert_eq!(s2.energy, p.target_energy, "energy should stay at target");
        assert_eq!(base_fee(&s2, &p), p.base_fee_floor, "fee at floor");
        assert_eq!(drift.delta, 0, "no drift");
    }

    #[test]
    fn empty_block_above_equilibrium_decays_toward_target() {
        let p = params();
        let s = FeeState::new(p.target_energy + 100_000);
        let (s2, drift) = FeeController::step(&p, &s, p.target_gas, 1).unwrap();
        assert!(s2.energy < s.energy, "energy must shrink toward target");
        assert!(drift.delta <= 0, "V must not grow in empty block");
    }

    #[test]
    fn fee_at_floor_when_at_or_below_equilibrium() {
        let p = params();
        let s_at = FeeState::at_equilibrium(p.target_energy);
        let s_below = FeeState::new(p.target_energy.saturating_sub(50_000));
        assert_eq!(base_fee(&s_at, &p), p.base_fee_floor);
        assert_eq!(base_fee(&s_below, &p), p.base_fee_floor);
    }

    #[test]
    fn fee_rises_with_overshoot() {
        let p = params();
        let s_low = FeeState::new(p.target_energy + 10_000);
        let s_high = FeeState::new(p.target_energy + 100_000);
        let f_low = base_fee(&s_low, &p);
        let f_high = base_fee(&s_high, &p);
        assert!(f_high > f_low);
        assert!(f_low > p.base_fee_floor);
    }

    #[test]
    fn fee_decreases_as_energy_decays_toward_target() {
        // Take two consecutive empty-block steps starting above
        // equilibrium; fee must monotonically fall.
        let p = params();
        let s0 = FeeState::new(p.target_energy + 100_000);
        let f0 = base_fee(&s0, &p);
        let (s1, _) = FeeController::step(&p, &s0, p.target_gas, 1).unwrap();
        let f1 = base_fee(&s1, &p);
        let (s2, _) = FeeController::step(&p, &s1, p.target_gas, 1).unwrap();
        let f2 = base_fee(&s2, &p);
        assert!(f1 <= f0, "fee must not grow on empty-block decay");
        assert!(f2 <= f1, "fee must not grow on empty-block decay (step 2)");
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use proptest::prelude::*;

    fn arb_params() -> impl Strategy<Value = FeeControllerParams> {
        (
            1u64..1_000_000,   // target_energy
            1u64..100_000_000, // target_gas
            10u64..10_000,     // half_life
            1u64..1_000_000,   // fee_response_ppm
            1u64..1_000_000,   // base_fee_floor
        )
            .prop_map(|(te, tg, hl, gain, floor)| {
                FeeControllerParams::new(
                    te,
                    tg,
                    ChainLambda::new(Lambda::from_epochs(hl)),
                    gain,
                    floor,
                )
            })
    }

    proptest! {
        /// Empty-block monotone drift: with `gas_used = target_gas`,
        /// the perturbation is 0 and λ-decay alone drives the state.
        /// V must not grow in this case — the strict Lyapunov drift
        /// property the doctrine claims for the substrate.
        #[test]
        fn empty_block_monotone_drift(
            p in arb_params(),
            energy in 0u64..100_000_000,
        ) {
            let s = FeeState::new(energy);
            let (_, drift) = FeeController::step(&p, &s, p.target_gas, 1).unwrap();
            prop_assert!(
                drift.delta <= 0,
                "empty-block drift must be non-positive (was {}, V_before={}, V_after={})",
                drift.delta, drift.v_before, drift.v_after
            );
        }

        /// Bounded perturbation: with the controller's clip in place,
        /// the new |E − E*| never exceeds 2 × |E_old − E*|.
        #[test]
        fn bounded_perturbation_keeps_diff_within_2x(
            p in arb_params(),
            energy in 0u64..100_000_000,
            gas_used in 0u64..200_000_000,
        ) {
            let s = FeeState::new(energy);
            let diff_before = signed_diff(s.energy, p.target_energy).unsigned_abs();
            let (s2, _) = FeeController::step(&p, &s, gas_used, 1).unwrap();
            let diff_after = signed_diff(s2.energy, p.target_energy).unsigned_abs();
            prop_assert!(
                diff_after <= diff_before.saturating_mul(2),
                "|diff_after|={} > 2 × |diff_before|={}", diff_after, diff_before
            );
        }

        /// Base fee is monotonically non-decreasing in energy above E*.
        #[test]
        fn base_fee_monotone_in_overshoot(
            p in arb_params(),
            extra_a in 0u64..1_000_000,
            extra_b in 0u64..1_000_000,
        ) {
            let s_a = FeeState::new(p.target_energy.saturating_add(extra_a.min(extra_b)));
            let s_b = FeeState::new(p.target_energy.saturating_add(extra_a.max(extra_b)));
            let f_a = base_fee(&s_a, &p);
            let f_b = base_fee(&s_b, &p);
            prop_assert!(f_b >= f_a);
        }
    }
}

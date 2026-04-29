//! Singh-Lyapunov fee path — the theorem-grade alternative to the
//! PID controller in [`crate::fees`].
//!
//! Wraps `evaporchain-fee-controller` for the execution-layer caller.
//! Lives alongside the existing `PidFeeController` (which stays the
//! default until the consensus + node binaries opt in via this path).
//!
//! ## Why a sibling instead of a replacement
//!
//! - PID is a heuristic with parameter tuning; the new path has
//!   provable empty-block monotone Lyapunov drift (asserted by the
//!   `evaporchain-fee-controller` crate's property test).
//! - Production swap from PID → Singh-Lyapunov is a governance
//!   amendment, not a code rip-out — both controllers run during the
//!   migration window.
//!
//! ## API
//!
//! - [`step`] applies one block update against the current `FeeState`
//!   under the chain-global `ChainLambda`. Returns `(FeeState, Drift)`.
//! - [`base_fee_at`] is the stateless `(E − E*) → fee` projection.

use evaporchain_energy_kernel::ChainLambda;
use evaporchain_fee_controller::{
    base_fee, controller::FeeControllerError, FeeController, FeeControllerParams, FeeState,
    Drift,
};
use evaporchain_types::Energy;

/// Apply one block of the Singh-Lyapunov controller.
pub fn step(
    params: &FeeControllerParams,
    state: &FeeState,
    gas_used: u64,
    epochs_elapsed: u64,
) -> Result<(FeeState, Drift), FeeControllerError> {
    FeeController::step(params, state, gas_used, epochs_elapsed)
}

/// Stateless base-fee projection at the current `FeeState`.
pub fn base_fee_at(state: &FeeState, params: &FeeControllerParams) -> Energy {
    base_fee(state, params)
}

/// Construct production-default fee-controller params from the chain
/// λ. Mirrors `FeeControllerParams::default_genesis()` but with a
/// caller-supplied λ so the execution layer can pass through whatever
/// the chain's `ChainLambda` is set to (governance-rotatable).
pub fn default_params(chain_lambda: ChainLambda) -> FeeControllerParams {
    FeeControllerParams::new(
        /* target_energy   */ 1_000_000,
        /* target_gas      */ 30_000_000,
        chain_lambda,
        /* fee_response_ppm*/ 125_000, // 1/8 in ppm — matches EIP-1559 responsiveness
        /* base_fee_floor  */ 1_000,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn step_at_equilibrium_is_a_fixed_point() {
        let p = default_params(lambda());
        let s = FeeState::at_equilibrium(p.target_energy);
        let (s2, drift) = step(&p, &s, p.target_gas, 1).unwrap();
        assert_eq!(s2.energy, p.target_energy);
        assert_eq!(drift.delta, 0);
    }

    #[test]
    fn empty_block_above_equilibrium_decays() {
        let p = default_params(lambda());
        let s = FeeState::new(p.target_energy + 100_000);
        let (s2, drift) = step(&p, &s, p.target_gas, 1).unwrap();
        assert!(s2.energy < s.energy);
        assert!(drift.delta <= 0, "Lyapunov drift must be non-positive on empty blocks");
    }

    #[test]
    fn base_fee_at_floor_when_below_equilibrium() {
        let p = default_params(lambda());
        let s = FeeState::at_equilibrium(p.target_energy);
        assert_eq!(base_fee_at(&s, &p), p.base_fee_floor);
    }

    #[test]
    fn base_fee_rises_with_overshoot() {
        let p = default_params(lambda());
        let s_low = FeeState::new(p.target_energy + 10_000);
        let s_high = FeeState::new(p.target_energy + 100_000);
        assert!(base_fee_at(&s_high, &p) > base_fee_at(&s_low, &p));
    }
}

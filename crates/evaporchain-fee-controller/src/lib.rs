//! Singh-Lyapunov Fee Controller — the whitepaper centerpiece.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 4:
//!
//! > Singh-Lyapunov Fee Controller — First L1 with **provably globally
//! > stable** fee market. Lyapunov `V(E) = ½(E − E*)²` converges
//! > *because* of decay. Antifragile under attack. **Whitepaper
//! > centerpiece.**
//!
//! And per the mechanism-design agent's source attribution: "Lyapunov
//! drift on EIP-1559". The shape is an *integrator with leak*:
//!
//! ```text
//!   E_{n+1} = decay(E_n − E*; λ, 1) + E* + (gas_used − target_gas)
//!   diff_{n+1} = decay(diff_n; λ, 1) + perturbation
//!   V(diff)   = ½ diff²
//! ```
//!
//! The natural λ-decay of `evaporchain-energy-kernel` does the
//! stabilizing work. The controller's only job is to translate
//! `(E − E*)` into a base-fee response — with the perturbation
//! magnitude clipped so each empty block is guaranteed-monotone in `V`.
//!
//! This crate ships the substrate (state machine + Lyapunov function +
//! step + drift measurement). The closed-form equilibrium extension
//! (CFM, Crooks-Singh) and the Sanov-Slashing companion live in their
//! own crates so each primitive is independently auditable.
//!
//! ## Module map
//!
//! - [`params`] — `FeeControllerParams` (target_energy, target_gas, λ,
//!   gain).
//! - [`state`] — `FeeState` (current energy + current base fee).
//! - [`lyapunov`] — `lyapunov_value(diff)` and signed-diff helpers.
//! - [`controller`] — `FeeController::step` applies one block update;
//!   returns `(FeeState, Drift)` so callers can audit the Lyapunov
//!   change directly.

pub mod controller;
pub mod lyapunov;
pub mod params;
pub mod state;

pub use controller::{base_fee, Drift, FeeController};
pub use lyapunov::{lyapunov_value, signed_diff};
pub use params::FeeControllerParams;
pub use state::FeeState;

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh-Lyapunov fee controller admits monotone
    /// convergence under decay: V(E) = ½(E−E*)² is non-increasing on
    /// empty-block trajectories, and `base_fee` does not grow.
    /// Pure-integer i128 math; validator-deterministic."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let p = FeeControllerParams::default_genesis();
        // Start above equilibrium so empty-block decay drives V down.
        let s0 = FeeState::new(p.target_energy.saturating_add(p.target_gas));
        let f0 = base_fee(&s0, &p);
        let v0 = lyapunov_value(s0.energy, p.target_energy);

        // Empty block: fee + Lyapunov both monotone non-increasing.
        let (s1, _) = FeeController::step(&p, &s0, 0, 1).unwrap();
        let f1 = base_fee(&s1, &p);
        let v1 = lyapunov_value(s1.energy, p.target_energy);
        assert!(f1 <= f0, "fee must not grow on empty-block decay");
        assert!(v1 <= v0, "Lyapunov must not grow on empty-block decay");

        let (s2, _) = FeeController::step(&p, &s1, 0, 2).unwrap();
        let f2 = base_fee(&s2, &p);
        let v2 = lyapunov_value(s2.energy, p.target_energy);
        assert!(f2 <= f1);
        assert!(v2 <= v1);
    }
}

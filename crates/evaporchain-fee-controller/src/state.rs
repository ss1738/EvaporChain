//! `FeeState` — the integrator-with-leak energy. Base fee is *not*
//! part of state — it's a stateless function of `(energy, params)`
//! exposed via [`crate::controller::base_fee`].
//!
//! Keeping base fee derived (rather than integrated) is what makes the
//! Lyapunov property go through cleanly: V depends on energy alone, so
//! the empty-block monotone-drift proof reduces to "decay shrinks
//! `|E − E*|`", which `evaporchain-types::energy_at_epoch` already gives
//! us (mechanized in `research/coq/EnergyDecayMonotonicity.v`).

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeState {
    /// Current cumulative energy E.
    pub energy: Energy,
}

impl FeeState {
    pub const fn new(energy: Energy) -> Self {
        Self { energy }
    }

    /// "At equilibrium" initial state — energy at target.
    pub const fn at_equilibrium(target_energy: Energy) -> Self {
        Self { energy: target_energy }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_equilibrium_sets_energy_to_target() {
        let s = FeeState::at_equilibrium(1_000_000);
        assert_eq!(s.energy, 1_000_000);
    }
}

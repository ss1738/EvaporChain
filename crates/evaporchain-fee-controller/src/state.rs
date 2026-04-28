//! `FeeState` — current energy + current base fee.
//!
//! The energy state is the integrator-with-leak; base fee is its
//! visible projection (what users actually pay).

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeState {
    /// Current cumulative energy E.
    pub energy: Energy,
    /// Current base fee charged on transactions (chain-native unit;
    /// the controller treats it as opaque).
    pub base_fee: Energy,
}

impl FeeState {
    pub const fn new(energy: Energy, base_fee: Energy) -> Self {
        Self { energy, base_fee }
    }

    /// "At equilibrium" initial state — energy at target, base fee at
    /// the supplied seed.
    pub const fn at_equilibrium(target_energy: Energy, base_fee_seed: Energy) -> Self {
        Self {
            energy: target_energy,
            base_fee: base_fee_seed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_equilibrium_sets_energy_to_target() {
        let s = FeeState::at_equilibrium(1_000_000, 1_000);
        assert_eq!(s.energy, 1_000_000);
        assert_eq!(s.base_fee, 1_000);
    }
}

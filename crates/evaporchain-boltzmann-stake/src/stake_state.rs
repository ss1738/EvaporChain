//! `ValidatorStake` — per-validator state for the Singh-Boltzmann
//! decay/refresh cycle.

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatorStake {
    /// Current effective active stake (after decay applied to
    /// `last_touched_epoch`).
    pub active: Energy,
    /// Epoch at which `active` was last advanced (decayed and/or
    /// refreshed). The decay function reads this to compute
    /// `epochs_elapsed`.
    pub last_touched_epoch: u64,
}

impl ValidatorStake {
    pub const fn new(active: Energy, last_touched_epoch: u64) -> Self {
        Self {
            active,
            last_touched_epoch,
        }
    }

    /// Initial state: full stake at epoch 0.
    pub const fn fresh(initial: Energy) -> Self {
        Self::new(initial, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_is_at_epoch_zero() {
        let s = ValidatorStake::fresh(1_000);
        assert_eq!(s.active, 1_000);
        assert_eq!(s.last_touched_epoch, 0);
    }
}

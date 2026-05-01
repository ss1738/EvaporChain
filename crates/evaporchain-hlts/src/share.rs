//! `Share` — single Shamir-style share with attached half-life.

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Share {
    /// 1-indexed share index (Shamir convention; index 0 is the secret).
    pub idx: u32,
    /// Seed energy at `observed_epoch`. Decays under chain-global λ.
    pub energy: Energy,
    pub observed_epoch: u64,
}

impl Share {
    pub const fn new(idx: u32, energy: Energy, observed_epoch: u64) -> Self {
        Self {
            idx,
            energy,
            observed_epoch,
        }
    }
}

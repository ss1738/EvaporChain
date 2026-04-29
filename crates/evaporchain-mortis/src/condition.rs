//! `MortisCondition` — chain-set parameters for the death trigger.

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MortisCondition {
    /// `ε` — refresh-pool floor below which the chain is considered
    /// economically dead.
    pub refresh_pool_floor: Energy,
    /// `N` — number of consecutive epochs the floor must be breached
    /// before the death certificate auto-mints. Stops single-block
    /// fluctuations from killing the chain.
    pub sustained_epochs: u64,
}

impl MortisCondition {
    pub const fn new(refresh_pool_floor: Energy, sustained_epochs: u64) -> Self {
        Self {
            refresh_pool_floor,
            sustained_epochs,
        }
    }

    /// Provisional governance defaults: floor = 1_000 energy units;
    /// 4_096 epochs (≈ one DEFAULT_LAMBDA half-life) of sustained
    /// breach. Both rotatable by governance.
    pub const fn default_genesis() -> Self {
        Self::new(1_000, 4_096)
    }
}

impl Default for MortisCondition {
    fn default() -> Self {
        Self::default_genesis()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_sane_values() {
        let c = MortisCondition::default_genesis();
        assert_eq!(c.refresh_pool_floor, 1_000);
        assert_eq!(c.sustained_epochs, 4_096);
    }
}

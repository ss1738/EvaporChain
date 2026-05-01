//! `DemurrageParams` — chain-set parameters for the piecewise-log rate.
//!
//! Both fields are governance-rotatable; the genesis defaults are
//! conservative placeholders pending the tokenomics ceremony per
//! INVENTION_STACK.md §8 open question "Total chain energy budget at
//! genesis sets all other constants."

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

/// Chain-set parameters governing the demurrage rate.
///
/// `lambda_base_ppm` is the per-epoch rate coefficient in parts-per-
/// million. The actual rate at a given balance is
/// `lambda_base_ppm · log_2(balance / threshold)` ppm of `balance` per
/// epoch — see [`crate::rate::rate_ppm`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemurrageParams {
    /// λ_base in parts-per-million per epoch. Range governance-bounded
    /// in production; `1` is a meaningfully gentle rate (1 ppm per
    /// epoch per log2-doubling above threshold).
    pub lambda_base_ppm: u64,
    /// Energy threshold below which demurrage is zero.
    pub threshold: Energy,
}

impl DemurrageParams {
    pub const fn new(lambda_base_ppm: u64, threshold: Energy) -> Self {
        Self {
            lambda_base_ppm,
            threshold,
        }
    }

    /// Provisional genesis defaults: 1 ppm per epoch per log-doubling,
    /// threshold of 1024 energy units. Both are placeholders pending
    /// the tokenomics ceremony.
    pub const fn default_genesis() -> Self {
        Self::new(1, 1024)
    }

    /// Convenience: a "no demurrage" param set (rate coefficient = 0).
    /// Useful for tests and for chains that want to disable demurrage
    /// at the parameter level without removing the code path.
    pub const fn disabled() -> Self {
        Self::new(0, u64::MAX)
    }

    /// Is this parameter set effectively a no-op?
    pub const fn is_disabled(&self) -> bool {
        self.lambda_base_ppm == 0
    }
}

impl Default for DemurrageParams {
    fn default() -> Self {
        Self::default_genesis()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_genesis_non_disabled() {
        let p = DemurrageParams::default_genesis();
        assert!(!p.is_disabled());
        assert_eq!(p.lambda_base_ppm, 1);
        assert_eq!(p.threshold, 1024);
    }

    #[test]
    fn disabled_round_trip() {
        let p = DemurrageParams::disabled();
        assert!(p.is_disabled());
    }

    #[test]
    fn default_matches_default_genesis() {
        assert_eq!(
            DemurrageParams::default(),
            DemurrageParams::default_genesis()
        );
    }
}

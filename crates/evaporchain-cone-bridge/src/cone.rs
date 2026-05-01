//! `EnergyCone` — per-chain decay cone.

use serde::{Deserialize, Serialize};

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};
use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnergyCone {
    pub chain_lambda: ChainLambda,
    pub threshold: Energy,
    pub committed_energy: Energy,
    pub observed_epoch: u64,
}

impl EnergyCone {
    pub const fn new(
        chain_lambda: ChainLambda,
        threshold: Energy,
        committed_energy: Energy,
        observed_epoch: u64,
    ) -> Self {
        Self {
            chain_lambda,
            threshold,
            committed_energy,
            observed_epoch,
        }
    }

    /// True iff at `query_epoch` the cone's λ-decayed remaining
    /// energy is at or above `threshold`.
    pub fn is_inside(&self, query_epoch: u64) -> bool {
        if query_epoch < self.observed_epoch {
            return self.committed_energy >= self.threshold;
        }
        let elapsed = query_epoch - self.observed_epoch;
        let remaining = energy_at_epoch(
            self.committed_energy,
            self.chain_lambda.half_life(),
            elapsed,
        );
        remaining >= self.threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn cone() -> EnergyCone {
        EnergyCone::new(ChainLambda::new(Lambda::from_epochs(100)), 500, 1000, 0)
    }

    #[test]
    fn at_observation_above_threshold_inside() {
        assert!(cone().is_inside(0));
    }

    #[test]
    fn after_decay_below_threshold_outside() {
        // After 200 epochs at half_life=100, remaining ≈ 250 < 500.
        assert!(!cone().is_inside(200));
    }

    #[test]
    fn before_observation_uses_committed() {
        let c = EnergyCone::new(ChainLambda::new(Lambda::from_epochs(100)), 500, 1000, 10);
        assert!(c.is_inside(0));
    }
}

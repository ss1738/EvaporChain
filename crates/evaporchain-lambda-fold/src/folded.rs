//! `FoldedInstance` — accumulated state across folded steps.

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldedInstance {
    /// Domain-separated blake3 accumulator over all folded
    /// `state_hash`es. Substrate stand-in for the real Nova
    /// R1CS-folded witness commitment.
    pub acc_hash: [u8; 32],
    /// Sum of all folded step energies, λ-decayed forward to
    /// `latest_epoch`. The verifier checks this against the chain's
    /// expected aggregate-energy budget.
    pub total_energy_remaining: u128,
    /// Number of folded steps. Verifier work stays O(log step_count)
    /// in the real Nova realisation; here we just track the count
    /// for audit purposes.
    pub step_count: u64,
    /// Epoch at which the latest fold operation was performed.
    pub latest_epoch: u64,
}

impl FoldedInstance {
    /// The "no folds yet" identity. `acc_hash` = canonical zero;
    /// `total_energy_remaining` = 0; `step_count` = 0;
    /// `latest_epoch` = 0.
    pub const fn identity() -> Self {
        Self {
            acc_hash: [0u8; 32],
            total_energy_remaining: 0,
            step_count: 0,
            latest_epoch: 0,
        }
    }

    pub const fn is_identity(&self) -> bool {
        self.step_count == 0
    }
}

impl Default for FoldedInstance {
    fn default() -> Self {
        Self::identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_empty() {
        let i = FoldedInstance::identity();
        assert!(i.is_identity());
        assert_eq!(i.step_count, 0);
        assert_eq!(i.acc_hash, [0u8; 32]);
        assert_eq!(i.total_energy_remaining, 0);
    }
}

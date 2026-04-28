//! `StepWitness` — per-step prover input.

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepWitness {
    /// 32-byte commitment to the step's state transition (e.g.
    /// `blake3(state_root_old || tx_root || state_root_new)`).
    pub state_hash: [u8; 32],
    /// Energy of this step (e.g. block gas × λ-coefficient). Decays
    /// alongside the rest of chain energy.
    pub step_energy: Energy,
    /// Epoch at which this step was observed.
    pub observed_epoch: u64,
}

impl StepWitness {
    pub const fn new(state_hash: [u8; 32], step_energy: Energy, observed_epoch: u64) -> Self {
        Self {
            state_hash,
            step_energy,
            observed_epoch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_construction() {
        let w = StepWitness::new([1u8; 32], 1000, 5);
        assert_eq!(w.state_hash, [1u8; 32]);
        assert_eq!(w.step_energy, 1000);
        assert_eq!(w.observed_epoch, 5);
    }
}

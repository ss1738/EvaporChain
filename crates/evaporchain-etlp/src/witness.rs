//! `EnergyWitness` — chain's proof that committed energy has accrued.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnergyWitness {
    pub committed_energy: Energy,
    pub observed_epoch: u64,
    /// blake3 binding over (capsule_seal_epoch, threshold,
    /// committed_energy, observed_epoch).
    pub binding: [u8; 32],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WitnessError {
    #[error("witness binding mismatch")]
    BindingMismatch,
}

impl EnergyWitness {
    /// Compute the binding hash for a witness against a specific
    /// capsule. Production callers MUST supply this exact hash in
    /// `binding` for the witness to verify.
    pub fn compute_binding(
        seal_epoch: u64,
        energy_threshold: Energy,
        committed_energy: Energy,
        observed_epoch: u64,
    ) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(b"evaporchain-etlp-witness");
        h.update(&seal_epoch.to_le_bytes());
        h.update(&energy_threshold.to_le_bytes());
        h.update(&committed_energy.to_le_bytes());
        h.update(&observed_epoch.to_le_bytes());
        *h.finalize().as_bytes()
    }
}

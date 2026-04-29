//! `EgFssKey` — energy-evolving signing key.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_types::Energy;

const EVOLVE_TAG: &[u8] = b"evaporchain-eg-fss-evolve";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgFssKey {
    pub period_index: u64,
    /// Current period's secret key material. Substrate is 32-byte
    /// blake3-chain stand-in; production is RSA/BLS.
    pub key_material: [u8; 32],
    /// Energy accumulated toward the next period's evolution.
    pub energy_residual: Energy,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum KeyError {
    #[error("threshold_per_period must be > 0")]
    ZeroThreshold,
}

impl EgFssKey {
    /// Construct a fresh key at period 0 from a seed.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            period_index: 0,
            key_material: seed,
            energy_residual: 0,
        }
    }

    /// Spend `energy` against this key's evolution counter. Once
    /// the residual crosses `threshold_per_period`, the key evolves
    /// (period_index ++, key_material = blake3(prev || period)).
    /// Multiple period crossings collapse into one evolved key per
    /// crossing in a single call.
    pub fn evolve(self, energy: Energy, threshold_per_period: Energy) -> Result<Self, KeyError> {
        if threshold_per_period == 0 {
            return Err(KeyError::ZeroThreshold);
        }
        let total = self
            .energy_residual
            .saturating_add(energy);
        let advances = total / threshold_per_period;
        let residual = total % threshold_per_period;
        let mut key_material = self.key_material;
        let mut period_index = self.period_index;
        for _ in 0..advances {
            period_index = period_index.saturating_add(1);
            let mut h = blake3::Hasher::new();
            h.update(EVOLVE_TAG);
            h.update(&key_material);
            h.update(&period_index.to_le_bytes());
            key_material = *h.finalize().as_bytes();
        }
        Ok(Self {
            period_index,
            key_material,
            energy_residual: residual,
        })
    }
}

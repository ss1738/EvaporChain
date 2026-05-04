//! Minimal triage record. The wallet may extend this with display
//! fields (name, image, owner) — the on-chain primitive only needs
//! the decay-relevant subset.

use evaporchain_types::{Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TriageItemError {
    #[error("energy_at_creation must be > 0")]
    ZeroEnergy,
    #[error("half_life must be > 0")]
    ZeroHalfLife,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageItem {
    pub id: [u8; 32],
    /// Energy at the most recent refresh (or initial energy at mint).
    pub energy_at_anchor: Energy,
    pub half_life: HalfLife,
    /// Epoch the cached `energy_at_anchor` was set.
    pub last_refreshed_epoch: Epoch,
}

impl TriageItem {
    pub fn new(
        id: [u8; 32],
        energy_at_anchor: Energy,
        half_life: HalfLife,
        last_refreshed_epoch: Epoch,
    ) -> Result<Self, TriageItemError> {
        if energy_at_anchor == 0 {
            return Err(TriageItemError::ZeroEnergy);
        }
        if half_life == 0 {
            return Err(TriageItemError::ZeroHalfLife);
        }
        Ok(Self {
            id,
            energy_at_anchor,
            half_life,
            last_refreshed_epoch,
        })
    }

    /// Current energy at `epoch_now` per the standard half-life rule.
    pub fn energy_at(&self, epoch_now: Epoch) -> Energy {
        let elapsed = epoch_now.saturating_sub(self.last_refreshed_epoch);
        evaporchain_types::energy_at_epoch(self.energy_at_anchor, self.half_life, elapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_energy() {
        assert_eq!(
            TriageItem::new([0; 32], 0, 100, 0).unwrap_err(),
            TriageItemError::ZeroEnergy
        );
    }

    #[test]
    fn rejects_zero_half_life() {
        assert_eq!(
            TriageItem::new([0; 32], 1000, 0, 0).unwrap_err(),
            TriageItemError::ZeroHalfLife
        );
    }

    #[test]
    fn energy_at_anchor_returns_initial() {
        let it = TriageItem::new([0; 32], 1000, 100, 50).unwrap();
        assert_eq!(it.energy_at(50), 1000);
    }

    #[test]
    fn energy_decays_over_time() {
        let it = TriageItem::new([0; 32], 1000, 100, 0).unwrap();
        let e = it.energy_at(100);
        assert!(e < 1000);
    }

    #[test]
    fn round_trip_serde() {
        let it = TriageItem::new([7; 32], 500, 50, 10).unwrap();
        let s = serde_json::to_string(&it).unwrap();
        let back: TriageItem = serde_json::from_str(&s).unwrap();
        assert_eq!(it, back);
    }
}

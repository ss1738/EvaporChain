//! [`WeightedMember`] — a single member's decaying voting weight.

use evaporchain_types::energy_at_epoch;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum QuorumError {
    #[error("zero member weight")]
    ZeroWeight,
    #[error("zero half-life")]
    ZeroHalfLife,
    #[error("threshold {bps} bps out of range (must be 1..=10000)")]
    ThresholdOutOfRange { bps: u32 },
    #[error("member already exists")]
    DuplicateMember,
    #[error("member not found")]
    MemberNotFound,
    #[error("non-monotone time: incoming {incoming} < last_refreshed {last}")]
    NonMonotoneTime { incoming: u64, last: u64 },
}

/// A member's voting weight, decaying along the half-life curve from
/// `weight` set at `last_refreshed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeightedMember {
    /// Weight baseline at `last_refreshed`.
    pub weight: u64,
    /// Half-life (epochs) of weight decay.
    pub half_life: u64,
    /// Epoch the weight baseline was last set.
    pub last_refreshed: u64,
}

impl WeightedMember {
    pub fn new(weight: u64, half_life: u64, now: u64) -> Result<Self, QuorumError> {
        if weight == 0 {
            return Err(QuorumError::ZeroWeight);
        }
        if half_life == 0 {
            return Err(QuorumError::ZeroHalfLife);
        }
        Ok(Self {
            weight,
            half_life,
            last_refreshed: now,
        })
    }

    /// Decayed weight at epoch `now`.
    pub fn current_weight(&self, now: u64) -> u64 {
        let elapsed = now.saturating_sub(self.last_refreshed);
        energy_at_epoch(self.weight, self.half_life, elapsed)
    }

    /// Top up the decayed weight and reset the decay clock to `now`.
    pub fn refresh(&mut self, top_up: u64, now: u64) -> Result<(), QuorumError> {
        if now < self.last_refreshed {
            return Err(QuorumError::NonMonotoneTime {
                incoming: now,
                last: self.last_refreshed,
            });
        }
        self.weight = self.current_weight(now).saturating_add(top_up);
        self.last_refreshed = now;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_zero_weight_and_half_life() {
        assert_eq!(WeightedMember::new(0, 10, 0).unwrap_err(), QuorumError::ZeroWeight);
        assert_eq!(
            WeightedMember::new(100, 0, 0).unwrap_err(),
            QuorumError::ZeroHalfLife
        );
    }

    #[test]
    fn weight_decays_at_half_life() {
        let m = WeightedMember::new(100, 10, 0).unwrap();
        assert_eq!(m.current_weight(0), 100);
        assert_eq!(m.current_weight(10), 50);
        assert_eq!(m.current_weight(20), 25);
    }

    #[test]
    fn weight_is_monotone_non_increasing() {
        let m = WeightedMember::new(1_000_000, 100, 0).unwrap();
        let a = m.current_weight(10);
        let b = m.current_weight(100);
        let c = m.current_weight(500);
        assert!(a >= b && b >= c);
    }

    #[test]
    fn refresh_tops_up_decayed_weight_and_resets_clock() {
        let mut m = WeightedMember::new(100, 10, 0).unwrap();
        // At t=20 weight is 25. Refresh +75 → 100.
        m.refresh(75, 20).unwrap();
        assert_eq!(m.current_weight(20), 100);
        assert_eq!(m.last_refreshed, 20);
        assert_eq!(m.current_weight(40), 25); // decays from t=20 again
    }

    #[test]
    fn refresh_in_the_past_rejected() {
        let mut m = WeightedMember::new(100, 10, 50).unwrap();
        assert!(matches!(
            m.refresh(1, 49),
            Err(QuorumError::NonMonotoneTime { .. })
        ));
    }
}

//! [`Reputation`] — a per-subject signed score from two decaying
//! accumulators (`merit` and `demerit`).

use evaporchain_types::energy_at_epoch;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RepError {
    #[error("zero half-life")]
    ZeroHalfLife,
    #[error("non-monotone time: incoming {incoming} < last_update {last}")]
    NonMonotoneTime { incoming: u64, last: u64 },
}

/// Signed reputation: `net = merit - demerit`, with each side decaying
/// independently along the same half-life curve from `last_update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reputation {
    /// Positive-standing accumulator (baseline at `last_update`).
    pub merit: u64,
    /// Negative-standing accumulator (baseline at `last_update`).
    pub demerit: u64,
    /// Half-life (epochs) applied to both accumulators.
    pub half_life: u64,
    /// Epoch both accumulators were last re-based.
    pub last_update: u64,
}

impl Reputation {
    pub fn new(half_life: u64, now: u64) -> Result<Self, RepError> {
        if half_life == 0 {
            return Err(RepError::ZeroHalfLife);
        }
        Ok(Self {
            merit: 0,
            demerit: 0,
            half_life,
            last_update: now,
        })
    }

    /// Decayed merit at `now`.
    pub fn merit_at(&self, now: u64) -> u64 {
        energy_at_epoch(self.merit, self.half_life, now.saturating_sub(self.last_update))
    }

    /// Decayed demerit at `now`.
    pub fn demerit_at(&self, now: u64) -> u64 {
        energy_at_epoch(self.demerit, self.half_life, now.saturating_sub(self.last_update))
    }

    /// Net signed reputation at `now` (`merit - demerit`).
    pub fn net_at(&self, now: u64) -> i128 {
        self.merit_at(now) as i128 - self.demerit_at(now) as i128
    }

    /// Whether net reputation is strictly positive at `now`.
    pub fn is_positive_at(&self, now: u64) -> bool {
        self.net_at(now) > 0
    }

    /// Both accumulators have fully decayed to zero at `now`.
    pub fn is_dormant_at(&self, now: u64) -> bool {
        self.merit_at(now) == 0 && self.demerit_at(now) == 0
    }

    /// Decay both accumulators to `now` and pin the clock there.
    fn rebase(&mut self, now: u64) -> Result<(), RepError> {
        if now < self.last_update {
            return Err(RepError::NonMonotoneTime {
                incoming: now,
                last: self.last_update,
            });
        }
        self.merit = self.merit_at(now);
        self.demerit = self.demerit_at(now);
        self.last_update = now;
        Ok(())
    }

    /// Add `amount` of merit at `now` (re-bases both sides first).
    pub fn record_merit(&mut self, amount: u64, now: u64) -> Result<(), RepError> {
        self.rebase(now)?;
        self.merit = self.merit.saturating_add(amount);
        Ok(())
    }

    /// Add `amount` of demerit at `now` (re-bases both sides first).
    pub fn record_demerit(&mut self, amount: u64, now: u64) -> Result<(), RepError> {
        self.rebase(now)?;
        self.demerit = self.demerit.saturating_add(amount);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Reputation {
        Reputation::new(10, 0).unwrap()
    }

    #[test]
    fn new_rejects_zero_half_life() {
        assert_eq!(Reputation::new(0, 0).unwrap_err(), RepError::ZeroHalfLife);
    }

    #[test]
    fn merit_raises_net() {
        let mut r = fresh();
        r.record_merit(1000, 0).unwrap();
        assert_eq!(r.net_at(0), 1000);
        assert!(r.is_positive_at(0));
    }

    #[test]
    fn demerit_can_drive_net_negative() {
        let mut r = fresh();
        r.record_merit(300, 0).unwrap();
        r.record_demerit(800, 0).unwrap();
        assert_eq!(r.net_at(0), -500);
        assert!(!r.is_positive_at(0));
    }

    #[test]
    fn both_sides_decay_and_net_halves() {
        let mut r = fresh();
        r.record_merit(1000, 0).unwrap();
        r.record_demerit(400, 0).unwrap();
        assert_eq!(r.net_at(0), 600);
        // one half-life: merit 500, demerit 200 → net 300.
        assert_eq!(r.merit_at(10), 500);
        assert_eq!(r.demerit_at(10), 200);
        assert_eq!(r.net_at(10), 300);
    }

    #[test]
    fn recency_dominates() {
        // Stale 100 vs fresh 100.
        let mut stale = fresh();
        stale.record_merit(100, 0).unwrap();
        let mut recent = fresh();
        recent.record_merit(100, 20).unwrap();
        // Compare both at t=20: stale decayed to 25, fresh still 100.
        assert_eq!(stale.net_at(20), 25);
        assert_eq!(recent.net_at(20), 100);
        assert!(recent.net_at(20) > stale.net_at(20));
    }

    #[test]
    fn fault_can_be_lived_down() {
        let mut r = fresh();
        r.record_demerit(1000, 0).unwrap();
        assert_eq!(r.net_at(0), -1000);
        // With no further faults, the demerit decays toward 0:
        // 1000 >> 5 = 31 at t=50, and fully gone by t=100.
        assert!(r.net_at(50) > -100);
        assert_eq!(r.net_at(100), 0);
    }

    #[test]
    fn recover_by_earning() {
        let mut r = fresh();
        r.record_demerit(1000, 0).unwrap();
        r.record_merit(2000, 0).unwrap();
        assert_eq!(r.net_at(0), 1000);
        assert!(r.is_positive_at(0));
    }

    #[test]
    fn dormant_after_full_decay() {
        let mut r = fresh();
        r.record_merit(1000, 0).unwrap();
        r.record_demerit(500, 0).unwrap();
        assert!(!r.is_dormant_at(0));
        assert!(r.is_dormant_at(100_000));
    }

    #[test]
    fn non_monotone_time_rejected() {
        let mut r = Reputation::new(10, 100).unwrap();
        assert!(matches!(
            r.record_merit(1, 50),
            Err(RepError::NonMonotoneTime { .. })
        ));
    }

    #[test]
    fn saturating_add_does_not_panic() {
        let mut r = fresh();
        r.record_merit(u64::MAX, 0).unwrap();
        r.record_merit(u64::MAX, 0).unwrap(); // saturates, no overflow
        assert_eq!(r.merit, u64::MAX);
    }

    #[test]
    fn serde_roundtrip() {
        let mut r = fresh();
        r.record_merit(123, 1).unwrap();
        r.record_demerit(45, 2).unwrap();
        let json = serde_json::to_string(&r).unwrap();
        let back: Reputation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }
}

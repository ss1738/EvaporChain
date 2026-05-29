//! [`DecayBucket`] — a single-stream decay-pressure rate limiter.
//!
//! Pressure rises by `cost` per admitted action and decays through
//! `evaporchain_types::energy_at_epoch`. An action is admitted only
//! while `decayed_pressure + cost <= capacity`.

use evaporchain_types::energy_at_epoch;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RateError {
    #[error("zero capacity")]
    ZeroCapacity,
    #[error("zero half-life")]
    ZeroHalfLife,
    #[error("non-monotone time: incoming {incoming} < last_update {last}")]
    NonMonotoneTime { incoming: u64, last: u64 },
    #[error("rate limited: pressure {current_pressure} + cost {cost} would exceed capacity {capacity}")]
    RateLimited {
        current_pressure: u64,
        cost: u64,
        capacity: u64,
    },
}

/// A decaying-pressure bucket. `pressure` is the baseline at
/// `last_update`; it decays from there along the half-life curve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecayBucket {
    /// Maximum pressure (burst size). An action is admitted only if the
    /// resulting pressure stays at or below this.
    pub capacity: u64,
    /// Half-life (epochs) of pressure decay — the recovery rate.
    pub half_life: u64,
    /// Pressure baseline at `last_update`.
    pub pressure: u64,
    /// Epoch the pressure baseline was last set.
    pub last_update: u64,
}

impl DecayBucket {
    /// Create an empty bucket (zero pressure → full allowance).
    pub fn new(capacity: u64, half_life: u64, now: u64) -> Result<Self, RateError> {
        if capacity == 0 {
            return Err(RateError::ZeroCapacity);
        }
        if half_life == 0 {
            return Err(RateError::ZeroHalfLife);
        }
        Ok(Self {
            capacity,
            half_life,
            pressure: 0,
            last_update: now,
        })
    }

    /// Decayed pressure at epoch `now`. Reads before `last_update`
    /// clamp to the baseline rather than growing.
    pub fn pressure_at(&self, now: u64) -> u64 {
        let elapsed = now.saturating_sub(self.last_update);
        energy_at_epoch(self.pressure, self.half_life, elapsed)
    }

    /// Headroom available at epoch `now`.
    pub fn available(&self, now: u64) -> u64 {
        self.capacity.saturating_sub(self.pressure_at(now))
    }

    /// Whether an action costing `cost` would be admitted at `now`,
    /// without mutating the bucket.
    pub fn would_allow(&self, cost: u64, now: u64) -> bool {
        self.pressure_at(now).saturating_add(cost) <= self.capacity
    }

    /// Attempt to admit an action costing `cost` at epoch `now`. On
    /// success the pressure becomes `decayed_pressure + cost` and the
    /// decay clock resets to `now`. On `RateLimited` the bucket is left
    /// unchanged. `now` must not run backwards.
    pub fn try_consume(&mut self, cost: u64, now: u64) -> Result<(), RateError> {
        if now < self.last_update {
            return Err(RateError::NonMonotoneTime {
                incoming: now,
                last: self.last_update,
            });
        }
        let current = self.pressure_at(now);
        let after = current.saturating_add(cost);
        if after > self.capacity {
            return Err(RateError::RateLimited {
                current_pressure: current,
                cost,
                capacity: self.capacity,
            });
        }
        self.pressure = after;
        self.last_update = now;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> DecayBucket {
        DecayBucket::new(100, 10, 0).unwrap()
    }

    // ── construction ─────────────────────────────────────────────

    #[test]
    fn new_rejects_zero_capacity() {
        assert_eq!(DecayBucket::new(0, 10, 0).unwrap_err(), RateError::ZeroCapacity);
    }

    #[test]
    fn new_rejects_zero_half_life() {
        assert_eq!(DecayBucket::new(100, 0, 0).unwrap_err(), RateError::ZeroHalfLife);
    }

    #[test]
    fn fresh_bucket_is_empty_with_full_allowance() {
        let b = fresh();
        assert_eq!(b.pressure_at(0), 0);
        assert_eq!(b.available(0), 100);
        assert!(b.would_allow(100, 0));
    }

    // ── consume + capacity ceiling ───────────────────────────────

    #[test]
    fn consume_reduces_headroom() {
        let mut b = fresh();
        b.try_consume(30, 0).unwrap();
        assert_eq!(b.pressure_at(0), 30);
        assert_eq!(b.available(0), 70);
    }

    #[test]
    fn consume_up_to_exact_capacity_then_refuse() {
        let mut b = fresh();
        b.try_consume(100, 0).unwrap(); // exactly at capacity
        assert_eq!(b.pressure_at(0), 100);
        assert!(matches!(
            b.try_consume(1, 0),
            Err(RateError::RateLimited { .. })
        ));
    }

    #[test]
    fn cost_larger_than_capacity_always_denied() {
        let mut b = fresh();
        assert!(matches!(
            b.try_consume(101, 0),
            Err(RateError::RateLimited { .. })
        ));
        // and it didn't mutate
        assert_eq!(b.pressure_at(0), 0);
    }

    #[test]
    fn denied_action_does_not_mutate() {
        let mut b = fresh();
        b.try_consume(100, 0).unwrap();
        let before = b.clone();
        let _ = b.try_consume(50, 0); // denied
        assert_eq!(b, before);
    }

    // ── decay / recovery ─────────────────────────────────────────

    #[test]
    fn pressure_decays_at_half_life() {
        let mut b = fresh();
        b.try_consume(100, 0).unwrap();
        assert_eq!(b.pressure_at(10), 50); // one half-life
        assert_eq!(b.pressure_at(20), 25); // two
    }

    #[test]
    fn bucket_fully_recovers_after_many_half_lives() {
        let mut b = fresh();
        b.try_consume(100, 0).unwrap();
        assert_eq!(b.pressure_at(10_000), 0);
        assert!(b.would_allow(100, 10_000));
    }

    #[test]
    fn pressure_is_monotone_non_increasing_without_consume() {
        let mut b = fresh();
        b.try_consume(100, 0).unwrap();
        let a = b.pressure_at(5);
        let c = b.pressure_at(15);
        let d = b.pressure_at(40);
        assert!(a >= c && c >= d);
    }

    #[test]
    fn partial_recovery_admits_partial_burst() {
        let mut b = fresh();
        b.try_consume(100, 0).unwrap();
        // At t=10 pressure is 50 → 50 headroom.
        assert!(b.would_allow(50, 10));
        assert!(!b.would_allow(51, 10));
        b.try_consume(50, 10).unwrap();
        assert_eq!(b.pressure_at(10), 100);
    }

    // ── monotone time ─────────────────────────────────────────────

    #[test]
    fn non_monotone_time_rejected() {
        let mut b = fresh();
        b.try_consume(10, 100).unwrap();
        assert!(matches!(
            b.try_consume(10, 50),
            Err(RateError::NonMonotoneTime { .. })
        ));
    }

    #[test]
    fn zero_cost_is_a_noop_consume() {
        let mut b = fresh();
        b.try_consume(40, 5).unwrap();
        b.try_consume(0, 5).unwrap();
        assert_eq!(b.pressure_at(5), 40);
    }

    proptest::proptest! {
        /// An admitted action never pushes pressure above capacity, and
        /// pressure decays monotonically between actions.
        #[test]
        fn property_capacity_invariant_and_monotone_decay(
            capacity in 1u64..1_000_000u64,
            half_life in 1u64..10_000u64,
            cost in 0u64..1_000_000u64,
            t_consume in 0u64..100_000u64,
            dt in 0u64..100_000u64,
        ) {
            let mut b = DecayBucket::new(capacity, half_life, 0).unwrap();
            let res = b.try_consume(cost, t_consume);
            // Whatever the outcome, pressure never exceeds capacity.
            proptest::prop_assert!(b.pressure_at(t_consume) <= capacity);
            if res.is_ok() {
                // Pressure decays (non-increasing) after the action.
                let p1 = b.pressure_at(t_consume);
                let p2 = b.pressure_at(t_consume + dt);
                proptest::prop_assert!(p2 <= p1);
            }
        }
    }
}

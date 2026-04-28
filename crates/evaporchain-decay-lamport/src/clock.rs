//! `LamportClock` — energy-driven logical clock.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use thiserror::Error;

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LamportClock {
    /// Current logical tick — increments by 1 per `tick_quantum`
    /// energy spent.
    pub current_tick: u64,
    /// Energy accumulated since the last tick. Wraps to 0 each time
    /// the accumulator crosses `tick_quantum`.
    pub accumulated_energy: Energy,
    /// Energy required to advance one tick. Chain-set governance
    /// parameter; constant per node.
    pub tick_quantum: Energy,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TickError {
    #[error("tick_quantum is 0 — clock cannot advance")]
    ZeroQuantum,
}

impl LamportClock {
    pub const fn new(tick_quantum: Energy) -> Self {
        Self {
            current_tick: 0,
            accumulated_energy: 0,
            tick_quantum,
        }
    }

    /// Spend `energy_spent` units. Returns the new clock with
    /// `current_tick` incremented as many times as the accumulator
    /// crosses `tick_quantum`.
    pub fn tick(self, energy_spent: Energy) -> Result<Self, TickError> {
        if self.tick_quantum == 0 {
            return Err(TickError::ZeroQuantum);
        }
        let total = (self.accumulated_energy as u128).saturating_add(energy_spent as u128);
        let q = self.tick_quantum as u128;
        let advances = (total / q) as u64;
        let residual = (total % q) as Energy;
        Ok(Self {
            current_tick: self.current_tick.saturating_add(advances),
            accumulated_energy: residual,
            tick_quantum: self.tick_quantum,
        })
    }

    /// Lamport merge: take the max of two clocks. Used when receiving
    /// a message from another node — caller's clock is bumped to at
    /// least the message's clock.
    ///
    /// Tick is `max(a.current_tick, b.current_tick)`. Accumulator is
    /// reset to 0 (we don't try to combine residuals across nodes —
    /// that would require a shared `tick_quantum` agreement).
    pub fn merge(self, other: Self) -> Self {
        Self {
            current_tick: self.current_tick.max(other.current_tick),
            accumulated_energy: 0,
            tick_quantum: self.tick_quantum, // keep our own quantum
        }
    }

    /// Strict-precedence comparison. Returns `Less`/`Greater` only
    /// when `current_tick` differs; `Equal` ticks are *concurrent*
    /// regardless of accumulator.
    pub fn precedes(&self, other: &Self) -> Ordering {
        self.current_tick.cmp(&other.current_tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_clock_at_tick_zero() {
        let c = LamportClock::new(100);
        assert_eq!(c.current_tick, 0);
        assert_eq!(c.accumulated_energy, 0);
    }

    #[test]
    fn zero_quantum_rejected() {
        let c = LamportClock::new(0);
        assert_eq!(c.tick(1).unwrap_err(), TickError::ZeroQuantum);
    }

    #[test]
    fn small_spend_below_quantum_no_tick_advance() {
        let c = LamportClock::new(100);
        let after = c.tick(50).unwrap();
        assert_eq!(after.current_tick, 0);
        assert_eq!(after.accumulated_energy, 50);
    }

    #[test]
    fn spend_at_quantum_advances_one_tick() {
        let c = LamportClock::new(100);
        let after = c.tick(100).unwrap();
        assert_eq!(after.current_tick, 1);
        assert_eq!(after.accumulated_energy, 0);
    }

    #[test]
    fn cumulative_residual_eventually_advances() {
        let c = LamportClock::new(100);
        let c1 = c.tick(60).unwrap();
        assert_eq!(c1.current_tick, 0);
        let c2 = c1.tick(50).unwrap(); // 60+50=110 → 1 tick + 10 residual
        assert_eq!(c2.current_tick, 1);
        assert_eq!(c2.accumulated_energy, 10);
    }

    #[test]
    fn large_spend_advances_many_ticks() {
        let c = LamportClock::new(100);
        let after = c.tick(1050).unwrap();
        assert_eq!(after.current_tick, 10);
        assert_eq!(after.accumulated_energy, 50);
    }

    #[test]
    fn merge_takes_max_tick() {
        let a = LamportClock {
            current_tick: 3,
            accumulated_energy: 50,
            tick_quantum: 100,
        };
        let b = LamportClock {
            current_tick: 5,
            accumulated_energy: 10,
            tick_quantum: 100,
        };
        assert_eq!(a.merge(b).current_tick, 5);
        assert_eq!(b.merge(a).current_tick, 5);
    }

    #[test]
    fn precedes_orders_by_tick() {
        let a = LamportClock {
            current_tick: 3,
            accumulated_energy: 99,
            tick_quantum: 100,
        };
        let b = LamportClock {
            current_tick: 4,
            accumulated_energy: 0,
            tick_quantum: 100,
        };
        assert_eq!(a.precedes(&b), Ordering::Less);
        assert_eq!(b.precedes(&a), Ordering::Greater);
    }

    #[test]
    fn equal_ticks_concurrent_regardless_of_residual() {
        let a = LamportClock {
            current_tick: 3,
            accumulated_energy: 99,
            tick_quantum: 100,
        };
        let b = LamportClock {
            current_tick: 3,
            accumulated_energy: 1,
            tick_quantum: 100,
        };
        assert_eq!(a.precedes(&b), Ordering::Equal);
    }
}

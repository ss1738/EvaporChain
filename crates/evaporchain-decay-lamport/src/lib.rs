//! Decay-Lamport Time.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 3:
//!
//! > **Decay-Lamport Time** — Clock ticks by energy spent, not wall
//! > clock. Decentralized; no NTP, no PoH leader.
//!
//! Source attribution (causal-time agent, round 2): Lamport × VDF ×
//! thermodynamic accounting.
//!
//! ## Mechanics
//!
//! Lamport's logical clocks tick when a *send* event happens. Decay-
//! Lamport ticks when a *unit of energy is spent*: each call to
//! `tick(clock, energy_spent)` adds `energy_spent` to the clock's
//! accumulator and increments `current_tick` once for every full
//! `tick_quantum` worth of energy. Arrival events still merge clocks
//! (`merge(a, b) = max(a, b)` per Lamport).
//!
//! Two clocks are *comparable* iff one strictly precedes the other
//! (`a.current_tick < b.current_tick`). Concurrent clocks have
//! `==` ticks but possibly different `accumulated_energy` residuals.
//!
//! ## Module map
//!
//! - `clock` — `LamportClock { current_tick, accumulated_energy,
//!   tick_quantum }`. tick + merge.
//! - The accumulator and quantum live inside the clock so distributed
//!   replicas can converge on tick parity even without sharing a
//!   wall-clock.

pub mod clock;

pub use clock::{LamportClock, TickError};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use std::cmp::Ordering;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Decay-Lamport time ticks per energy spent, not
    /// wall clock. `tick(energy)` advances `current_tick` once per
    /// `tick_quantum` of accumulated spend. `merge(a, b)` takes the
    /// max tick (Lamport rule). Equal ticks are CONCURRENT regardless
    /// of accumulator residual; tick_quantum=0 fails closed."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let c0 = LamportClock::new(100);
        assert_eq!(c0.current_tick, 0);

        // Below quantum → no tick advance.
        let c1 = c0.tick(50).unwrap();
        assert_eq!(c1.current_tick, 0);
        assert_eq!(c1.accumulated_energy, 50);

        // Cumulative crossing → tick advances.
        let c2 = c1.tick(60).unwrap(); // 50+60=110 → 1 tick, 10 residual
        assert_eq!(c2.current_tick, 1);
        assert_eq!(c2.accumulated_energy, 10);

        // Large spend → many ticks.
        let big = c0.tick(1_050).unwrap();
        assert_eq!(big.current_tick, 10);
        assert_eq!(big.accumulated_energy, 50);

        // Merge takes max tick.
        let a = LamportClock {
            current_tick: 3,
            accumulated_energy: 99,
            tick_quantum: 100,
        };
        let b = LamportClock {
            current_tick: 5,
            accumulated_energy: 10,
            tick_quantum: 100,
        };
        assert_eq!(a.merge(b).current_tick, 5);
        assert_eq!(b.merge(a).current_tick, 5);

        // Equal ticks → concurrent (Equal), residual irrelevant.
        let p = LamportClock {
            current_tick: 3,
            accumulated_energy: 99,
            tick_quantum: 100,
        };
        let q = LamportClock {
            current_tick: 3,
            accumulated_energy: 1,
            tick_quantum: 100,
        };
        assert_eq!(p.precedes(&q), Ordering::Equal);

        // Distinct ticks → strict ordering.
        assert_eq!(a.precedes(&b), Ordering::Less);

        // Zero quantum fails closed.
        assert!(matches!(
            LamportClock::new(0).tick(1),
            Err(TickError::ZeroQuantum)
        ));
    }
}

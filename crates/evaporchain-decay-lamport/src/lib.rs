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

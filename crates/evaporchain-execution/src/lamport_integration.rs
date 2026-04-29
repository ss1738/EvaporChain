//! Decay-Lamport logical clock — wires `evaporchain-decay-lamport` into the
//! execution layer.
//!
//! The clock lives on `SimpleExecutor`.  Every block production call ticks it
//! with `gas_used` as the energy proxy:
//!
//! ```text
//! clock.tick(gas_used)  →  clock.current_tick may increment
//! ```
//!
//! `tick_quantum` is set to `GAS_TRANSFER` (21_000) so one full Transfer
//! costs 1 logical tick, giving the clock a meaningful and validator-invariant
//! rate across block sizes.
//!
//! Cross-block message ordering uses `clock.precedes(other)` for deterministic
//! causal ordering without a wall clock.

use evaporchain_decay_lamport::LamportClock;
use evaporchain_types::Energy;

/// Default tick quantum: energy required to advance the Decay-Lamport clock
/// by one tick.  Set to `GAS_TRANSFER` (21,000) so a single bare Transfer
/// costs exactly one logical clock unit.
pub const DEFAULT_TICK_QUANTUM: Energy = 21_000;

/// Build the chain's initial `LamportClock` at genesis.
pub fn genesis_clock() -> LamportClock {
    LamportClock::new(DEFAULT_TICK_QUANTUM)
}

/// Advance the `LamportClock` by `energy_spent` units.
///
/// Silently saturates on `TickError::Overflow` — a full u64 tick counter
/// implies the chain has produced ≫10^15 blocks; saturation is correct.
pub fn tick_block(clock: LamportClock, energy_spent: Energy) -> LamportClock {
    match clock.tick(energy_spent) {
        Ok(c) => c,
        Err(_) => clock, // overflow saturated — keep existing clock
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genesis_clock_starts_at_tick_zero() {
        let c = genesis_clock();
        assert_eq!(c.current_tick, 0);
        assert_eq!(c.accumulated_energy, 0);
    }

    #[test]
    fn one_transfer_advances_one_tick() {
        let c = genesis_clock();
        let c2 = tick_block(c, DEFAULT_TICK_QUANTUM);
        assert_eq!(c2.current_tick, 1);
    }

    #[test]
    fn below_quantum_no_tick() {
        let c = genesis_clock();
        let c2 = tick_block(c, DEFAULT_TICK_QUANTUM - 1);
        assert_eq!(c2.current_tick, 0);
        assert_eq!(c2.accumulated_energy, DEFAULT_TICK_QUANTUM - 1);
    }

    #[test]
    fn multi_block_accumulates() {
        let c = genesis_clock();
        let c2 = tick_block(c, DEFAULT_TICK_QUANTUM * 3 + 5_000);
        assert_eq!(c2.current_tick, 3);
    }

    #[test]
    fn tick_quantum_is_gas_transfer() {
        // Contractual: any change here must be a governance-level decision.
        assert_eq!(DEFAULT_TICK_QUANTUM, 21_000);
    }
}

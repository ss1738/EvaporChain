//! End-to-end Decay-Lamport fixture.
//!
//! Two replicas tick independently from a synthetic energy-spend
//! workload, then merge via Lamport's max-rule. The fixture asserts:
//!
//! 1. **Determinism** — same energy trace yields the same `current_tick`
//!    regardless of in-order vs batched accumulation.
//! 2. **Comparability** — strictly-ordered ticks compare via `Ord`.
//! 3. **Merge monotonicity** — `merge(a, b).current_tick ≥
//!    max(a.current_tick, b.current_tick)`.
//!
//! Doctrine: `INVENTION_STACK.md` §4.1 row 3 — clock ticks by energy
//! spent, not wall-clock; merges preserve causal ordering.

use evaporchain_decay_lamport::LamportClock;

const TICK_QUANTUM: u64 = 1_000;

fn tick_all(mut c: LamportClock, spends: &[u64]) -> LamportClock {
    for s in spends {
        c = c.tick(*s).unwrap();
    }
    c
}

#[test]
fn e2e_independent_replicas_merge_to_max() {
    // Replica A: spends 100 ten times → 1000 total → exactly 1 tick.
    let a = tick_all(LamportClock::new(TICK_QUANTUM), &[100; 10]);
    assert_eq!(a.current_tick, 1);
    assert_eq!(a.accumulated_energy, 0);

    // Replica B: one big spend of 3500 → 3 ticks, 500 residual.
    let b = LamportClock::new(TICK_QUANTUM).tick(3_500).unwrap();
    assert_eq!(b.current_tick, 3);
    assert_eq!(b.accumulated_energy, 500);

    // Merge: max-tick wins (Lamport rule).
    let merged = a.merge(b);
    assert_eq!(merged.current_tick, 3);
    // Residual is reset on merge — we don't combine residuals across
    // nodes (see `clock.rs` doc).
    assert_eq!(merged.accumulated_energy, 0);
}

#[test]
fn e2e_batched_equals_streamed() {
    // Determinism across granularities: 10×100 == 1×1000.
    let streamed = tick_all(LamportClock::new(TICK_QUANTUM), &[100; 10]);
    let batched = LamportClock::new(TICK_QUANTUM).tick(1_000).unwrap();
    assert_eq!(streamed.current_tick, batched.current_tick);
    assert_eq!(streamed.accumulated_energy, batched.accumulated_energy);
}

#[test]
fn e2e_ordering_chain_strictly_increasing() {
    let mut c = LamportClock::new(TICK_QUANTUM);
    let mut prev_tick = c.current_tick;
    for _ in 0..5 {
        c = c.tick(TICK_QUANTUM).unwrap();
        assert!(c.current_tick > prev_tick, "tick must strictly advance");
        prev_tick = c.current_tick;
    }
    assert_eq!(c.current_tick, 5);
}

#[test]
fn e2e_merge_of_equal_clocks_is_idempotent() {
    let a = LamportClock::new(TICK_QUANTUM).tick(2_500).unwrap();
    let merged = a.merge(a);
    assert_eq!(merged.current_tick, a.current_tick);
}

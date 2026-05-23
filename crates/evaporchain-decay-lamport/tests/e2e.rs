//! End-to-end integration tests for evaporchain-decay-lamport.
//!
//! Non-trivial fixture: 3-node validator cluster exchanging energy.
//!
//! Validators A, B, C each run a Decay-Lamport clock. They spend energy
//! on state transitions and merge clocks on message receipt.
//!
//!   Epoch 0: A spends 500k energy → A.tick rises to 1
//!   Epoch 1: B spends 300k energy → B.tick rises to 1
//!   Epoch 2: A spends 200k energy → A.tick stays 1 (below quantum)
//!   Epoch 3: A merges B's clock → A.tick = max(A, B) = 1; causal ordering holds
//!   Epoch 4: C spends 1.2M energy → C.tick rises to 2 (crosses quantum twice)
//!   Epoch 5: B merges C's clock → B.tick = max(1, 2) = 2
//!   Epoch 6: A merges B's clock → A.tick = max(1, 2) = 2 — transitivity
//!
//! Doctrine claim (INVENTION_STACK §4.1 row 3):
//!   "Clock ticks by energy spent, not wall clock — decentralized;
//!    no NTP, no PoH leader."
//!
//! Adversarial: a clock cannot be forged ahead without energy expenditure.
//!
//! INVENTION_STACK.md §4.1 Decay-Lamport Time.

use evaporchain_decay_lamport::LamportClock;

const TICK_QUANTUM: u64 = 500_000;

fn make_clock() -> LamportClock {
    LamportClock::new(TICK_QUANTUM)
}

// ── Fixture helpers ───────────────────────────────────────────────────────

fn spend(clock: LamportClock, energy: u64) -> LamportClock {
    clock.tick(energy).expect("tick should not overflow in test")
}

// ── Happy-path causal ordering ────────────────────────────────────────────

#[test]
fn three_node_causality_chain() {
    let a = spend(make_clock(), 500_000); // tick=1, accumulated=500k
    let b = spend(make_clock(), 300_000); // tick=0, accumulated=300k
    let c = spend(make_clock(), 1_200_000); // tick=2, accumulated=1.2M

    assert_eq!(a.current_tick, 1, "A: one full quantum spent");
    assert_eq!(b.current_tick, 0, "B: below quantum — no tick yet");
    assert_eq!(c.current_tick, 2, "C: two full quanta spent");

    // B merges C — B's tick advances to max(0, 2) = 2.
    let b_after_c = b.merge(c);
    assert_eq!(b_after_c.current_tick, 2);

    // A merges B-after-C — transitivity: A now knows C's causal past.
    let a_after_bc = a.merge(b_after_c);
    assert_eq!(a_after_bc.current_tick, 2, "transitivity: A observes C's causal past");
}

#[test]
fn merge_is_commutative() {
    let a = spend(make_clock(), 600_000);
    let b = spend(make_clock(), 1_100_000);
    assert_eq!(a.merge(b).current_tick, b.merge(a).current_tick);
}

#[test]
fn merge_is_idempotent() {
    let a = spend(make_clock(), 600_000);
    let a2 = a.merge(a);
    assert_eq!(a2.current_tick, a.current_tick);
    // merge() always resets accumulated_energy to 0 — residuals are not
    // combined across nodes (cross-node residual combination is undefined).
    assert_eq!(a2.accumulated_energy, 0);
}

#[test]
fn energy_accumulates_across_ticks() {
    let a = spend(make_clock(), 300_000);
    let a = spend(a, 400_000); // 300k + 400k = 700k total → 1 tick (500k quantum), 200k residual
    assert_eq!(a.current_tick, 1);
    // accumulated_energy is the residual after the last tick boundary, not total spent.
    assert_eq!(a.accumulated_energy, 200_000);
}

// ── Causal precedence ─────────────────────────────────────────────────────

#[test]
fn precedes_strictly_less() {
    use std::cmp::Ordering;
    let a = spend(make_clock(), 600_000); // tick=1
    let b = spend(make_clock(), 1_100_000); // tick=2
    assert_eq!(a.precedes(&b), Ordering::Less);
    assert_eq!(b.precedes(&a), Ordering::Greater);
}

#[test]
fn precedes_equal_ticks_concurrent() {
    use std::cmp::Ordering;
    let a = spend(make_clock(), 600_000);
    let b = spend(make_clock(), 700_000);
    // Both tick=1 — concurrent events.
    assert_eq!(a.precedes(&b), Ordering::Equal);
}

// ── Adversarial: no free ticks ────────────────────────────────────────────

#[test]
fn tick_zero_energy_does_not_advance() {
    let a = spend(make_clock(), 0);
    assert_eq!(a.current_tick, 0);
    assert_eq!(a.accumulated_energy, 0);
}

#[test]
fn overflow_guard_saturates_never_panics() {
    // tick_quantum=1 — every energy unit advances the tick.
    // accumulated + u64::MAX via saturating_add → current_tick saturates at
    // u64::MAX rather than wrapping or panicking. The only error path is
    // ZeroQuantum which quantum=1 does not trigger.
    let clock = LamportClock::new(1);
    let c = clock.tick(u64::MAX).expect("quantum=1 must not error");
    // quantum=1: advances = u64::MAX / 1 = u64::MAX; saturating_add(0, u64::MAX) = u64::MAX.
    assert_eq!(c.current_tick, u64::MAX);
}

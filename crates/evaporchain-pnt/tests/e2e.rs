//! End-to-end integration tests for evaporchain-pnt.
//!
//! Non-trivial fixture: a privacy DEX note-commitment lifecycle.
//!
//! Doctrine claim (INVENTION_STACK §4.2 — Phasing Nullifier Tree):
//!   "Bounded nullifier sets — kills monotone privacy-chain growth.
//!   Tornado/Aztec/Zcash all accumulate nullifiers forever: state size
//!   scales with CUMULATIVE activity. PNT bounds state by ACTIVE activity:
//!   after a full window rotation, the live_count tracks only the last
//!   `window_depth` phases, not all history."
//!
//! Privacy DEX scenario (window_depth = 3):
//!
//!   Phase 0: Alice deposits [A1, A2, A3], Bob deposits [B1, B2]
//!     window = [p0(5)]          live = 5
//!
//!   → advance →
//!
//!   Phase 1: Spend nullifiers S_A1, S_A2; Carol deposits [C1, C2, C3, C4]
//!     window = [p0(5), p1(6)]   live = 11
//!
//!   → advance →
//!
//!   Phase 2: Spend nullifiers S_B1, S_B2; Dave deposits [D1, D2, D3]
//!     window = [p0(5), p1(6), p2(5)]   live = 16
//!
//!   → advance → (phase 0 drops out)
//!
//!   Phase 3: Empty
//!     window = [p1(6), p2(5), p3(0)]   live = 11
//!
//!   A1, A2, A3, B1, B2 from phase 0 are FORGOTTEN.
//!   The chain accepts re-deposits against them (window lost them).
//!
//!   Monotone-growth comparison (Tornado/Aztec/Zcash model):
//!     After phase 3, monotone store would hold 16 nullifiers.
//!     PNT holds 11 — bounded by window × peak-phase-activity.

use evaporchain_pnt::{Nullifier, PhasedNullifierTree, PntError};

const DEPTH: usize = 3;

fn n(tag: u8) -> Nullifier { [tag; 32] }

// Phase-0 deposits
const A1: u8 = 0x10;  const A2: u8 = 0x11;  const A3: u8 = 0x12;
const B1: u8 = 0x20;  const B2: u8 = 0x21;
// Phase-1 spend nullifiers + Carol deposits
const SA1: u8 = 0x30; const SA2: u8 = 0x31;
const C1: u8 = 0x40;  const C2: u8 = 0x41;  const C3: u8 = 0x42;  const C4: u8 = 0x43;
// Phase-2 spend nullifiers + Dave deposits
const SB1: u8 = 0x50; const SB2: u8 = 0x51;
const D1: u8 = 0x60;  const D2: u8 = 0x61;  const D3: u8 = 0x62;

// ── Full privacy-DEX lifecycle ────────────────────────────────────────────────

#[test]
fn privacy_dex_four_phase_note_lifecycle() {
    let mut tree = PhasedNullifierTree::new(DEPTH).unwrap();
    assert_eq!(tree.current_phase, 0);
    assert_eq!(tree.live_count(), 0);

    // ── Phase 0: Alice + Bob deposit ────────────────────────────────
    for tag in [A1, A2, A3, B1, B2] {
        tree.insert_nullifier(n(tag)).unwrap();
    }
    assert_eq!(tree.live_count(), 5);
    assert!(tree.is_spent_in_window(&n(A1)));
    assert!(tree.is_spent_in_window(&n(B2)));

    // ── Phase 1: spend A1, A2; Carol deposits ────────────────────────
    tree.advance_phase();
    assert_eq!(tree.current_phase, 1);
    for tag in [SA1, SA2, C1, C2, C3, C4] {
        tree.insert_nullifier(n(tag)).unwrap();
    }
    assert_eq!(tree.live_count(), 11,   // p0(5) + p1(6)
        "window = [p0, p1] → 5 + 6 = 11 live nullifiers");
    // Phase-0 deposits still visible (window has depth 3).
    assert!(tree.is_spent_in_window(&n(A3)), "A3 still in window after one advance");

    // ── Phase 2: spend B1, B2; Dave deposits ─────────────────────────
    tree.advance_phase();
    assert_eq!(tree.current_phase, 2);
    for tag in [SB1, SB2, D1, D2, D3] {
        tree.insert_nullifier(n(tag)).unwrap();
    }
    assert_eq!(tree.live_count(), 16,   // p0(5) + p1(6) + p2(5)
        "window = [p0, p1, p2] → 16 live nullifiers");
    assert!(tree.is_spent_in_window(&n(A1)), "A1 still in window after two advances");

    // ── Phase 3: rotate — phase 0 drops out ──────────────────────────
    tree.advance_phase();
    assert_eq!(tree.current_phase, 3);
    assert_eq!(tree.live_count(), 11,   // p1(6) + p2(5) + p3(0)
        "window = [p1, p2, p3] → 11 live after phase-0 eviction");

    // Phase-0 nullifiers are now forgotten.
    for tag in [A1, A2, A3, B1, B2] {
        assert!(!tree.is_spent_in_window(&n(tag)),
            "phase-0 nullifier {tag:#x} must be forgotten after window rotation");
    }

    // Phase-1 and phase-2 nullifiers still live.
    assert!(tree.is_spent_in_window(&n(SA1)));
    assert!(tree.is_spent_in_window(&n(D3)));
}

// ── Bounded state: the core doctrine claim ────────────────────────────────────

#[test]
fn state_size_bounded_not_monotone_after_full_window_rotation() {
    // Tornado/Aztec/Zcash: after N phases of K nullifiers each → N*K total.
    // PNT (depth=D): after N (N>D) phases → D*K total.
    //
    // Demonstrate with D=3, K=10, N=7.
    const K: usize = 10;
    const N: usize = 7;
    let mut tree = PhasedNullifierTree::new(DEPTH).unwrap();

    // Each phase gets K unique nullifiers.
    for phase in 0u8..N as u8 {
        for slot in 0u8..K as u8 {
            // Tag = phase*16 + slot (stays unique across phases).
            let tag = phase.wrapping_mul(16).wrapping_add(slot);
            let mut nullifier = [0u8; 32];
            nullifier[0] = phase;
            nullifier[1] = slot;
            tree.insert_nullifier(nullifier).unwrap();
        }
        if phase < N as u8 - 1 {
            tree.advance_phase();
        }
    }

    // Monotone-growth alternative would hold N*K = 70 nullifiers.
    // PNT holds at most DEPTH*K = 30.
    assert!(tree.live_count() <= DEPTH * K,
        "PNT live_count {} must be bounded by window_depth({}) × K({}) = {}",
        tree.live_count(), DEPTH, K, DEPTH * K);

    // Exactly DEPTH phases' worth are live (last 3 phases × 10 each).
    assert_eq!(tree.live_count(), DEPTH * K,
        "after full rotation, exactly window_depth={DEPTH} phases live with {K} nullifiers each");
}

// ── Double-spend detection ────────────────────────────────────────────────────

#[test]
fn double_spend_rejected_immediately_within_same_phase() {
    let mut tree = PhasedNullifierTree::new(DEPTH).unwrap();
    tree.insert_nullifier(n(A1)).unwrap();
    let err = tree.insert_nullifier(n(A1)).unwrap_err();
    assert!(matches!(err, PntError::DoubleSpend { .. }),
        "immediate double-spend must be rejected");
}

#[test]
fn double_spend_rejected_across_phases_while_in_window() {
    // Nullifier inserted in phase 0; attempt re-spend in phase 1.
    let mut tree = PhasedNullifierTree::new(DEPTH).unwrap();
    tree.insert_nullifier(n(B1)).unwrap();
    tree.advance_phase();
    // Phase 0 still in window (depth=3).
    let err = tree.insert_nullifier(n(B1)).unwrap_err();
    assert!(matches!(err, PntError::DoubleSpend { .. }),
        "cross-phase double-spend must be rejected while nullifier is within the window");
}

#[test]
fn aged_out_nullifier_can_be_reinserted() {
    // After the window rotates past the phase that held the nullifier,
    // the chain forgets it — the nullifier can be reused (for a fresh deposit).
    let mut tree = PhasedNullifierTree::new(2).unwrap(); // depth=2 for faster rotation
    tree.insert_nullifier(n(A1)).unwrap();               // phase 0
    tree.advance_phase();                                 // window = [p0, p1]
    assert!(tree.is_spent_in_window(&n(A1)));
    tree.advance_phase();                                 // window = [p1, p2]; p0 dropped
    assert!(!tree.is_spent_in_window(&n(A1)),
        "nullifier must be forgotten after phase aged out");
    // Fresh re-insert succeeds.
    tree.insert_nullifier(n(A1)).unwrap();
    assert!(tree.is_spent_in_window(&n(A1)), "re-inserted nullifier must be live again");
}

// ── Window-depth boundary cases ───────────────────────────────────────────────

#[test]
fn depth_1_maximum_pruning_only_current_phase_live() {
    // depth=1: only the current phase's nullifiers are live.
    // Any nullifier from the previous phase is immediately forgotten on advance.
    let mut tree = PhasedNullifierTree::new(1).unwrap();
    tree.insert_nullifier(n(A1)).unwrap();
    assert!(tree.is_spent_in_window(&n(A1)));
    tree.advance_phase();
    assert!(!tree.is_spent_in_window(&n(A1)),
        "depth=1: nullifier must be forgotten immediately on first advance");
    // New phase: empty.
    assert_eq!(tree.live_count(), 0);
    // Re-insert works (window forgot A1).
    tree.insert_nullifier(n(A1)).unwrap();
}

#[test]
fn depth_equals_phase_count_nothing_yet_evicted() {
    // If window_depth = number of phases used, nothing is evicted yet.
    // Only the (depth+1)-th advance triggers eviction.
    let mut tree = PhasedNullifierTree::new(3).unwrap();
    tree.insert_nullifier(n(A1)).unwrap(); // phase 0
    tree.advance_phase();
    tree.insert_nullifier(n(B1)).unwrap(); // phase 1
    tree.advance_phase();
    tree.insert_nullifier(n(C1)).unwrap(); // phase 2
    // Window has exactly 3 phases — all live.
    assert_eq!(tree.live_count(), 3);
    assert!(tree.is_spent_in_window(&n(A1)), "A1 at phase-0 still in depth-3 window");
    // Advance to phase 3 — now phase 0 is evicted.
    tree.advance_phase();
    assert!(!tree.is_spent_in_window(&n(A1)), "A1 must be evicted on depth+1-th advance");
    assert!(tree.is_spent_in_window(&n(B1)), "B1 at phase-1 still in window");
    assert!(tree.is_spent_in_window(&n(C1)), "C1 at phase-2 still in window");
}

// ── Phase counter and live_count accounting ───────────────────────────────────

#[test]
fn phase_counter_increments_monotonically_on_each_advance() {
    let mut tree = PhasedNullifierTree::new(DEPTH).unwrap();
    for expected in 1u64..=10 {
        tree.advance_phase();
        assert_eq!(tree.current_phase, expected,
            "current_phase must increment by 1 on each advance");
    }
}

#[test]
fn live_count_exact_accounting_across_three_phases() {
    let mut tree = PhasedNullifierTree::new(3).unwrap();

    // Phase 0: 3 nullifiers.
    for tag in [A1, A2, A3] { tree.insert_nullifier(n(tag)).unwrap(); }
    assert_eq!(tree.live_count(), 3);

    // Advance to phase 1: 5 nullifiers.
    tree.advance_phase();
    for tag in [B1, B2, C1, C2, C3] { tree.insert_nullifier(n(tag)).unwrap(); }
    assert_eq!(tree.live_count(), 8, "p0(3) + p1(5) = 8");

    // Advance to phase 2: 2 nullifiers.
    tree.advance_phase();
    for tag in [D1, D2] { tree.insert_nullifier(n(tag)).unwrap(); }
    assert_eq!(tree.live_count(), 10, "p0(3) + p1(5) + p2(2) = 10");

    // Advance to phase 3: p0 dropped, phase 3 empty.
    tree.advance_phase();
    assert_eq!(tree.live_count(), 7, "p1(5) + p2(2) + p3(0) = 7 after p0 evicted");
}

// ── Adversarial construction guard ───────────────────────────────────────────

#[test]
fn adversarial_zero_depth_rejected_at_construction() {
    let err = PhasedNullifierTree::new(0).unwrap_err();
    assert!(matches!(err, PntError::ZeroDepth),
        "zero window_depth must be rejected with ZeroDepth error");
}

// ── Multi-user cross-phase isolation ─────────────────────────────────────────

#[test]
fn multiple_users_independent_nullifiers_no_cross_contamination() {
    // Alice's and Bob's nullifiers are independent — Alice's double-spend
    // attempt doesn't interfere with Bob's unrelated operations.
    let mut tree = PhasedNullifierTree::new(DEPTH).unwrap();

    tree.insert_nullifier(n(A1)).unwrap();
    tree.insert_nullifier(n(B1)).unwrap();

    // Alice attempts double-spend — rejected.
    assert!(tree.insert_nullifier(n(A1)).is_err());

    // Bob's next insert is unaffected.
    tree.insert_nullifier(n(B2)).unwrap();
    assert_eq!(tree.live_count(), 3, "Alice's failed insert must not corrupt state");
    assert!(tree.is_spent_in_window(&n(B2)));
}

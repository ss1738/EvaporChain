//! Coverage tests for Phasing Nullifier Tree (PNT) — Tier 2 substrate
//! per `INVENTION_STACK.md §4.2`. The chain's anti-monotone-growth
//! privacy primitive (Tornado/Aztec/Zcash all suffer this).
//!
//! Existing in-module tests cover the press-claim happy path. This
//! file adds:
//!
//!   - `current_phase` increment + saturation invariants
//!   - Window-length invariant under repeated advance
//!   - Cross-phase double-spend detection
//!   - `window_depth = 1` corner case (immediate forgetting)
//!   - Serde round-trip
//!   - `PntError` Display + Eq variant discrimination
//!   - Fresh-tree + empty-tree-advance no-panic
//!   - Different-nullifier insert chain

use evaporchain_pnt::{Nullifier, PhasedNullifierTree, PntError};

fn n(b: u8) -> Nullifier {
    [b; 32]
}

// =================================================================
// Phase counter invariants
// =================================================================

#[test]
fn current_phase_increments_by_one_on_advance() {
    let mut t = PhasedNullifierTree::new(3).unwrap();
    assert_eq!(t.current_phase, 0);
    t.advance_phase();
    assert_eq!(t.current_phase, 1);
    t.advance_phase();
    assert_eq!(t.current_phase, 2);
    t.advance_phase();
    assert_eq!(t.current_phase, 3);
}

#[test]
fn current_phase_saturates_at_u64_max() {
    let mut t = PhasedNullifierTree::new(2).unwrap();
    t.current_phase = u64::MAX;
    // saturating_add(1) on u64::MAX → u64::MAX (no panic).
    t.advance_phase();
    assert_eq!(t.current_phase, u64::MAX);
}

// =================================================================
// Window length invariants
// =================================================================

#[test]
fn window_length_never_exceeds_depth_under_repeated_advance() {
    let depth = 3;
    let mut t = PhasedNullifierTree::new(depth).unwrap();
    // Initial: 1 phase (the current one).
    assert_eq!(t.window.len(), 1);
    // After 1 advance: 2 phases.
    t.advance_phase();
    assert_eq!(t.window.len(), 2);
    // After 2 advances: 3 phases (depth).
    t.advance_phase();
    assert_eq!(t.window.len(), 3);
    // After many more advances: still capped at depth.
    for _ in 0..50 {
        t.advance_phase();
        assert_eq!(t.window.len(), depth, "window must never exceed depth");
    }
}

#[test]
fn window_depth_one_immediately_forgets_on_advance() {
    let mut t = PhasedNullifierTree::new(1).unwrap();
    t.insert_nullifier(n(1)).unwrap();
    assert!(t.is_spent_in_window(&n(1)));
    t.advance_phase(); // window = [new empty phase]; phase 0 dropped
    assert!(
        !t.is_spent_in_window(&n(1)),
        "depth=1 must forget the prior phase immediately"
    );
    // And re-insert is allowed.
    t.insert_nullifier(n(1)).unwrap();
}

// =================================================================
// Cross-phase double-spend
// =================================================================

#[test]
fn double_spend_detected_across_phases_within_window() {
    let mut t = PhasedNullifierTree::new(3).unwrap();
    t.insert_nullifier(n(7)).unwrap(); // phase 0
    t.advance_phase();
    // Same nullifier in phase 1 must still be rejected (still in window).
    let err = t.insert_nullifier(n(7)).unwrap_err();
    assert!(matches!(err, PntError::DoubleSpend { .. }));
}

#[test]
fn double_spend_error_carries_offending_nullifier() {
    let mut t = PhasedNullifierTree::new(2).unwrap();
    let target = n(42);
    t.insert_nullifier(target).unwrap();
    match t.insert_nullifier(target) {
        Err(PntError::DoubleSpend { n: returned }) => assert_eq!(returned, target),
        other => panic!("expected DoubleSpend, got {other:?}"),
    }
}

// =================================================================
// Misc behavior
// =================================================================

#[test]
fn is_spent_in_window_false_for_unknown_nullifier() {
    let mut t = PhasedNullifierTree::new(2).unwrap();
    t.insert_nullifier(n(1)).unwrap();
    assert!(!t.is_spent_in_window(&n(99)), "unknown nullifier reads as unspent");
}

#[test]
fn advance_phase_on_empty_tree_does_not_panic() {
    let mut t = PhasedNullifierTree::new(2).unwrap();
    t.advance_phase();
    t.advance_phase();
    t.advance_phase();
    assert_eq!(t.window.len(), 2);
    assert_eq!(t.live_count(), 0);
}

#[test]
fn distinct_nullifiers_in_same_phase_all_retained() {
    let mut t = PhasedNullifierTree::new(1).unwrap();
    for i in 0..16u8 {
        t.insert_nullifier(n(i)).unwrap();
    }
    assert_eq!(t.live_count(), 16);
    for i in 0..16u8 {
        assert!(t.is_spent_in_window(&n(i)));
    }
}

// =================================================================
// Serde round-trip
// =================================================================

#[test]
fn tree_serde_round_trips_preserves_state() {
    let mut t = PhasedNullifierTree::new(3).unwrap();
    t.insert_nullifier(n(1)).unwrap();
    t.insert_nullifier(n(2)).unwrap();
    t.advance_phase();
    t.insert_nullifier(n(3)).unwrap();

    let json = serde_json::to_string(&t).expect("serialize");
    let back: PhasedNullifierTree = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(t, back, "round-trip must preserve full state");
    assert_eq!(back.current_phase, 1);
    assert_eq!(back.window.len(), 2);
    assert_eq!(back.live_count(), 3);
    assert!(back.is_spent_in_window(&n(1)));
    assert!(back.is_spent_in_window(&n(3)));
}

// =================================================================
// PntError ergonomics
// =================================================================

#[test]
fn pnt_error_displays_both_variants() {
    let zd = PntError::ZeroDepth.to_string();
    let ds = PntError::DoubleSpend { n: n(7) }.to_string();
    assert!(zd.contains("window_depth") || zd.contains("depth"), "got: {zd}");
    assert!(ds.contains("double-spend") || ds.contains("nullifier"), "got: {ds}");
}

#[test]
fn pnt_error_eq_discriminates_variants_and_payloads() {
    // PntError derives PartialEq + Eq + Debug but NOT Clone.
    let ds7 = PntError::DoubleSpend { n: n(7) };
    let ds7_again = PntError::DoubleSpend { n: n(7) };
    let ds8 = PntError::DoubleSpend { n: n(8) };
    let zd = PntError::ZeroDepth;
    assert_eq!(ds7, ds7_again);
    assert_ne!(ds7, ds8, "different nullifier payloads must be Ne");
    assert_ne!(ds7, zd, "different variants must be Ne");
}

// =================================================================
// State invariants under stress
// =================================================================

#[test]
fn live_count_matches_sum_of_unique_inserts_across_window() {
    // Insert 10 per phase, but advance only BETWEEN phases (not after
    // the last). Depth=4 holds all 4 phases of 10 = 40 live nullifiers.
    let mut t = PhasedNullifierTree::new(4).unwrap();
    let mut total = 0usize;
    for phase in 0..4u8 {
        for i in 0..10u8 {
            t.insert_nullifier(n(phase * 10 + i)).unwrap();
            total += 1;
        }
        // Advance only if this is NOT the last phase.
        if phase < 3 {
            t.advance_phase();
        }
    }
    assert_eq!(t.live_count(), total, "all 4 phases of 10 entries still live");
    assert_eq!(t.window.len(), 4);
    assert_eq!(t.current_phase, 3, "advanced 3 times → current_phase = 3");
}

#[test]
fn oldest_phase_drops_when_window_saturates_on_advance() {
    // depth=2. Build [p0(3), p1(3)]:
    //   - insert 3 in p0; advance → [p0, p1=empty]
    //   - insert 3 in p1; advance → window already at depth=2; pop p0;
    //     push fresh → window = [p1(3), p2=empty]
    // Phase 0's 3 entries dropped. Phase 1's 3 still live.
    let mut t = PhasedNullifierTree::new(2).unwrap();
    // Phase 0: nullifiers 0..3 (template `phase * 10 + i` with phase=0).
    for i in 0..3u8 { t.insert_nullifier(n(i)).unwrap(); }
    t.advance_phase();
    for i in 0..3u8 { t.insert_nullifier(n(10 + i)).unwrap(); }
    t.advance_phase(); // saturates: pops phase 0

    assert_eq!(t.live_count(), 3, "only phase 1's 3 entries still live");
    // Phase 0's nullifier (n(0)) is forgotten.
    assert!(!t.is_spent_in_window(&n(0)));
    assert!(!t.is_spent_in_window(&n(1)));
    assert!(!t.is_spent_in_window(&n(2)));
    // Phase 1's nullifier (n(10)) still live.
    assert!(t.is_spent_in_window(&n(10)));
    assert!(t.is_spent_in_window(&n(11)));
    assert!(t.is_spent_in_window(&n(12)));
}

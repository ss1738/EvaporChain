//! End-to-end integration tests for evaporchain-singh-lineage.
//!
//! Non-trivial fixture: two-child digital estate, graduated dormancy.
//!
//!   Issuer: Alice  (addr=0xAA)
//!   Child 1: Carol (addr=0xCC), weight=5_000bp (50%)
//!   Child 2: Dave  (addr=0xDD), weight=5_000bp (50%)
//!
//!   Ladder: doctrine_default(1) — 1 epoch == 1 "day":
//!     90 epochs  → share=2_500bp (25%)
//!     180 epochs → share=5_000bp (50%)
//!     365 epochs → share=10_000bp (100%)
//!
//!   Authority formula: share_bp × weight_bp / FULL_AUTHORITY_BP.
//!
//!   Epoch   0: Register. Add Carol + Dave.
//!   Epoch  50: Living.   Total authority = 0.
//!   Epoch  90: Tier 1.   Each child = 2_500 × 5_000/10_000 = 1_250.  Total = 2_500.
//!   Epoch 180: Tier 2.   Each = 5_000 × 5_000/10_000 = 2_500.        Total = 5_000.
//!   Epoch 365: Tier 3.   Each = 10_000 × 5_000/10_000 = 5_000.       Total = 10_000.
//!   Epoch 365: Touch.    dormancy resets to 0. authority snaps back to 0.
//!   Epoch 455 (365+90): Tier 1 again.  Each = 1_250. (Clock was reset.)
//!
//! Doctrine claim (INVENTION_STACK.md §A5.4 / Singh-Lineage):
//! "Crypto solves inheritance. Graduated dormancy: 25% at 90 days,
//!  50% at 180 days, full at 365 days. Any signed transaction from
//!  the issuer resets the clock. Total authority across heirs never
//!  exceeds FULL_AUTHORITY_BP (10_000 bp). Per-asset designation."
//!
//! Adversarial fixture: OverWeighted, DuplicateSuccessor, NotIssuer,
//! UnknownSuccessor, TouchBackwardsInTime, empty Ladder.
//!
//! INVENTION_STACK §A5.4: Singh-Lineage.

use evaporchain_singh_lineage::{
    DormancyTier, Ladder, LadderError, Lineage, LineageError, LineageId, Successor,
};

// ── Helpers ───────────────────────────────────────────────────────────────

fn lid(b: u8) -> LineageId {
    [b; 32]
}
fn alice() -> [u8; 32] {
    [0xAA; 32]
}
fn carol() -> [u8; 32] {
    [0xCC; 32]
}
fn dave() -> [u8; 32] {
    [0xDD; 32]
}
fn eve() -> [u8; 32] {
    [0xEE; 32]
}

fn carol_50pct() -> Successor {
    Successor::new(carol(), 5_000, "carol", alice()).unwrap()
}
fn dave_50pct() -> Successor {
    Successor::new(dave(), 5_000, "dave", alice()).unwrap()
}

fn two_child_estate() -> Lineage {
    let ladder = Ladder::doctrine_default(1).unwrap();
    let mut l = Lineage::register(lid(1), alice(), ladder, 0);
    l.add_successor(alice(), carol_50pct()).unwrap();
    l.add_successor(alice(), dave_50pct()).unwrap();
    l
}

// ── Non-trivial fixture ───────────────────────────────────────────────────

#[test]
fn fixture_living_issuer_has_zero_authority() {
    let l = two_child_estate();
    // Before first dormancy tier (epoch < 90): zero authority.
    assert_eq!(l.authority_at(0, &carol()), 0);
    assert_eq!(l.authority_at(50, &carol()), 0);
    assert_eq!(l.authority_at(89, &carol()), 0);
    assert_eq!(l.total_authority_at(50), 0);
}

#[test]
fn fixture_tier1_at_90_epochs_grants_25pct_share() {
    // dormancy=90: share=2_500bp. Each child: 2_500 × 5_000/10_000 = 1_250.
    let l = two_child_estate();
    assert_eq!(l.authority_at(90, &carol()), 1_250);
    assert_eq!(l.authority_at(90, &dave()), 1_250);
    assert_eq!(
        l.total_authority_at(90),
        2_500,
        "total = share × 1.00 (two 50% heirs)"
    );
}

#[test]
fn fixture_tier2_at_180_epochs_grants_50pct_share() {
    let l = two_child_estate();
    assert_eq!(l.authority_at(180, &carol()), 2_500);
    assert_eq!(l.authority_at(180, &dave()), 2_500);
    assert_eq!(l.total_authority_at(180), 5_000);
}

#[test]
fn fixture_tier3_at_365_epochs_grants_full_authority() {
    // dormancy=365: share=10_000bp (full). Each child: 5_000bp. Total = 10_000.
    let l = two_child_estate();
    assert_eq!(l.authority_at(365, &carol()), 5_000);
    assert_eq!(l.authority_at(365, &dave()), 5_000);
    let total = l.total_authority_at(365);
    assert_eq!(
        total, 10_000,
        "total at full dormancy must == FULL_AUTHORITY_BP"
    );
    // Heirs sum to full — no double-spend.
    assert_eq!(
        l.authority_at(365, &carol()) + l.authority_at(365, &dave()),
        10_000
    );
}

#[test]
fn fixture_touch_resets_clock_and_authority_to_zero() {
    let mut l = two_child_estate();
    // At epoch 365: full dormancy. Touch resets clock.
    assert_eq!(l.authority_at(365, &carol()), 5_000);
    l.touch(alice(), 365).unwrap();
    assert_eq!(l.last_seen_epoch, 365);
    // Immediately after touch: dormancy=0 → authority=0.
    assert_eq!(l.authority_at(365, &carol()), 0);
    // At epoch 365+89=454: dormancy=89, below first tier → authority=0.
    assert_eq!(l.authority_at(454, &carol()), 0);
    // At epoch 365+90=455: dormancy=90, tier 1 → authority=1_250.
    assert_eq!(l.authority_at(455, &carol()), 1_250);
}

#[test]
fn fixture_graduated_accrual_across_all_tiers() {
    // Monotone: authority never decreases as dormancy grows.
    let l = two_child_estate();
    let epochs = [0u64, 50, 89, 90, 100, 179, 180, 200, 364, 365, 1_000];
    let mut prev = 0u32;
    for &e in &epochs {
        let auth = l.authority_at(e, &carol());
        assert!(
            auth >= prev,
            "authority not monotone at epoch {e}: {auth} < {prev}"
        );
        prev = auth;
    }
}

#[test]
fn fixture_dormancy_epochs_measures_silence() {
    let mut l = Lineage::register(lid(2), alice(), Ladder::doctrine_default(1).unwrap(), 0);
    assert_eq!(l.dormancy_epochs(0), 0);
    assert_eq!(l.dormancy_epochs(100), 100);
    l.touch(alice(), 50).unwrap();
    assert_eq!(l.dormancy_epochs(50), 0);
    assert_eq!(l.dormancy_epochs(90), 40);
    assert_eq!(l.dormancy_epochs(20), 0); // saturating: clock-skew defensive
}

#[test]
fn fixture_remove_successor_reduces_total_authority() {
    let mut l = two_child_estate();
    l.remove_successor(alice(), carol()).unwrap();
    // Only Dave remains at 50% weight. Total at full dormancy = 5_000.
    assert_eq!(l.total_authority_at(365), 5_000);
    assert_eq!(
        l.authority_at(365, &carol()),
        0,
        "removed successor has zero authority"
    );
}

// ── Doctrine tests ────────────────────────────────────────────────────────

#[test]
fn doctrine_total_authority_never_exceeds_full() {
    let l = two_child_estate();
    for epoch in [0u64, 50, 90, 180, 365, 10_000] {
        assert!(
            l.total_authority_at(epoch) <= 10_000,
            "total must never exceed FULL_AUTHORITY_BP at epoch {epoch}"
        );
    }
}

#[test]
fn doctrine_unknown_successor_returns_zero() {
    let l = two_child_estate();
    assert_eq!(
        l.authority_at(365, &eve()),
        0,
        "unknown address → zero authority"
    );
    assert_eq!(l.authority_at(365, &[0xFF; 32]), 0);
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_over_weighted_successors_rejected() {
    let mut l = Lineage::register(lid(3), alice(), Ladder::doctrine_default(1).unwrap(), 0);
    l.add_successor(alice(), carol_50pct()).unwrap(); // 5_000bp
                                                      // Dave would push total to 10_000bp (just fine).
    l.add_successor(alice(), dave_50pct()).unwrap(); // total=10_000bp
                                                     // Eve would push over FULL_AUTHORITY_BP.
    let eve_tiny = Successor::new(eve(), 1, "eve", alice()).unwrap();
    let err = l.add_successor(alice(), eve_tiny).unwrap_err();
    assert!(matches!(err, LineageError::OverWeighted { .. }));
}

#[test]
fn adversarial_duplicate_successor_rejected() {
    let mut l = Lineage::register(lid(4), alice(), Ladder::doctrine_default(1).unwrap(), 0);
    let c1 = Successor::new(carol(), 3_000, "carol", alice()).unwrap();
    l.add_successor(alice(), c1).unwrap();
    let c2 = Successor::new(carol(), 3_000, "carol-dup", alice()).unwrap();
    let err = l.add_successor(alice(), c2).unwrap_err();
    assert!(matches!(err, LineageError::DuplicateSuccessor(_)));
}

#[test]
fn adversarial_add_successor_requires_issuer() {
    let mut l = Lineage::register(lid(5), alice(), Ladder::doctrine_default(1).unwrap(), 0);
    let err = l.add_successor(carol(), carol_50pct()).unwrap_err();
    assert!(matches!(err, LineageError::NotIssuer { .. }));
}

#[test]
fn adversarial_remove_nonexistent_successor_rejected() {
    let mut l = two_child_estate();
    let err = l.remove_successor(alice(), eve()).unwrap_err();
    assert!(matches!(err, LineageError::UnknownSuccessor(_)));
}

#[test]
fn adversarial_touch_requires_issuer() {
    let mut l = two_child_estate();
    let err = l.touch(carol(), 100).unwrap_err();
    assert!(matches!(err, LineageError::NotIssuer { .. }));
}

#[test]
fn adversarial_touch_backwards_in_time_rejected() {
    let mut l = two_child_estate();
    l.touch(alice(), 100).unwrap();
    let err = l.touch(alice(), 50).unwrap_err();
    assert!(matches!(
        err,
        LineageError::TouchBackwardsInTime {
            now: 50,
            last_seen: 100
        }
    ));
}

#[test]
fn adversarial_empty_ladder_rejected() {
    assert_eq!(Ladder::new(vec![]).unwrap_err(), LadderError::Empty);
}

#[test]
fn adversarial_non_monotone_ladder_threshold_rejected() {
    let err = Ladder::new(vec![
        DormancyTier {
            epochs_dormant: 100,
            authority_share_bp: 2_500,
        },
        DormancyTier {
            epochs_dormant: 50,
            authority_share_bp: 5_000,
        },
    ])
    .unwrap_err();
    assert!(matches!(err, LadderError::NonMonotoneThreshold { .. }));
}

#[test]
fn adversarial_decreasing_authority_in_ladder_rejected() {
    let err = Ladder::new(vec![
        DormancyTier {
            epochs_dormant: 100,
            authority_share_bp: 5_000,
        },
        DormancyTier {
            epochs_dormant: 200,
            authority_share_bp: 2_500,
        },
    ])
    .unwrap_err();
    assert!(matches!(err, LadderError::AuthorityDecreased { .. }));
}

// ── Cross-cuts ───────────────────────────────────────────────────────────

#[test]
fn serde_round_trip_preserves_full_lineage() {
    let l = two_child_estate();
    let json = serde_json::to_string(&l).unwrap();
    let back: Lineage = serde_json::from_str(&json).unwrap();
    assert_eq!(l, back);
}

#[test]
fn authority_queries_are_deterministic() {
    let l = two_child_estate();
    assert_eq!(
        l.authority_at(365, &carol()),
        l.authority_at(365, &carol()),
        "authority_at must be deterministic"
    );
}

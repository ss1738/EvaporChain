//! GraveGraph V2 split Dedications e2e — the Woolf Literary Estate
//!
//! Scenario: author WOOLF declares a split Legacy edge across 4 beneficiaries
//! (Leonard 40%, Vanessa 30%, Octavia 20%, Vita 10%) at epoch 5. WOOLF dies
//! at epoch 1941. Each survivor curates their Dedication independently;
//! the chain tracks total_share_paid_bp and refuses all further claims once
//! the split reaches 10_000 bp.

use evaporchain_grave_graph_split::{
    split::Curation, SplitError, SplitId, SplitLegacy, SplitState,
};

fn woolf() -> [u8; 32] {
    [0xAA; 32]
}
fn leonard() -> [u8; 32] {
    [0xBB; 32]
}
fn vanessa() -> [u8; 32] {
    [0xCC; 32]
}
fn octavia() -> [u8; 32] {
    [0xDD; 32]
}
fn vita() -> [u8; 32] {
    [0xEE; 32]
}
fn stranger() -> [u8; 32] {
    [0xFF; 32]
}
fn sid(n: u8) -> SplitId {
    SplitId([n; 32])
}

/// WOOLF estate: Leonard 40%, Vanessa 30%, Octavia 20%, Vita 10%.
fn woolf_estate() -> SplitLegacy {
    SplitLegacy::declare(
        sid(1),
        woolf(),
        vec![
            (leonard(), 4_000),
            (vanessa(), 3_000),
            (octavia(), 2_000),
            (vita(), 1_000),
        ],
        5,
    )
    .unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn woolf_estate_full_lifecycle() {
    let mut estate = woolf_estate();

    // Declared but source still alive
    assert!(matches!(estate.state, SplitState::Pending));
    assert_eq!(estate.declared_at_epoch, 5);
    assert_eq!(estate.total_share_paid_bp, 0);
    assert_eq!(estate.dedications.len(), 4);
    assert!(!estate.is_fully_distributed());

    // Curating before death is rejected
    assert!(matches!(
        estate.curate(leonard(), Curation::Accepted),
        Err(SplitError::NotInverted)
    ));

    // WOOLF dies at epoch 1941
    estate.certify_source_death(1941);
    assert!(matches!(
        estate.state,
        SplitState::Inverted {
            died_at_epoch: 1941
        }
    ));

    // Each beneficiary curates independently
    estate.curate(leonard(), Curation::Accepted).unwrap();
    assert_eq!(estate.total_share_paid_bp, 4_000);
    assert!(!estate.is_fully_distributed());

    estate.curate(vanessa(), Curation::Rejected).unwrap();
    assert_eq!(estate.total_share_paid_bp, 7_000);

    estate.curate(octavia(), Curation::Hidden).unwrap();
    assert_eq!(estate.total_share_paid_bp, 9_000);

    estate.curate(vita(), Curation::Accepted).unwrap();
    assert_eq!(estate.total_share_paid_bp, 10_000);
    assert!(estate.is_fully_distributed());

    // No further claims accepted after full distribution
    assert!(matches!(
        estate.curate(leonard(), Curation::Hidden),
        Err(SplitError::AlreadyCurated(_))
    ));
}

#[test]
fn share_accounting_exact() {
    let estate = woolf_estate();
    assert_eq!(estate.share_for(leonard()), Some(4_000));
    assert_eq!(estate.share_for(vanessa()), Some(3_000));
    assert_eq!(estate.share_for(octavia()), Some(2_000));
    assert_eq!(estate.share_for(vita()), Some(1_000));
    assert_eq!(estate.share_for(stranger()), None);
    let total: u64 = estate.dedications.iter().map(|d| d.share_bp).sum();
    assert_eq!(
        total, 10_000,
        "declared shares must sum to exactly 10_000 bp"
    );
}

#[test]
fn partial_distribution_tracks_unclaimed_slots() {
    // Only Leonard and Vanessa curate; Octavia and Vita remain pending.
    let mut estate = woolf_estate();
    estate.certify_source_death(1941);

    estate.curate(leonard(), Curation::Accepted).unwrap();
    estate.curate(vanessa(), Curation::Rejected).unwrap();

    assert_eq!(estate.total_share_paid_bp, 7_000);
    assert!(!estate.is_fully_distributed());

    let oct_ded = estate
        .dedications
        .iter()
        .find(|d| d.recipient == octavia())
        .unwrap();
    assert!(!oct_ded.claimed, "Octavia's slot must remain unclaimed");
    assert!(matches!(oct_ded.curation, Curation::Pending));
}

#[test]
fn pending_curation_is_not_a_real_claim() {
    // Curation::Pending is a no-op; total does not increment; slot stays open.
    let mut estate = woolf_estate();
    estate.certify_source_death(1941);

    estate.curate(leonard(), Curation::Pending).unwrap();
    assert_eq!(
        estate.total_share_paid_bp, 0,
        "Pending curation must not increment share"
    );

    let ded = estate
        .dedications
        .iter()
        .find(|d| d.recipient == leonard())
        .unwrap();
    assert!(!ded.claimed, "Pending must not mark slot claimed");

    // Leonard can still submit a real curation afterwards
    estate.curate(leonard(), Curation::Accepted).unwrap();
    assert_eq!(estate.total_share_paid_bp, 4_000);
}

#[test]
fn curation_order_does_not_affect_final_total() {
    let mut fwd = woolf_estate();
    fwd.certify_source_death(1941);
    fwd.curate(leonard(), Curation::Accepted).unwrap();
    fwd.curate(vanessa(), Curation::Rejected).unwrap();
    fwd.curate(octavia(), Curation::Hidden).unwrap();
    fwd.curate(vita(), Curation::Accepted).unwrap();

    let mut rev = woolf_estate();
    rev.certify_source_death(1941);
    rev.curate(vita(), Curation::Accepted).unwrap();
    rev.curate(octavia(), Curation::Hidden).unwrap();
    rev.curate(vanessa(), Curation::Rejected).unwrap();
    rev.curate(leonard(), Curation::Accepted).unwrap();

    assert_eq!(fwd.total_share_paid_bp, rev.total_share_paid_bp);
    assert!(fwd.is_fully_distributed());
    assert!(rev.is_fully_distributed());
}

#[test]
fn two_independent_legacies_do_not_interfere() {
    // shared_r appears in both legacies; curating on one must not affect the other.
    let source_a = [0x01; 32];
    let source_b = [0x02; 32];
    let shared_r = [0x10; 32];

    let mut leg_a = SplitLegacy::declare(
        sid(1),
        source_a,
        vec![(shared_r, 7_000), (leonard(), 3_000)],
        0,
    )
    .unwrap();
    let mut leg_b = SplitLegacy::declare(
        sid(2),
        source_b,
        vec![(shared_r, 5_000), (vanessa(), 5_000)],
        0,
    )
    .unwrap();

    leg_a.certify_source_death(100);
    leg_b.certify_source_death(200);

    leg_a.curate(shared_r, Curation::Accepted).unwrap();
    assert_eq!(leg_a.total_share_paid_bp, 7_000);
    assert_eq!(
        leg_b.total_share_paid_bp, 0,
        "leg_b must be unaffected by leg_a curations"
    );

    leg_b.curate(shared_r, Curation::Rejected).unwrap();
    assert_eq!(leg_b.total_share_paid_bp, 5_000);

    // leg_a slot for shared_r is already claimed
    assert!(matches!(
        leg_a.curate(shared_r, Curation::Rejected),
        Err(SplitError::AlreadyCurated(_))
    ));
}

#[test]
fn single_recipient_full_stake_split() {
    let mut leg = SplitLegacy::declare(sid(9), woolf(), vec![(leonard(), 10_000)], 0).unwrap();
    assert_eq!(leg.dedications.len(), 1);

    leg.certify_source_death(50);
    leg.curate(leonard(), Curation::Accepted).unwrap();

    assert_eq!(leg.total_share_paid_bp, 10_000);
    assert!(leg.is_fully_distributed());
}

#[test]
fn ten_way_equal_split_fully_distributes() {
    // 10 recipients × 1000 bp = 10_000 bp exactly.
    let source = [0x00; 32];
    let recipients: Vec<[u8; 32]> = (1u8..=10).map(|i| [i; 32]).collect();
    let spec: Vec<([u8; 32], u64)> = recipients.iter().map(|&r| (r, 1_000)).collect();

    let mut leg = SplitLegacy::declare(sid(10), source, spec, 0).unwrap();
    assert_eq!(leg.dedications.len(), 10);

    leg.certify_source_death(999);
    for &r in &recipients {
        leg.curate(r, Curation::Accepted).unwrap();
    }

    assert_eq!(leg.total_share_paid_bp, 10_000);
    assert!(leg.is_fully_distributed());
}

#[test]
fn adversarial_stranger_cannot_claim_existing_slot() {
    let mut estate = woolf_estate();
    estate.certify_source_death(1941);

    let err = estate.curate(stranger(), Curation::Accepted).unwrap_err();
    assert!(
        matches!(err, SplitError::UnknownRecipient(_)),
        "stranger must receive UnknownRecipient, got {:?}",
        err
    );
    // No bp leaked
    assert_eq!(estate.total_share_paid_bp, 0);
}

#[test]
fn adversarial_double_claim_blocked_at_any_curation_variant() {
    // First curation accepted; subsequent attempts with any variant are blocked.
    let mut estate = woolf_estate();
    estate.certify_source_death(1941);
    estate.curate(leonard(), Curation::Accepted).unwrap();

    for choice in [Curation::Accepted, Curation::Rejected, Curation::Hidden] {
        assert!(
            matches!(
                estate.curate(leonard(), choice),
                Err(SplitError::AlreadyCurated(_))
            ),
            "double-claim with {:?} must be rejected",
            choice
        );
    }
    // Share did not double-count
    assert_eq!(estate.total_share_paid_bp, 4_000);
}

#[test]
fn declaration_guards_zero_share_duplicate_and_self() {
    // Zero share at position 1 → ZeroShare(1)
    assert!(matches!(
        SplitLegacy::declare(
            sid(1),
            woolf(),
            vec![(leonard(), 5_000), (vanessa(), 0), (octavia(), 5_000)],
            0,
        )
        .unwrap_err(),
        SplitError::ZeroShare(1)
    ));

    // Duplicate recipient → DuplicateRecipient
    assert!(matches!(
        SplitLegacy::declare(
            sid(2),
            woolf(),
            vec![(leonard(), 5_000), (leonard(), 5_000)],
            0,
        )
        .unwrap_err(),
        SplitError::DuplicateRecipient(_)
    ));

    // Source tries to add themselves → SelfRecipient
    assert_eq!(
        SplitLegacy::declare(sid(3), woolf(), vec![(woolf(), 10_000)], 0).unwrap_err(),
        SplitError::SelfRecipient
    );

    // Empty spec → EmptySplit
    assert!(matches!(
        SplitLegacy::declare(sid(4), woolf(), vec![], 0).unwrap_err(),
        SplitError::EmptySplit
    ));
}

#[test]
fn epoch_of_death_recorded_precisely() {
    let mut estate = woolf_estate();
    estate.certify_source_death(1941);
    match estate.state {
        SplitState::Inverted { died_at_epoch } => {
            assert_eq!(
                died_at_epoch, 1941,
                "epoch of death must match certify_source_death arg"
            );
        }
        SplitState::Pending => panic!("state must be Inverted after certify_source_death"),
    }
}

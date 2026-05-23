//! SCL end-to-end: DAO treasury delegation lifecycle.
//!
//! Doctrine (INVENTION_STACK §A5.2): "The first blockchain where
//! permissions can't outlive their purpose." Structural revocation —
//! no revocation tx, no race window.
//!
//! Fixture: a DAO grants its lead developer a 500-epoch capability
//! to call `treasury.withdraw` on the DAO's multisig vault. The
//! developer exercises the capability mid-window, then resells the
//! remaining lease term to a yield-optimizer via SDDC. The yield
//! optimizer exercises within window and is structurally blocked
//! after expiry — no revocation transaction was ever needed.

use evaporchain_scl::{
    is_authorised, list_lease_for_sale, settle_lease_resale, AuthError, Capability, Lease,
    LeaseError, MarketError,
};
use evaporchain_sddc::bid::Bid;

fn addr(b: u8) -> evaporchain_types::AccountAddress {
    [b; 32]
}

fn auction_id(b: u8) -> [u8; 32] {
    [b; 32]
}

const DEVELOPER: u8 = 0xDE;
const YIELD_OPT: u8 = 0xF0;

fn withdraw_cap() -> Capability {
    Capability::new(
        *b"treasury.withdraw.v1............",
        *b"vault.multisig.0xDEAD..........A",
    )
    .unwrap()
}

fn grant_lease() -> Lease {
    // DAO grants DEVELOPER the capability at epoch 0; expires at 500.
    Lease::issue([0xDA; 32], withdraw_cap(), addr(DEVELOPER), 0, 500).unwrap()
}

// ── Happy path ───────────────────────────────────────────────────────────────

#[test]
fn developer_exercises_within_window() {
    let lease = grant_lease();
    is_authorised(&lease, &withdraw_cap(), addr(DEVELOPER), 100).unwrap();
}

#[test]
fn capability_still_active_at_exact_expiry_epoch() {
    let lease = grant_lease();
    is_authorised(&lease, &withdraw_cap(), addr(DEVELOPER), 500).unwrap();
}

#[test]
fn developer_sells_lease_to_yield_optimizer() {
    let mut lease = grant_lease();
    lease
        .change_subject(addr(DEVELOPER), addr(YIELD_OPT))
        .unwrap();
    assert_eq!(lease.subject, addr(YIELD_OPT));
    // Original developer can no longer use the lease.
    let err = is_authorised(&lease, &withdraw_cap(), addr(DEVELOPER), 200).unwrap_err();
    assert!(matches!(err, AuthError::NotSubject { .. }));
    // New holder can.
    is_authorised(&lease, &withdraw_cap(), addr(YIELD_OPT), 200).unwrap();
}

#[test]
fn yield_optimizer_exercises_then_expires_structurally() {
    let mut lease = grant_lease();
    lease
        .change_subject(addr(DEVELOPER), addr(YIELD_OPT))
        .unwrap();
    // Active at epoch 400.
    is_authorised(&lease, &withdraw_cap(), addr(YIELD_OPT), 400).unwrap();
    // Structurally expired at 501 — no revoke tx was needed.
    let err = is_authorised(&lease, &withdraw_cap(), addr(YIELD_OPT), 501).unwrap_err();
    assert!(matches!(err, AuthError::Expired { now: 501, expires_at: 500 }));
}

#[test]
fn sddc_resale_transfers_subject_on_clear() {
    let mut lease = grant_lease();
    // DEVELOPER opens a SDDC auction for the remaining 500-epoch lease.
    // ceiling=2000, floor=200, opened at epoch 0, duration=100.
    let mut auction =
        list_lease_for_sale(&lease, addr(DEVELOPER), auction_id(1), 2000, 200, 0, 100).unwrap();
    // YIELD_OPT submits a bid: max_price=1200, lambda_tolerance=500 ≥ lot_lambda=500.
    // At epoch 50 the SDDC price is midway: 2000 - (1800 * 50/100) = 2000 - 900 = 1100.
    // 1200 ≥ 1100 → clears.
    let bid = Bid::new(addr(YIELD_OPT), 1200, 500, 50).unwrap();
    let cleared = settle_lease_resale(&mut lease, &mut auction, &[bid], 50)
        .unwrap()
        .unwrap();
    assert_eq!(cleared.winner, addr(YIELD_OPT));
    assert_eq!(lease.subject, addr(YIELD_OPT));
}

#[test]
fn sddc_resale_no_clear_keeps_original_subject() {
    let mut lease = grant_lease();
    let mut auction =
        list_lease_for_sale(&lease, addr(DEVELOPER), auction_id(2), 2000, 200, 0, 100).unwrap();
    // Bid far below market price — should not clear.
    let bid = Bid::new(addr(YIELD_OPT), 50, 500, 0).unwrap();
    let result = settle_lease_resale(&mut lease, &mut auction, &[bid], 50).unwrap();
    assert!(result.is_none());
    assert_eq!(lease.subject, addr(DEVELOPER));
}

#[test]
fn lot_lambda_equals_epochs_remaining_at_list_time() {
    let lease = grant_lease();
    // List at epoch 100 → epochs_remaining = 500 - 100 = 400.
    let auction =
        list_lease_for_sale(&lease, addr(DEVELOPER), auction_id(3), 2000, 200, 100, 50).unwrap();
    assert_eq!(auction.lot_lambda, 400);
}

// ── Adversarial ─────────────────────────────────────────────────────────────

#[test]
fn mev_searcher_cannot_steal_lease_via_change_subject() {
    // MEV searcher (neither DEVELOPER nor YIELD_OPT) tries to snatch the lease.
    let mut lease = grant_lease();
    let err = lease
        .change_subject(addr(0xEE), addr(0xEE)) // caller ≠ current subject
        .unwrap_err();
    assert_eq!(err, LeaseError::NotCurrentSubject);
    assert_eq!(lease.subject, addr(DEVELOPER));
}

#[test]
fn third_party_cannot_list_lease_for_sale() {
    let lease = grant_lease();
    let err =
        list_lease_for_sale(&lease, addr(0xEE), auction_id(4), 1000, 100, 0, 50).unwrap_err();
    assert_eq!(err, MarketError::NotCurrentSubject);
}

#[test]
fn cannot_list_expired_lease() {
    let lease = grant_lease();
    // epoch 600 is past expires_at=500.
    let err = list_lease_for_sale(&lease, addr(DEVELOPER), auction_id(5), 1000, 100, 600, 50)
        .unwrap_err();
    assert_eq!(err, MarketError::Expired);
}

#[test]
fn wrong_capability_rejected_mid_window() {
    let lease = grant_lease();
    let other = Capability::new(
        *b"treasury.read.v1................",
        *b"vault.multisig.0xDEAD..........A",
    )
    .unwrap();
    let err = is_authorised(&lease, &other, addr(DEVELOPER), 100).unwrap_err();
    assert_eq!(err, AuthError::CapabilityMismatch);
}

#[test]
fn change_subject_twice_old_holder_loses_authority() {
    let mut lease = grant_lease();
    lease
        .change_subject(addr(DEVELOPER), addr(YIELD_OPT))
        .unwrap();
    // DEVELOPER tries to re-claim → not the current subject.
    let err = lease
        .change_subject(addr(DEVELOPER), addr(DEVELOPER))
        .unwrap_err();
    assert_eq!(err, LeaseError::NotCurrentSubject);
}

#[test]
fn epoch_boundary_is_exact_no_off_by_one() {
    let lease = grant_lease();
    // Active at 499, 500; expired at 501.
    is_authorised(&lease, &withdraw_cap(), addr(DEVELOPER), 499).unwrap();
    is_authorised(&lease, &withdraw_cap(), addr(DEVELOPER), 500).unwrap();
    assert!(is_authorised(&lease, &withdraw_cap(), addr(DEVELOPER), 501).is_err());
}

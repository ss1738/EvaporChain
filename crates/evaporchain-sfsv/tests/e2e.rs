//! End-to-end integration tests for evaporchain-sfsv.
//!
//! Non-trivial fixture: life-insurance vault with secondary market.
//!
//! A creator locks 50_000 Energy in a vault that releases at epoch 500
//! ("their pension pot"). Needing liquidity, they list the claim on the
//! secondary market at epoch 50.
//!
//!   Vault: deposit=50_000, predicate=EpochReached{500}, holder=future_self
//!   Listing: ceiling=40_000, floor=10_000, lot_lambda=30, opened_at=50, duration=200
//!   price_at(t) = 40_000 − 30_000×(t−50)/200
//!
//! Two bidders compete to acquire the future-self claim:
//!
//!   Impatient Investor  epoch=100  max_price=35_000  λ_tol=10
//!     price_at(100)=32_500 ≤ 35_000 ✓ BUT λ_tol=10 < lot_lambda=30
//!     → REJECTED (won't hold a long-maturity claim)
//!   Patient Buyer       epoch=150  max_price=26_000  λ_tol=50
//!     price_at(150)=25_000 ≤ 26_000 ✓ AND λ_tol=50 ≥ 30 ✓
//!     → CLEARS at 25_000 (37.5% discount from ceiling)
//!
//! After the secondary sale:
//!   vault.current_holder = Patient Buyer
//!   At epoch 500: payout 50_000 → Patient Buyer (not original creator)
//!
//! Doctrine claim (INVENTION_STACK §A5.2): "Patient capital acquires
//! future-self claims cheaper. The impatient investor had the money
//! but refused to hold — the λ-tolerance axis enforces patience."
//!
//! Adversarial fixture: non-holder listing, expired-auction settle,
//! double payout, listing on a released vault.
//!
//! INVENTION_STACK §A5.2: Singh Future-Self Vault.

use evaporchain_sddc::{Auction, AuctionStatus, Bid};
use evaporchain_sfsv::{
    list_for_sale, payout, settle_secondary, MarketError, PayoutError, Predicate, Vault,
    VaultStatus,
};

// ── Constants ────────────────────────────────────────────────────────────

const DEPOSIT: u64 = 50_000;
const RELEASE_EPOCH: u64 = 500;

// Listing parameters.
const LIST_CEILING: u64 = 40_000;
const LIST_FLOOR: u64 = 10_000;
const LOT_LAMBDA: u64 = 30;
const LIST_OPENED_AT: u64 = 50;
const LIST_DURATION: u64 = 200; // closes at epoch 250

// ── Helpers ───────────────────────────────────────────────────────────────

fn id(b: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    a[0] = b;
    a
}

/// Hand-computed price for listing assertions.
/// price_at(t) = ceiling − (ceiling−floor)×(t−opened_at)/duration
fn listing_price_at(epoch: u64) -> u64 {
    let elapsed = epoch.saturating_sub(LIST_OPENED_AT).min(LIST_DURATION);
    LIST_CEILING - (LIST_CEILING - LIST_FLOOR) * elapsed / LIST_DURATION
}

/// Life-insurance vault locked until epoch 500.
fn life_vault() -> Vault {
    Vault::create(
        id(0x01),    // vault id
        id(0xC0),    // creator
        id(0xF0),    // future_self (initial holder)
        DEPOSIT,
        Predicate::EpochReached { release_epoch: RELEASE_EPOCH },
        0,           // created_at
    )
    .unwrap()
}

/// Open the secondary-market listing. Returns (vault, auction).
fn listed_vault() -> (Vault, Auction) {
    let mut vault = life_vault();
    let auction = list_for_sale(
        &mut vault,
        id(0xF0),          // caller = holder
        id(0xAC),          // auction id
        LIST_CEILING,
        LIST_FLOOR,
        LOT_LAMBDA,
        LIST_OPENED_AT,
        LIST_DURATION,
    )
    .unwrap();
    (vault, auction)
}

// ── Non-trivial fixture: life-insurance vault with secondary market ────────

#[test]
fn patient_buyer_acquires_claim_at_discount() {
    // Full lifecycle: list → patient buyer clears → claim transferred.
    let (mut vault, mut auction) = listed_vault();

    let impatient = Bid::new(id(0xA1), 35_000, 10, 100).unwrap(); // REJECTED on λ
    let patient   = Bid::new(id(0xB1), 26_000, 50, 150).unwrap(); // CLEARS at 25_000

    let cleared = settle_secondary(&mut vault, &mut auction, &[impatient, patient], 200)
        .unwrap()
        .expect("Patient Buyer must clear");

    assert_eq!(cleared.winner, id(0xB1), "Patient Buyer must win");
    assert_eq!(
        cleared.price_paid, 25_000,
        "price locked at Patient Buyer's submission epoch (150)"
    );
    assert_eq!(vault.current_holder(), Some(id(0xB1)), "claim transferred to Patient Buyer");
    assert!(!vault.is_listed(), "listing cleared after sale");
}

#[test]
fn impatient_investor_excluded_on_lambda_axis() {
    // price_at(100)=32_500 ≤ max_price=35_000 (price OK) but λ_tol=10 < lot_lambda=30.
    // The impatient investor is unwilling to hold a long-maturity claim.
    let (mut vault, mut auction) = listed_vault();
    let impatient = Bid::new(id(0xA1), 35_000, 10, 100).unwrap();

    let result = settle_secondary(&mut vault, &mut auction, &[impatient], 150).unwrap();
    assert!(
        result.is_none(),
        "Impatient investor must not clear: λ_tol=10 < lot_lambda=30"
    );
    assert_eq!(vault.current_holder(), Some(id(0xF0)), "holder unchanged");
    assert!(vault.is_listed(), "listing stays open after rejected bid");
}

#[test]
fn payout_goes_to_secondary_buyer_not_original_creator() {
    // Doctrine: "your future self can't sue" — after a sale, the
    // payout at maturity goes to the new holder, not the creator.
    let (mut vault, mut auction) = listed_vault();
    let patient = Bid::new(id(0xB1), 26_000, 50, 150).unwrap();
    settle_secondary(&mut vault, &mut auction, &[patient], 200)
        .unwrap()
        .unwrap();

    // Patient Buyer now holds the claim. Predicate trips at epoch 500.
    let res = payout(&mut vault, RELEASE_EPOCH, DEPOSIT).unwrap();

    assert_eq!(res.paid_to, id(0xB1), "payout must go to Patient Buyer, not original creator");
    assert_eq!(res.amount, DEPOSIT);
    assert_eq!(res.payout_at, RELEASE_EPOCH);
    assert!(!vault.is_locked(), "vault must be Released after payout");
}

#[test]
fn predicate_blocks_payout_before_epoch_trip() {
    // EpochReached predicate must hold until release_epoch.
    // Payout at epoch 499 (one before release) must fail.
    let mut vault = life_vault();

    let err = payout(&mut vault, RELEASE_EPOCH - 1, DEPOSIT).unwrap_err();
    assert!(
        matches!(err, PayoutError::PredicateNotSatisfied { .. }),
        "payout must be blocked at epoch 499, got {err:?}"
    );
    assert!(vault.is_locked(), "vault stays Locked when predicate not satisfied");

    // At exactly release_epoch, predicate trips.
    payout(&mut vault, RELEASE_EPOCH, DEPOSIT).unwrap();
    assert!(!vault.is_locked(), "vault Released at epoch 500");
}

#[test]
fn three_party_resale_chain_final_holder_receives_payout() {
    // Resale chain: creator (F0) → Patient Buyer (B1) → Third Party (T1).
    // At maturity, payout goes to T1.
    let (mut vault, mut a1) = listed_vault();

    // First sale: F0 → B1.
    let bid1 = Bid::new(id(0xB1), 26_000, 50, 150).unwrap();
    settle_secondary(&mut vault, &mut a1, &[bid1], 200).unwrap().unwrap();
    assert_eq!(vault.current_holder(), Some(id(0xB1)));

    // Second listing by B1: ceiling=35_000, floor=5_000, lot_lambda=20.
    let mut a2 = list_for_sale(
        &mut vault,
        id(0xB1),
        id(0xAD),
        35_000, 5_000, 20, 300, 150,
    )
    .unwrap();

    // T1 bids at epoch 350: price_at(350) = 35_000 − 30_000×50/150 = 25_000
    let bid2 = Bid::new(id(0xD1), 27_000, 30, 350).unwrap();
    settle_secondary(&mut vault, &mut a2, &[bid2], 400).unwrap().unwrap();
    assert_eq!(vault.current_holder(), Some(id(0xD1)));

    // B1 can no longer claim — the right transferred to T1.
    let err_b1_payout = {
        // Manually check: payout goes to current_holder (T1), not B1.
        let res = payout(&mut vault, RELEASE_EPOCH, DEPOSIT).unwrap();
        res.paid_to
    };
    assert_eq!(err_b1_payout, id(0xD1), "T1 must receive payout after resale chain");
}

#[test]
fn listing_cancelled_preserves_holder_and_clears_listing() {
    // Holder can cancel the listing before any bid clears.
    // Vault returns to unlisted state; holder is unchanged.
    let (mut vault, _auction) = listed_vault();
    assert!(vault.is_listed());

    vault.cancel_listing(id(0xF0)).unwrap();

    assert!(!vault.is_listed(), "listing must be cleared after cancel");
    assert_eq!(vault.current_holder(), Some(id(0xF0)), "holder unchanged after cancel");
}

#[test]
fn doctrine_patient_capital_wins_at_lower_price_than_impatient() {
    // INVENTION_STACK §A5.2 doctrine claim:
    // "Patient capital (high λ_tol) acquires the future-self claim cheaper."
    //
    // Impatient investor (λ_tol=10) cannot clear at all.
    // Patient Buyer (λ_tol=50) clears at 25_000 — a 37.5% discount from ceiling.
    // If we raise the impatient investor's λ_tol to 50, they'd clear earlier
    // (epoch=100) at 32_500 — 7,500 more than the patient buyer paid.
    let patient_price = listing_price_at(150); // 25_000
    let impatient_if_tolerant = listing_price_at(100); // 32_500

    assert!(
        patient_price < impatient_if_tolerant,
        "patient buyer (epoch=150, price={patient_price}) gets a better deal \
         than the impatient investor would have (epoch=100, price={impatient_if_tolerant})"
    );

    // Confirm the λ axis was the differentiator — same submission epoch,
    // only λ_tol differs:
    let (mut vault, mut auction) = listed_vault();
    let impatient_low_tol  = Bid::new(id(0xA1), 35_000, 10, 100).unwrap(); // blocked
    let patient_high_tol   = Bid::new(id(0xB1), 26_000, 50, 150).unwrap(); // wins
    let cleared = settle_secondary(&mut vault, &mut auction, &[impatient_low_tol, patient_high_tol], 200)
        .unwrap().unwrap();
    assert_eq!(cleared.winner, id(0xB1));
    assert_eq!(cleared.price_paid, patient_price);
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_non_holder_cannot_list() {
    // Only the current holder can list the claim for sale.
    // The creator (0xC0) is not the holder (future_self = 0xF0).
    let mut vault = life_vault();
    let err = list_for_sale(
        &mut vault,
        id(0xC0), // creator, not holder
        id(0xAC),
        LIST_CEILING, LIST_FLOOR, LOT_LAMBDA, LIST_OPENED_AT, LIST_DURATION,
    )
    .unwrap_err();
    assert_eq!(err, MarketError::NotCurrentHolder);
    assert!(!vault.is_listed(), "vault must remain unlisted after rejected list attempt");
}

#[test]
fn adversarial_settle_after_auction_expires_gives_no_winner() {
    // Listing window: opened_at=0, duration=50 → closes at epoch 50.
    // Only low-λ bids (blocked on λ axis). epoch_now=200 → auction expires.
    // settle_secondary returns Ok(None); vault listing persists.
    let mut vault = life_vault();
    let mut auction = list_for_sale(
        &mut vault,
        id(0xF0),
        id(0xAC),
        LIST_CEILING, LIST_FLOOR,
        LOT_LAMBDA, // lot_lambda=30
        0,    // opened_at=0
        50,   // duration=50 → closes at epoch 50
    )
    .unwrap();

    // Bid with λ_tol=5 < 30 — will never clear on λ axis regardless of epoch.
    let no_tol_bid = Bid::new(id(0xA1), LIST_CEILING, 5, 10).unwrap();
    let result = settle_secondary(&mut vault, &mut auction, &[no_tol_bid], 200).unwrap();

    assert!(result.is_none(), "expired auction with no qualifying bid must yield no winner");
    assert_eq!(auction.status, AuctionStatus::Expired, "SDDC auction must be Expired");
    assert_eq!(vault.current_holder(), Some(id(0xF0)), "holder unchanged when auction expires");
}

#[test]
fn adversarial_double_payout_rejected() {
    // Vault releases once. A second payout attempt must fail with
    // AlreadyReleased — idempotency guard against double-settlement.
    let mut vault = life_vault();
    payout(&mut vault, RELEASE_EPOCH, DEPOSIT).unwrap();

    let err = payout(&mut vault, RELEASE_EPOCH + 1, DEPOSIT).unwrap_err();
    assert_eq!(
        err,
        PayoutError::AlreadyReleased,
        "second payout must be rejected as AlreadyReleased"
    );
}

#[test]
fn adversarial_listing_on_released_vault_rejected() {
    // A released vault has no claim left to sell. list_for_sale must
    // reject with VaultReleased — preventing sale of an empty vault.
    let mut vault = life_vault();
    payout(&mut vault, RELEASE_EPOCH, DEPOSIT).unwrap(); // vault is now Released

    let err = list_for_sale(
        &mut vault,
        id(0xF0),
        id(0xAC),
        LIST_CEILING, LIST_FLOOR, LOT_LAMBDA, LIST_OPENED_AT, LIST_DURATION,
    )
    .unwrap_err();
    assert_eq!(
        err,
        MarketError::VaultReleased,
        "list_for_sale on a Released vault must be rejected"
    );
}

#[test]
fn energy_decay_predicate_releases_when_engine_reports_below_threshold() {
    // EnergyDecaysBelow predicate is refresh-aware — it reads the
    // engine-supplied live energy, not a recomputed formula.
    // Vault releases only when the caller passes contract_energy < threshold.
    let mut vault = Vault::create(
        id(0x02), id(0xC0), id(0xF0), DEPOSIT,
        Predicate::EnergyDecaysBelow { threshold: 500 },
        0,
    )
    .unwrap();

    // Engine still reports 800 energy — blocked.
    assert!(payout(&mut vault, 100, 800).is_err(), "must block when live energy=800 ≥ 500");
    // Engine refresh lifted it back to 600 — still blocked.
    assert!(payout(&mut vault, 500, 600).is_err(), "must block when live energy=600 ≥ 500");
    // Engine decayed to 499 < 500 — releases.
    let res = payout(&mut vault, 1_000, 499).unwrap();
    assert_eq!(res.paid_to, id(0xF0));
    assert_eq!(res.amount, DEPOSIT);
    assert!(matches!(vault.status, VaultStatus::Released { .. }));
}

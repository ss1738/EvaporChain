//! HBCT end-to-end: GB grid intraday capacity market.
//!
//! Doctrine (INVENTION_STACK §A3.4): "capacity in hour H decays to
//! 0 at H+1 — single-λ is dimensionally honest, not metaphor."
//!
//! Fixture: three GB battery aggregators (OCTOPUS, HABITAT, BALANCE)
//! mint day-ahead HBCT tokens across four consecutive delivery hours.
//! One secondary transfer moves 100 MWh from OCTOPUS to BALANCE to
//! hedge a net-short position. Four epoch ticks close each hour slot
//! in turn; each tick's auto-burn is verified against the expected
//! MWh total.

use evaporchain_hbct::{
    auto_burn_at_slot_close, BookError, HbctBook, HbctToken, MAX_DELIVERY_LOCATION_LEN,
};

fn addr(b: u8) -> evaporchain_types::AccountAddress {
    [b; 32]
}

// BMU codes (GB Elexon BMRS format — kept short for tests).
const LOC_A: &[u8] = b"OCTOPUS-BATT-GB";
const LOC_B: &[u8] = b"HABITAT-BATT-GB";
const LOC_C: &[u8] = b"BALANCE-BATT-GB";

const OCTOPUS: u8 = 0xAA;
const HABITAT: u8 = 0xBB;
const BALANCE: u8 = 0xCC;

// Hour slots: chain epochs at which each delivery window closes.
const H1: u64 = 100; // 00:00–00:30
const H2: u64 = 101;
const H3: u64 = 102;
const H4: u64 = 103;

fn token(holder: u8, loc: &[u8], slot: u64, mwh: u64) -> HbctToken {
    HbctToken::new(loc.to_vec(), slot, mwh, addr(holder), 0).unwrap()
}

/// Build the starting state: three aggregators, four hour slots,
/// one secondary transfer from OCTOPUS to BALANCE.
fn build_book() -> HbctBook {
    let mut book = HbctBook::new();

    // OCTOPUS: 500 MWh each at H1, H2, H3.
    book.mint(token(OCTOPUS, LOC_A, H1, 500)).unwrap();
    book.mint(token(OCTOPUS, LOC_A, H2, 500)).unwrap();
    book.mint(token(OCTOPUS, LOC_A, H3, 500)).unwrap();

    // HABITAT: 300 MWh each at H2, H3, H4.
    book.mint(token(HABITAT, LOC_B, H2, 300)).unwrap();
    book.mint(token(HABITAT, LOC_B, H3, 300)).unwrap();
    book.mint(token(HABITAT, LOC_B, H4, 300)).unwrap();

    // Secondary market: OCTOPUS sells 100 MWh at H2 to BALANCE.
    book.transfer(&LOC_A.to_vec(), H2, addr(OCTOPUS), addr(BALANCE), 100)
        .unwrap();

    book
}

#[test]
fn initial_book_state_after_transfer() {
    let book = build_book();
    // OCTOPUS retains 400 MWh at H2 (sold 100 to BALANCE).
    assert_eq!(book.balance(&LOC_A.to_vec(), H2, addr(OCTOPUS)), 400);
    assert_eq!(book.balance(&LOC_A.to_vec(), H2, addr(BALANCE)), 100);
    // OCTOPUS still holds full H1 and H3.
    assert_eq!(book.balance(&LOC_A.to_vec(), H1, addr(OCTOPUS)), 500);
    assert_eq!(book.balance(&LOC_A.to_vec(), H3, addr(OCTOPUS)), 500);
    // 7 book entries total: 3 OCTOPUS + 3 HABITAT + 1 BALANCE.
    assert_eq!(book.len(), 7);
}

#[test]
fn tick_to_h1_burns_only_h1_tokens() {
    let mut book = build_book();
    let out = auto_burn_at_slot_close(&mut book, H1);
    // Only OCTOPUS H1 entry burns.
    assert_eq!(out.entries_removed, 1);
    assert_eq!(out.mwh_burnt, 500);
    // H2/H3/H4 entries all survive.
    assert_eq!(book.balance(&LOC_A.to_vec(), H2, addr(OCTOPUS)), 400);
    assert_eq!(book.balance(&LOC_B.to_vec(), H4, addr(HABITAT)), 300);
}

#[test]
fn tick_to_h2_burns_h1_and_h2_entries() {
    let mut book = build_book();
    // One combined tick that covers H1 and H2.
    let out = auto_burn_at_slot_close(&mut book, H2);
    // H1: OCTOPUS 500. H2: OCTOPUS 400, HABITAT 300, BALANCE 100.
    assert_eq!(out.entries_removed, 4);
    assert_eq!(out.mwh_burnt, 500 + 400 + 300 + 100);
    // H3 and H4 survive.
    assert_eq!(book.balance(&LOC_A.to_vec(), H3, addr(OCTOPUS)), 500);
    assert_eq!(book.balance(&LOC_B.to_vec(), H3, addr(HABITAT)), 300);
    assert_eq!(book.balance(&LOC_B.to_vec(), H4, addr(HABITAT)), 300);
    assert_eq!(book.len(), 3);
}

#[test]
fn sequential_ticks_burn_progressively() {
    let mut book = build_book();

    // Tick H1: 1 entry, 500 MWh.
    let out1 = auto_burn_at_slot_close(&mut book, H1);
    assert_eq!(out1.entries_removed, 1);
    assert_eq!(out1.mwh_burnt, 500);
    assert_eq!(book.len(), 6);

    // Tick H2: 3 entries (OCTOPUS 400 + HABITAT 300 + BALANCE 100).
    let out2 = auto_burn_at_slot_close(&mut book, H2);
    assert_eq!(out2.entries_removed, 3);
    assert_eq!(out2.mwh_burnt, 800);
    assert_eq!(book.len(), 3);

    // Tick H3: 2 entries (OCTOPUS 500 + HABITAT 300).
    let out3 = auto_burn_at_slot_close(&mut book, H3);
    assert_eq!(out3.entries_removed, 2);
    assert_eq!(out3.mwh_burnt, 800);
    assert_eq!(book.len(), 1);

    // Tick H4: final entry (HABITAT 300).
    let out4 = auto_burn_at_slot_close(&mut book, H4);
    assert_eq!(out4.entries_removed, 1);
    assert_eq!(out4.mwh_burnt, 300);
    assert!(book.is_empty());

    // Total MWh across all ticks.
    let total = out1.mwh_burnt + out2.mwh_burnt + out3.mwh_burnt + out4.mwh_burnt;
    assert_eq!(total, 500 + 400 + 300 + 100 + 500 + 300 + 300);
}

#[test]
fn post_burn_balance_is_zero_no_explicit_error() {
    // After a slot closes the balance is structurally 0. No error is
    // raised on read — the entry is simply absent.
    let mut book = build_book();
    auto_burn_at_slot_close(&mut book, H1);
    assert_eq!(book.balance(&LOC_A.to_vec(), H1, addr(OCTOPUS)), 0);
}

#[test]
fn idle_tick_below_any_slot_no_op() {
    let mut book = build_book();
    let before = book.len();
    let out = auto_burn_at_slot_close(&mut book, 50);
    assert_eq!(out.entries_removed, 0);
    assert_eq!(book.len(), before);
}

// ── Adversarial ─────────────────────────────────────────────────────────────

#[test]
fn over_transfer_rejected() {
    let mut book = build_book();
    // OCTOPUS has 500 at H1 but tries to transfer 600.
    let err = book
        .transfer(&LOC_A.to_vec(), H1, addr(OCTOPUS), addr(BALANCE), 600)
        .unwrap_err();
    assert!(matches!(err, BookError::Insufficient { available: 500, .. }));
}

#[test]
fn transfer_from_nonexistent_entry_rejected() {
    let mut book = build_book();
    let err = book
        .transfer(&LOC_C.to_vec(), H1, addr(HABITAT), addr(OCTOPUS), 1)
        .unwrap_err();
    assert!(matches!(err, BookError::NoEntry(_)));
}

#[test]
fn location_over_cap_cannot_enter_book() {
    let big = vec![b'X'; MAX_DELIVERY_LOCATION_LEN + 1];
    let err = HbctToken::new(big, 200, 50, addr(1), 0).unwrap_err();
    assert!(matches!(err, evaporchain_hbct::TokenError::LocationTooLong { .. }));
}

#[test]
fn multiple_locations_isolated() {
    // Burning LOC_A tokens does not touch LOC_B or LOC_C balances.
    let mut book = build_book();
    auto_burn_at_slot_close(&mut book, H3);
    // LOC_B H4 (HABITAT) must survive.
    assert_eq!(book.balance(&LOC_B.to_vec(), H4, addr(HABITAT)), 300);
}

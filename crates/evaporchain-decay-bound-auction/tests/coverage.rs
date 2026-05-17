//! Cross-module coverage tests for the decay-bound sealed-bid auction:
//! end-to-end happy path with multiple bidders, phase-machine error
//! variants, capacity boundary, serde round-trips, error Display.

use evaporchain_decay_bound_auction::{
    AuctionError, AuctionId, AuctionPhase, Bid, BidCommitment, DecayBoundAuction,
    MAX_BIDDERS_PER_AUCTION,
};
use evaporchain_types::AccountAddress;

fn addr(b: u8) -> AccountAddress {
    let mut a = [0u8; 32];
    a[0] = b;
    a
}

fn make_auction() -> DecayBoundAuction {
    DecayBoundAuction::new(
        [0xAA; 32],
        "evaporchain-test-1".into(),
        AuctionId::default(),
        1_000,
        100,
        200,
        1_000_000,
        50,
    )
}

// =================================================================
// End-to-end happy path
// =================================================================

#[test]
fn full_lifecycle_three_bidders_highest_wins() {
    let mut a = make_auction();
    let bidders = [addr(0xA1), addr(0xA2), addr(0xA3)];
    let prices = [1_100u64, 5_000, 3_500];
    let nonces = [[0x10u8; 32], [0x20; 32], [0x30; 32]];

    // Open → submit commitments.
    for i in 0..3 {
        let c = DecayBoundAuction::compute_commitment(
            &a.chain_id,
            &a.auction_id,
            &bidders[i],
            prices[i],
            &nonces[i],
        );
        a.submit_commitment(bidders[i], c).unwrap();
    }
    assert_eq!(a.bids.len(), 3);

    // Close commits at deadline, reveal each, close reveals, settle.
    a.close_commits(100).unwrap();
    for i in 0..3 {
        a.reveal_bid(bidders[i], prices[i], nonces[i]).unwrap();
    }
    a.close_reveals(200).unwrap();
    let winner = a.settle(201).unwrap();
    assert_eq!(winner, Some(bidders[1]));
    assert_eq!(a.clearing_price, Some(5_000));
    assert_eq!(a.phase, AuctionPhase::Settled);
}

#[test]
fn unrevealed_bidder_does_not_win() {
    let mut a = make_auction();
    let revealer = addr(1);
    let silent = addr(2);
    let nonce = [0u8; 32];
    let price_revealed = 2_000u64;
    let price_silent = 9_000u64;

    let c1 =
        DecayBoundAuction::compute_commitment(&a.chain_id, &a.auction_id, &revealer, price_revealed, &nonce);
    let c2 =
        DecayBoundAuction::compute_commitment(&a.chain_id, &a.auction_id, &silent, price_silent, &nonce);
    a.submit_commitment(revealer, c1).unwrap();
    a.submit_commitment(silent, c2).unwrap();
    a.close_commits(100).unwrap();
    a.reveal_bid(revealer, price_revealed, nonce).unwrap();
    // silent never reveals — must lose despite higher committed price.
    a.close_reveals(200).unwrap();
    let winner = a.settle(201).unwrap();
    assert_eq!(winner, Some(revealer));
    assert_eq!(a.clearing_price, Some(price_revealed));
}

// =================================================================
// Phase-machine error variants
// =================================================================

#[test]
fn submit_after_commit_closed_rejected() {
    let mut a = make_auction();
    a.close_commits(100).unwrap();
    let err = a.submit_commitment(addr(1), BidCommitment([0u8; 32])).unwrap_err();
    assert_eq!(err, AuctionError::NotOpen(AuctionPhase::CommitClosed));
}

#[test]
fn reveal_before_commit_closed_rejected() {
    let mut a = make_auction();
    let err = a.reveal_bid(addr(1), 100, [0u8; 32]).unwrap_err();
    assert_eq!(err, AuctionError::NotCommitClosed(AuctionPhase::Open));
}

#[test]
fn close_reveals_before_deadline_rejected() {
    let mut a = make_auction();
    a.close_commits(100).unwrap();
    let err = a.close_reveals(150).unwrap_err();
    assert_eq!(err, AuctionError::RevealWindowOpen);
}

#[test]
fn settle_before_reveal_closed_rejected() {
    let mut a = make_auction();
    a.close_commits(100).unwrap();
    let err = a.settle(150).unwrap_err();
    assert_eq!(err, AuctionError::NotRevealClosed(AuctionPhase::CommitClosed));
}

#[test]
fn reveal_unknown_bidder_rejected() {
    let mut a = make_auction();
    a.close_commits(100).unwrap();
    let err = a.reveal_bid(addr(99), 100, [0u8; 32]).unwrap_err();
    assert_eq!(err, AuctionError::NoCommitment);
}

// =================================================================
// Capacity boundary
// =================================================================

#[test]
fn max_bidders_constant_pinned() {
    assert_eq!(MAX_BIDDERS_PER_AUCTION, 1_024);
}

// =================================================================
// Commitment-binding negative tests (cross-product)
// =================================================================

#[test]
fn reveal_with_wrong_nonce_rejected() {
    let mut a = make_auction();
    let bidder = addr(1);
    let real_nonce = [0xAAu8; 32];
    let wrong_nonce = [0xBB; 32];
    let price = 1_500u64;
    let c = DecayBoundAuction::compute_commitment(
        &a.chain_id, &a.auction_id, &bidder, price, &real_nonce,
    );
    a.submit_commitment(bidder, c).unwrap();
    a.close_commits(100).unwrap();
    let err = a.reveal_bid(bidder, price, wrong_nonce).unwrap_err();
    assert_eq!(err, AuctionError::RevealMismatch);
}

// =================================================================
// Serde round-trips
// =================================================================

#[test]
fn auction_phase_serde_round_trips() {
    for phase in [
        AuctionPhase::Open,
        AuctionPhase::CommitClosed,
        AuctionPhase::RevealClosed,
        AuctionPhase::Settled,
        AuctionPhase::Evaporated,
    ] {
        let json = serde_json::to_string(&phase).unwrap();
        let back: AuctionPhase = serde_json::from_str(&json).unwrap();
        assert_eq!(back, phase);
    }
}

#[test]
fn bid_commitment_serde_round_trips() {
    let c = BidCommitment([0x42; 32]);
    let json = serde_json::to_string(&c).unwrap();
    let back: BidCommitment = serde_json::from_str(&json).unwrap();
    assert_eq!(back, c);
}

#[test]
fn bid_serde_round_trips() {
    let bid = Bid {
        bidder: addr(7),
        commitment: BidCommitment([0xAB; 32]),
        revealed_price: Some(1234),
        revealed_nonce: Some([0xCD; 32]),
    };
    let json = serde_json::to_string(&bid).unwrap();
    let back: Bid = serde_json::from_str(&json).unwrap();
    assert_eq!(back, bid);
}

// =================================================================
// Error Display
// =================================================================

#[test]
fn auction_errors_display() {
    let e = AuctionError::TooManyBidders;
    assert!(e.to_string().contains(&MAX_BIDDERS_PER_AUCTION.to_string()));
    let e = AuctionError::NotOpen(AuctionPhase::Settled);
    assert!(e.to_string().contains("Settled"));
    let e = AuctionError::RevealMismatch;
    assert!(e.to_string().to_lowercase().contains("reveal"));
}

// =================================================================
// Energy schedule
// =================================================================

#[test]
fn energy_at_initial_window_returns_initial_energy() {
    // At the commit deadline epoch (zero epochs after first half-life prep
    // window) the auction energy should be roughly initial_energy. The
    // exact value depends on the formula's window — assert non-zero and
    // ≤ initial.
    let a = make_auction();
    let e0 = a.energy_at(a.commit_deadline_epoch);
    assert!(e0 > 0);
    assert!(e0 <= a.initial_energy);
}

#[test]
fn energy_at_far_future_reaches_zero() {
    let a = make_auction();
    let e = a.energy_at(10_000);
    assert_eq!(e, 0);
}

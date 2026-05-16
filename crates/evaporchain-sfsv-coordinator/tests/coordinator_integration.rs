//! Integration tests for the SFSV coordinator off-chain pipeline.
//!
//! Non-trivial fixture: two vault listings (A and B) with different
//! price/lambda parameters. Bids arrive for both; A clears, B does not.
//! After the clear tick A is removed from the auctioneer registry and B
//! remains open. A second adversarial bid tests zero-price rejection.
//!
//! No live node required — the test exercises the auctioneer state machine
//! and market clearing purely in-process.
//!
//! INVENTION_STACK §A5.2: SFSV off-chain coordinator.

use evaporchain_sddc::auction::{Auction, AuctionId};
use evaporchain_sfsv::{predicate::Predicate, vault::Vault};
use evaporchain_sfsv_coordinator::auctioneer::{Auctioneer, BidRequest};

// ── Helpers ───────────────────────────────────────────────────────────────

fn addr(b: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    a[0] = b;
    a
}

fn auction_id(b: u8) -> AuctionId {
    addr(b)
}

fn build_vault(future_self: u8, deposit: u64, release_epoch: u64) -> Vault {
    Vault::create(
        addr(future_self),
        addr(0xAA),
        addr(future_self),
        deposit,
        Predicate::EpochReached { release_epoch },
        0,
    )
    .unwrap()
}

/// Build a vault already in the listed state (mirrors coordinator poller).
fn build_listed_vault(
    future_self: u8,
    ceiling: u64,
    floor: u64,
    opened_at: u64,
    duration: u64,
) -> Vault {
    let mut v = build_vault(future_self, 1_000, 9_999);
    v.list_for_sale(addr(future_self), ceiling, floor, opened_at, duration)
        .unwrap();
    v
}

fn build_auction(ceiling: u64, floor: u64, opened_at: u64, duration: u64, b: u8) -> Auction {
    Auction::open(auction_id(b), ceiling, floor, 0, opened_at, duration).unwrap()
}

fn hex_addr(b: u8) -> String {
    hex::encode(addr(b))
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn two_listings_one_clears_one_stays_open() {
    let mut ae = Auctioneer::new();

    // Vault A: ceiling=1000, floor=100, opened_at=0, duration=200
    let vault_a = build_listed_vault(0xAA, 1000, 100, 0, 200);
    let auction_a = build_auction(1000, 100, 0, 200, 0xA0);
    ae.register_listing(1, vault_a, auction_a).unwrap();

    // Vault B: ceiling=800, floor=200, opened_at=0, duration=200
    let vault_b = build_listed_vault(0xBB, 800, 200, 0, 200);
    let auction_b = build_auction(800, 200, 0, 200, 0xB0);
    ae.register_listing(2, vault_b, auction_b).unwrap();

    assert!(ae.is_tracked(1));
    assert!(ae.is_tracked(2));

    // SDDC clears at price_at(bid.submitted_at), not price_at(epoch_now).
    // Vault A: ceiling=1000, floor=100, opened_at=0, duration=200.
    // price_at(50) = 1000 - 900*(50/200) = 775. max_price=700 < 775 → no clear.
    ae.submit_bid(&BidRequest {
        contract_id: 1,
        bidder_hex: hex_addr(0xCC),
        max_price: 700,
        lambda_tolerance: 5,
        submitted_at: 50,
    })
    .unwrap();

    // No clear at epoch 50 (price_at(50)=775 > max_price=700).
    let clears = ae.try_clear_all(50);
    assert!(clears.is_empty(), "bid at epoch 50 must not clear (price 775 > max 700)");
    assert!(ae.is_tracked(1), "vault A still open");

    // Submit a second bid at epoch 100: price_at(100)=550. max_price=700 ≥ 550 → clears.
    ae.submit_bid(&BidRequest {
        contract_id: 1,
        bidder_hex: hex_addr(0xCC),
        max_price: 700,
        lambda_tolerance: 5,
        submitted_at: 100,
    })
    .unwrap();

    let clears = ae.try_clear_all(100);
    assert_eq!(clears.len(), 1, "vault A must clear at epoch 100");
    let c = &clears[0];
    assert_eq!(c.contract_id, 1);
    assert_eq!(c.winner, addr(0xCC));
    assert_eq!(c.price_paid, 550); // price_at(submitted_at=100)

    // A is gone; B is still open (no bids).
    assert!(!ae.is_tracked(1), "vault A removed after clear");
    assert!(ae.is_tracked(2), "vault B still open");
}

#[test]
fn bid_for_unknown_contract_rejected() {
    let mut ae = Auctioneer::new();
    let err = ae
        .submit_bid(&BidRequest {
            contract_id: 999,
            bidder_hex: hex_addr(0xDD),
            max_price: 100,
            lambda_tolerance: 0,
            submitted_at: 0,
        })
        .unwrap_err();
    assert!(
        matches!(err, evaporchain_sfsv_coordinator::auctioneer::AuctioneerError::NotListed(999)),
        "got {err:?}"
    );
}

#[test]
fn zero_price_bid_rejected() {
    let mut ae = Auctioneer::new();
    let vault = build_listed_vault(0xAA, 1000, 100, 0, 200);
    let auction = build_auction(1000, 100, 0, 200, 0xA0);
    ae.register_listing(1, vault, auction).unwrap();

    let err = ae
        .submit_bid(&BidRequest {
            contract_id: 1,
            bidder_hex: hex_addr(0xEE),
            max_price: 0, // adversarial: zero price
            lambda_tolerance: 100,
            submitted_at: 0,
        })
        .unwrap_err();
    assert!(
        matches!(
            err,
            evaporchain_sfsv_coordinator::auctioneer::AuctioneerError::BadBid(_)
        ),
        "zero-price bid must be rejected as BadBid, got {err:?}"
    );
}

#[test]
fn expired_listing_gives_no_clear() {
    let mut ae = Auctioneer::new();
    // Listing: opened_at=0, duration=10 → expires at epoch 10.
    let vault = build_listed_vault(0xAA, 1000, 100, 0, 10);
    let auction = build_auction(1000, 100, 0, 10, 0xA0);
    ae.register_listing(1, vault, auction).unwrap();

    // Bid at epoch 5 (within window).
    ae.submit_bid(&BidRequest {
        contract_id: 1,
        bidder_hex: hex_addr(0xCC),
        max_price: 5000, // well above any price in the range
        lambda_tolerance: 100,
        submitted_at: 5,
    })
    .unwrap();

    // Clear at epoch 11 — past the duration. SDDC should expire the auction.
    // After expiry the listing is dropped from the auctioneer.
    let clears = ae.try_clear_all(11);
    assert!(clears.is_empty(), "expired listing must not produce a clear");
    // Auctioneer drops the expired listing.
    assert!(!ae.is_tracked(1), "expired listing must be dropped");
}

#[test]
fn sequential_resale_chain_clears_independently() {
    // Two independent sales: vault A (first owner) → bidder C,
    // then the coordinator re-registers vault A for a second listing
    // (new owner C listed it again).
    let mut ae = Auctioneer::new();

    let vault_a1 = build_listed_vault(0xAA, 1000, 100, 0, 200);
    let auction_a1 = build_auction(1000, 100, 0, 200, 0xA1);
    ae.register_listing(1, vault_a1, auction_a1).unwrap();

    ae.submit_bid(&BidRequest {
        contract_id: 1,
        bidder_hex: hex_addr(0xCC),
        max_price: 1000,
        lambda_tolerance: 50,
        submitted_at: 10,
    })
    .unwrap();

    let clears = ae.try_clear_all(100);
    assert_eq!(clears.len(), 1);
    assert_eq!(clears[0].winner, addr(0xCC));
    assert!(!ae.is_tracked(1));

    // Re-list: new owner 0xCC re-registers the same contract id.
    let vault_a2 = build_listed_vault(0xCC, 500, 50, 200, 100);
    let auction_a2 = build_auction(500, 50, 200, 100, 0xA2);
    ae.register_listing(1, vault_a2, auction_a2).unwrap();
    assert!(ae.is_tracked(1));

    ae.submit_bid(&BidRequest {
        contract_id: 1,
        bidder_hex: hex_addr(0xDD),
        max_price: 500,
        lambda_tolerance: 50,
        submitted_at: 220,
    })
    .unwrap();

    let clears2 = ae.try_clear_all(250);
    assert_eq!(clears2.len(), 1);
    assert_eq!(clears2[0].winner, addr(0xDD));
}

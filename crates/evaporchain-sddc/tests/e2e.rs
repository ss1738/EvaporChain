//! End-to-end integration tests for evaporchain-sddc.
//!
//! Non-trivial fixture: tokenised carbon-credit secondary market.
//!
//! A "decaying" carbon credit (lot_lambda=40) is auctioned with a
//! continuous Dutch descent over a 200-epoch window:
//!   ceiling=10_000, floor=1_000, lot_lambda=40, opened_at=0, duration=200
//!
//! Linear price descent: price_at(t) = 10_000 − 9_000×t/200
//!
//! Four bidders with different (epoch, max_price, lambda_tolerance) tuples:
//!
//!   Alice  epoch=20  max_price=9_000  λ_tol=30
//!     price_at(20)=9_100 > 9_000 → REJECTED (price axis: can't afford this early)
//!   Bob    epoch=50  max_price=9_000  λ_tol=15
//!     price_at(50)=7_750 ≤ 9_000 ✓ BUT λ_tol=15 < lot_lambda=40 → REJECTED (λ axis)
//!   Carol  epoch=80  max_price=7_000  λ_tol=50
//!     price_at(80)=6_400 ≤ 7_000 ✓ AND λ_tol=50 ≥ 40 ✓ → CLEARS at 6_400
//!   Dave   epoch=120 max_price=5_000  λ_tol=50
//!     would clear at 4_600, but Carol (epoch=80) satisfies first.
//!
//! Doctrine claim (INVENTION_STACK §A5.2): "High-λ-tolerant bidders
//! win at lower prices. Bob had the money but not the λ-tolerance;
//! Carol waited, accepted the decaying asset, and won cheaper."
//!
//! Adversarial fixture: bids before auction opens, clears on expired
//! auctions, double-clear attempts, zero-max-price bids.
//!
//! INVENTION_STACK §A5.2: Singh Decay-Dutch Continuous Auction.

use evaporchain_sddc::{
    try_clear, Auction, AuctionStatus, Bid, BidError, ClearError, LifecycleError,
};

// ── Constants ────────────────────────────────────────────────────────────

const CEILING: u64 = 10_000;
const FLOOR: u64 = 1_000;
const LOT_LAMBDA: u64 = 40;
const OPENED_AT: u64 = 0;
const DURATION: u64 = 200;

// ── Helpers ───────────────────────────────────────────────────────────────

fn id(b: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    a[0] = b;
    a
}

/// Hand-computed linear price descent for assertions.
/// price_at(t) = ceiling − (ceiling−floor)×(t−opened_at)/duration
fn expected_price(epoch: u64) -> u64 {
    let elapsed = epoch.saturating_sub(OPENED_AT).min(DURATION);
    CEILING - (CEILING - FLOOR) * elapsed / DURATION
}

fn carbon_auction() -> Auction {
    Auction::open(id(0xA0), CEILING, FLOOR, LOT_LAMBDA, OPENED_AT, DURATION).unwrap()
}

// ── Non-trivial fixture: four-bidder carbon-credit session ────────────────

#[test]
fn carol_wins_at_epoch_80_joint_clear() {
    // Full four-bidder session. Alice rejected (price), Bob rejected (λ),
    // Carol clears at 6_400, Dave's later bid is moot.
    let mut auction = carbon_auction();

    let alice = Bid::new(id(0xD1), 9_000, 30, 20).unwrap(); // REJECTED — price
    let bob = Bid::new(id(0xD2), 9_000, 15, 50).unwrap(); // REJECTED — λ axis
    let carol = Bid::new(id(0xD3), 7_000, 50, 80).unwrap(); // CLEARS
    let dave = Bid::new(id(0xD4), 5_000, 50, 120).unwrap(); // would clear, but Carol first

    let cleared = try_clear(&mut auction, &[alice, bob, carol, dave], 130)
        .unwrap()
        .expect("Carol must clear");

    assert_eq!(cleared.winner, id(0xD3), "Carol must win");
    assert_eq!(
        cleared.price_paid, 6_400,
        "price locked at Carol's submission epoch (80)"
    );
    assert_eq!(cleared.cleared_at, 80);
}

#[test]
fn alice_excluded_on_price_axis() {
    // Alice submits at epoch=20 with max_price=9_000. price_at(20)=9_100.
    // 9_000 < 9_100 → price axis fails; auction remains open.
    let mut auction = carbon_auction();
    let alice = Bid::new(id(0xD1), 9_000, 30, 20).unwrap();

    let result = try_clear(&mut auction, &[alice], 50).unwrap();
    assert!(
        result.is_none(),
        "Alice must not clear: price 9_100 > max_price 9_000"
    );
    assert!(
        auction.is_open(),
        "auction must stay open after Alice's rejected bid"
    );
}

#[test]
fn bob_excluded_on_lambda_axis() {
    // Bob at epoch=50: price_at(50)=7_750 ≤ max_price=9_000 (price OK),
    // but λ_tol=15 < lot_lambda=40 → λ axis fails.
    let mut auction = carbon_auction();
    let bob = Bid::new(id(0xD2), 9_000, 15, 50).unwrap();

    let result = try_clear(&mut auction, &[bob], 80).unwrap();
    assert!(
        result.is_none(),
        "Bob must not clear: λ_tol=15 < lot_lambda=40, even though price is favorable"
    );
    assert!(auction.is_open());
}

#[test]
fn carol_wins_not_dave_earliest_satisfier_rule() {
    // Both Carol (epoch=80) and Dave (epoch=120) satisfy both axes
    // at their respective submission epochs. Carol submitted earlier
    // so she wins — clearing is deterministic by submission epoch.
    let mut auction = carbon_auction();
    let carol = Bid::new(id(0xD3), 7_000, 50, 80).unwrap();
    let dave = Bid::new(id(0xD4), 5_000, 50, 120).unwrap();

    let cleared = try_clear(&mut auction, &[dave.clone(), carol.clone()], 130)
        .unwrap()
        .expect("must clear");

    assert_eq!(
        cleared.winner,
        id(0xD3),
        "Carol (epoch=80) wins over Dave (epoch=120)"
    );
    assert_eq!(cleared.price_paid, 6_400); // price_at(80), not price_at(120)
}

#[test]
fn cleared_price_equals_price_at_submission_epoch() {
    // Explicit proof that price_paid = price_at(submitted_at), not price_at(epoch_now).
    // Carol submits at 80 → price_paid=6_400.
    // epoch_now=150 would give price_at(150)=3_250 if used — that would be wrong.
    let mut auction = carbon_auction();
    let carol = Bid::new(id(0xD3), 7_000, 50, 80).unwrap();

    let cleared = try_clear(&mut auction, &[carol], 150).unwrap().unwrap();

    assert_eq!(
        cleared.price_paid,
        expected_price(80), // 6_400
        "price must be price_at(submitted_at=80)=6_400, not price_at(epoch_now=150)=3_250"
    );
}

#[test]
fn doctrine_high_lambda_tolerance_wins_at_lower_price() {
    // INVENTION_STACK §A5.2 doctrine claim:
    // "High-λ-tolerant bidders win at lower prices."
    //
    // Bob has λ_tol=15 — he CAN'T clear at epoch=50 even though he has
    // the money (price 7_750 ≤ 9_000). He refuses to hold a decaying asset.
    //
    // Carol has λ_tol=50 — she tolerates decay. She submits the SAME
    // price axis willingness (max_price=9_000 too) at epoch=50 and would
    // clear at 7_750. But we show Carol waited to get an even lower price:
    // by tolerating λ, she can choose her clearing epoch (80 → 6_400).
    //
    // Quantified: Carol's price (6_400) < what Bob would have paid (7_750)
    // if he had matched Carol's λ tolerance.
    let carol_price = expected_price(80); // 6_400 — Carol's actual clearing price
    let bob_price = expected_price(50); // 7_750 — price where Bob had money but not λ

    assert!(
        carol_price < bob_price,
        "high-λ-tolerant bidder (Carol) wins cheaper ({carol_price}) \
         than the impatient bidder would have faced ({bob_price})"
    );

    // Confirm the λ axis was indeed the differentiator for Bob:
    let mut auction = carbon_auction();
    let bob_with_high_tol = Bid::new(id(0xD2), 9_000, 50, 50).unwrap(); // same as Bob but λ_tol=50
    let cleared = try_clear(&mut auction, &[bob_with_high_tol], 60)
        .unwrap()
        .expect("Bob with high λ_tol must clear");
    assert_eq!(cleared.price_paid, bob_price, "Bob-with-λ clears at 7_750");
    assert!(
        carol_price < cleared.price_paid,
        "Carol still gets a better price by waiting: {carol_price} < {}",
        cleared.price_paid
    );
}

#[test]
fn clearing_deterministic_regardless_of_bid_slice_order() {
    // Validators receive bids via gossip in arbitrary order; clearing
    // must produce the same winner regardless of slice ordering.
    // Carol wins in both permutations.
    let alice = Bid::new(id(0xD1), 9_000, 30, 20).unwrap();
    let bob = Bid::new(id(0xD2), 9_000, 15, 50).unwrap();
    let carol = Bid::new(id(0xD3), 7_000, 50, 80).unwrap();
    let dave = Bid::new(id(0xD4), 5_000, 50, 120).unwrap();

    let mut a1 = carbon_auction();
    let mut a2 = carbon_auction();

    let c1 = try_clear(
        &mut a1,
        &[alice.clone(), bob.clone(), carol.clone(), dave.clone()],
        130,
    )
    .unwrap()
    .unwrap();
    let c2 = try_clear(&mut a2, &[dave, carol, bob, alice], 130)
        .unwrap()
        .unwrap();

    assert_eq!(
        c1.winner, c2.winner,
        "winner must be identical across orderings"
    );
    assert_eq!(
        c1.price_paid, c2.price_paid,
        "price must be identical across orderings"
    );
    assert_eq!(
        c1.cleared_at, c2.cleared_at,
        "clearing epoch must be identical"
    );
}

#[test]
fn auction_expires_with_no_satisfying_bid() {
    // All bids fail both axes (only low-λ impatient bidders). Once
    // epoch_now passes the close window, the auction marks itself Expired.
    let mut auction = carbon_auction();
    // Only Bob-like bids (λ_tol=5 < lot_lambda=40) — will never clear on λ axis.
    let b1 = Bid::new(id(0xD1), 9_000, 5, 10).unwrap();
    let b2 = Bid::new(id(0xD2), 9_000, 5, 100).unwrap();

    let result = try_clear(&mut auction, &[b1, b2], 9_999).unwrap();
    assert!(result.is_none(), "no bid satisfies both axes; must expire");
    assert_eq!(
        auction.status,
        AuctionStatus::Expired,
        "auction must be Expired after window passes with no clear"
    );
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_bid_before_auction_opens_rejected() {
    // Auction opens at epoch=100. Bid submitted at epoch=50 must be
    // rejected — not silently skipped — to prevent front-running.
    let mut auction = Auction::open(id(0xA1), 10_000, 1_000, 40, 100, 200).unwrap();
    let early_bid = Bid::new(id(0xD1), 10_000, 50, 50).unwrap(); // submitted_at=50 < opened_at=100

    let err = try_clear(&mut auction, &[early_bid], 150).unwrap_err();
    assert!(
        matches!(
            err,
            ClearError::Lifecycle(LifecycleError::BidBeforeOpen {
                submitted_at: 50,
                opened_at: 100,
                ..
            })
        ),
        "bid submitted before auction opens must be rejected with BidBeforeOpen, got {err:?}"
    );
}

#[test]
fn adversarial_try_clear_on_expired_auction_rejected() {
    // Once the auction is Expired, further try_clear must fail with NotOpen.
    let mut auction = Auction::open(id(0xA1), 10_000, 1_000, 40, 0, 10).unwrap();
    // Expire it.
    try_clear(&mut auction, &[], 9_999).unwrap();
    assert_eq!(auction.status, AuctionStatus::Expired);

    let b = Bid::new(id(0xD1), 10_000, 50, 5).unwrap();
    let err = try_clear(&mut auction, &[b], 20).unwrap_err();
    assert!(
        matches!(err, ClearError::Lifecycle(LifecycleError::NotOpen)),
        "try_clear on Expired auction must return NotOpen, got {err:?}"
    );
}

#[test]
fn adversarial_double_clear_rejected() {
    // Once an auction clears, a second try_clear must fail with NotOpen —
    // re-entrancy / double-settlement is impossible.
    let mut auction = carbon_auction();
    let carol = Bid::new(id(0xD3), 7_000, 50, 80).unwrap();
    try_clear(&mut auction, std::slice::from_ref(&carol), 100)
        .unwrap()
        .unwrap(); // first clear OK

    let err = try_clear(&mut auction, std::slice::from_ref(&carol), 110).unwrap_err();
    assert!(
        matches!(err, ClearError::Lifecycle(LifecycleError::NotOpen)),
        "second try_clear on Cleared auction must return NotOpen, got {err:?}"
    );
}

#[test]
fn adversarial_zero_max_price_bid_rejected() {
    // A zero-price bid is a protocol violation — Bid::new must reject it
    // before it can reach try_clear (prevents griefing with dust bids).
    let err = Bid::new(id(0xD1), 0, 50, 10).unwrap_err();
    assert_eq!(
        err,
        BidError::ZeroMaxPrice,
        "zero max_price must be rejected at construction time"
    );
}

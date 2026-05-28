//! End-to-end integration tests for `evaporchain-decay-bound-auction`.
//!
//! # Doctrine claim (INVENTION_STACK.md §A5.2 / Singh Decay-Dutch Continuous
//! Auction substrate)
//!
//! > "An auction whose end-condition is energy-decay rather than block-height.
//! >  A commitment cannot be replayed across auctions or chains: the DST binds
//! >  chain_id + auction_id + bidder + price + nonce into the BLAKE3 output.
//! >  An auction that is not refreshed (energy → 0) before the reveal window
//! >  closes evaporates instead of settling — no winner, all committed deposits
//! >  become refundable.  This matches the doctrine pattern of 'things that
//! >  aren't refreshed cease to exist'."
//!
//! # Non-trivial fixture — 4-bidder compound scenario
//!
//! Participants:
//!   Alice   [0x01..] — commits + reveals, highest valid price (5_000) → wins
//!   Bob     [0x02..] — commits but never reveals (front-run / no-show)
//!   Carol   [0x03..] — commits + reveals, below reserve_price (800)  → filtered
//!   Dave    [0x04..] — commits + reveals, valid but lower (2_500)     → loses to Alice
//!
//! reserve_price = 1_000; commit_deadline = 100; reveal_deadline = 200.
//!
//! Expected outcome:
//!   • Alice wins; clearing_price = 5_000.
//!   • Bob's unrevealed price (9_000) is invisible to settle — no-show
//!     cannot affect the winner.
//!   • Carol's revealed price (800 < 1_000) is skipped.
//!   • Dave loses price comparison.
//!
//! # Adversarial tests
//!
//! 1. Cross-auction commitment replay: commitment computed for auction_id_1
//!    is submitted to auction_id_2; the reveal fails with RevealMismatch
//!    because the DST encodes the auction_id.
//!
//! 2. Cross-chain commitment replay: commitment computed on "chain-A"
//!    submitted to an identical auction on "chain-B"; reveal fails with
//!    RevealMismatch because the DST encodes chain_id.
//!
//! 3. Evaporation with full reveals: energy_at(far-future-epoch) == 0 →
//!    settle() returns Err(EvaporatedDuringReveal) even when bidders revealed
//!    valid prices; phase → Evaporated; no winner recorded.
//!
//! 4. Phase sequence completeness: commit → close_commits → reveal → but
//!    skip close_reveals → settle() returns NotRevealClosed.

use evaporchain_decay_bound_auction::{
    AuctionError, AuctionId, AuctionPhase, BidCommitment, DecayBoundAuction,
    MAX_BIDDERS_PER_AUCTION,
};
use evaporchain_types::AccountAddress;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn addr(b: u8) -> AccountAddress {
    let mut a = [0u8; 32];
    a[0] = b;
    a
}

fn nonce(b: u8) -> [u8; 32] {
    [b; 32]
}

fn default_auction() -> DecayBoundAuction {
    DecayBoundAuction::new(
        [0xAA; 32],
        "evaporchain-e2e-1".into(),
        AuctionId::default(),
        1_000,     // reserve_price
        100,       // commit_deadline_epoch
        200,       // reveal_deadline_epoch
        1_000_000, // initial_energy
        50,        // half_life_epochs
    )
}

// ── Non-trivial fixture ───────────────────────────────────────────────────────

/// INVENTION_STACK §A5.2 — non-trivial 4-bidder compound scenario.
///
/// Covers all four outcome paths in a single fixture:
///   (1) winner: Alice reveals highest valid price;
///   (2) front-run / no-show: Bob commits but never reveals;
///   (3) below-reserve: Carol reveals below reserve_price;
///   (4) losing: Dave reveals a valid but lower price.
#[test]
fn fixture_four_bidder_compound_scenario() {
    let mut a = default_auction();
    let alice = addr(0x01);
    let bob = addr(0x02);
    let carol = addr(0x03);
    let dave = addr(0x04);

    let alice_price = 5_000u64;
    let bob_price = 9_000u64; // highest but Bob never reveals
    let carol_price = 800u64; // below reserve_price=1_000
    let dave_price = 2_500u64;

    let alice_nonce = nonce(0xA1);
    let bob_nonce = nonce(0xB2);
    let carol_nonce = nonce(0xC3);
    let dave_nonce = nonce(0xD4);

    // ── Commit phase ─────────────────────────────────────────────────────────
    for (bidder, price, n) in [
        (alice, alice_price, alice_nonce),
        (bob, bob_price, bob_nonce),
        (carol, carol_price, carol_nonce),
        (dave, dave_price, dave_nonce),
    ] {
        let c =
            DecayBoundAuction::compute_commitment(&a.chain_id, &a.auction_id, &bidder, price, &n);
        a.submit_commitment(bidder, c).unwrap();
    }
    assert_eq!(a.bids.len(), 4);
    assert_eq!(a.phase, AuctionPhase::Open);

    // ── Close commits ─────────────────────────────────────────────────────────
    a.close_commits(100).unwrap();
    assert_eq!(a.phase, AuctionPhase::CommitClosed);

    // ── Reveal phase — Bob deliberately does not reveal ──────────────────────
    a.reveal_bid(alice, alice_price, alice_nonce).unwrap();
    // Bob: no-show
    a.reveal_bid(carol, carol_price, carol_nonce).unwrap();
    a.reveal_bid(dave, dave_price, dave_nonce).unwrap();

    // ── Close reveals + settle ────────────────────────────────────────────────
    a.close_reveals(200).unwrap();
    let winner = a.settle(201).unwrap();

    // Alice wins with 5_000; Bob's 9_000 is invisible (no reveal).
    assert_eq!(
        winner,
        Some(alice),
        "Alice must win despite Bob's higher committed-but-unrevealed price"
    );
    assert_eq!(a.clearing_price, Some(alice_price));
    assert_eq!(a.phase, AuctionPhase::Settled);

    // Carol's below-reserve reveal has no effect on the winner.
    assert_ne!(a.winner, Some(carol));
    // Dave's valid bid loses.
    assert_ne!(a.winner, Some(dave));
    // Bob's unrevealed bid leaves no trace in winner field.
    assert_ne!(a.winner, Some(bob));
}

/// Submission order must not affect the settled winner.
#[test]
fn fixture_submission_order_independent() {
    let run = |order: &[(AccountAddress, u64, [u8; 32])]| {
        let mut a = default_auction();
        for &(bidder, price, n) in order {
            let c = DecayBoundAuction::compute_commitment(
                &a.chain_id,
                &a.auction_id,
                &bidder,
                price,
                &n,
            );
            a.submit_commitment(bidder, c).unwrap();
        }
        a.close_commits(100).unwrap();
        for &(bidder, price, n) in order {
            a.reveal_bid(bidder, price, n).unwrap();
        }
        a.close_reveals(200).unwrap();
        a.settle(201).unwrap();
        (a.winner, a.clearing_price)
    };

    let alice = addr(1);
    let bob = addr(2);
    let carol = addr(3);

    let bids = [
        (alice, 5_000u64, nonce(0xA1)),
        (bob, 3_000, nonce(0xB2)),
        (carol, 4_500, nonce(0xC3)),
    ];
    let bids_rev = [
        (carol, 4_500u64, nonce(0xC3)),
        (bob, 3_000, nonce(0xB2)),
        (alice, 5_000, nonce(0xA1)),
    ];

    let fwd = run(&bids);
    let rev = run(&bids_rev);
    assert_eq!(fwd, rev, "settlement must be submission-order-independent");
    assert_eq!(fwd.0, Some(alice), "Alice must win both orderings");
}

// ── Adversarial: cross-auction commitment replay ──────────────────────────────

/// Adversarial — DST binds auction_id:
/// A commitment computed for `auction_id_1` cannot be replayed as a valid
/// commitment in `auction_id_2` even if all other parameters are identical.
///
/// INVENTION_STACK §A5.2: "A commitment cannot be replayed across auctions
/// or chains: the DST binds chain_id + auction_id."
#[test]
fn adversarial_cross_auction_replay_rejected() {
    let chain_id = "evaporchain-test";
    let auction_id_1: [u8; 32] = [0x01; 32];
    let auction_id_2: [u8; 32] = [0x02; 32];
    let bidder = addr(1);
    let price = 2_000u64;
    let n = nonce(0x42);

    // Attacker computes commitment for auction 1.
    let c_for_auction_1 =
        DecayBoundAuction::compute_commitment(chain_id, &auction_id_1, &bidder, price, &n);

    // Auction 2: same chain_id but different auction_id.
    let mut a2 = DecayBoundAuction::new(
        auction_id_2,
        chain_id.to_string(),
        AuctionId::default(),
        1_000,
        100,
        200,
        1_000_000,
        50,
    );
    // Submit the replayed commitment from auction 1 into auction 2.
    a2.submit_commitment(bidder, c_for_auction_1).unwrap();
    a2.close_commits(100).unwrap();

    // Reveal with the same (price, nonce) pair — must fail because the
    // commitment hash encodes auction_id_2 but was built with auction_id_1.
    let err = a2.reveal_bid(bidder, price, n).unwrap_err();
    assert_eq!(
        err,
        AuctionError::RevealMismatch,
        "cross-auction replay must fail at reveal time"
    );
}

/// Adversarial — DST binds chain_id:
/// A commitment computed on "chain-A" cannot be replayed on "chain-B".
#[test]
fn adversarial_cross_chain_replay_rejected() {
    let auction_id: [u8; 32] = [0xBB; 32];
    let bidder = addr(2);
    let price = 3_000u64;
    let n = nonce(0x55);

    let c_for_chain_a =
        DecayBoundAuction::compute_commitment("chain-A", &auction_id, &bidder, price, &n);

    // Identical auction deployed on "chain-B".
    let mut a_b = DecayBoundAuction::new(
        auction_id,
        "chain-B".to_string(),
        AuctionId::default(),
        1_000,
        100,
        200,
        1_000_000,
        50,
    );
    a_b.submit_commitment(bidder, c_for_chain_a).unwrap();
    a_b.close_commits(100).unwrap();

    let err = a_b.reveal_bid(bidder, price, n).unwrap_err();
    assert_eq!(
        err,
        AuctionError::RevealMismatch,
        "cross-chain replay must fail at reveal time"
    );
}

// ── Adversarial: evaporation with full valid reveals ─────────────────────────

/// Adversarial — energy evaporation overrides a fully-revealed auction:
/// even when all bidders have revealed valid prices, settle() at a
/// far-future epoch where energy_at(epoch) == 0 must transition the
/// auction to Evaporated, not Settled, and return an error.
///
/// INVENTION_STACK §A5.2: "An auction that is not refreshed (energy → 0)
/// before the reveal window closes evaporates instead of settling."
#[test]
fn adversarial_evaporation_with_full_valid_reveals() {
    let mut a = default_auction();
    let alice = addr(1);
    let bob = addr(2);

    for (bidder, price, n) in [(alice, 5_000u64, nonce(0xA1)), (bob, 3_000, nonce(0xB2))] {
        let c =
            DecayBoundAuction::compute_commitment(&a.chain_id, &a.auction_id, &bidder, price, &n);
        a.submit_commitment(bidder, c).unwrap();
    }
    a.close_commits(100).unwrap();
    a.reveal_bid(alice, 5_000, nonce(0xA1)).unwrap();
    a.reveal_bid(bob, 3_000, nonce(0xB2)).unwrap();
    a.close_reveals(200).unwrap();

    // Settle at epoch 10_000 — far past every half-life; energy == 0.
    let result = a.settle(10_000);
    assert_eq!(
        result,
        Err(AuctionError::EvaporatedDuringReveal),
        "settle at far-future epoch must evaporate even with full reveals"
    );
    assert_eq!(a.phase, AuctionPhase::Evaporated);
    assert!(
        a.winner.is_none(),
        "evaporated auction must not record a winner"
    );
    assert!(
        a.clearing_price.is_none(),
        "evaporated auction must not record a clearing price"
    );
}

/// An evaporated auction must not be settleable a second time.
#[test]
fn adversarial_settle_after_evaporation_rejected() {
    let mut a = default_auction();
    let bidder = addr(1);
    let c = DecayBoundAuction::compute_commitment(
        &a.chain_id,
        &a.auction_id,
        &bidder,
        5_000,
        &nonce(0x01),
    );
    a.submit_commitment(bidder, c).unwrap();
    a.close_commits(100).unwrap();
    a.reveal_bid(bidder, 5_000, nonce(0x01)).unwrap();
    a.close_reveals(200).unwrap();
    // First settle → evaporation.
    let _ = a.settle(10_000);
    assert_eq!(a.phase, AuctionPhase::Evaporated);
    // Second attempt: must be rejected (not in RevealClosed).
    let err = a.settle(10_001).unwrap_err();
    assert_eq!(err, AuctionError::NotRevealClosed(AuctionPhase::Evaporated));
}

// ── Adversarial: phase sequence violations ────────────────────────────────────

/// Reveal cannot be called before commits are closed (Open phase).
#[test]
fn adversarial_reveal_in_open_phase_rejected() {
    let mut a = default_auction();
    let bidder = addr(1);
    let c = DecayBoundAuction::compute_commitment(
        &a.chain_id,
        &a.auction_id,
        &bidder,
        2_000,
        &nonce(0x01),
    );
    a.submit_commitment(bidder, c).unwrap();
    // Phase is still Open — reveal must fail.
    let err = a.reveal_bid(bidder, 2_000, nonce(0x01)).unwrap_err();
    assert_eq!(err, AuctionError::NotCommitClosed(AuctionPhase::Open));
}

/// Settle cannot skip close_reveals.
#[test]
fn adversarial_settle_skips_close_reveals_rejected() {
    let mut a = default_auction();
    let bidder = addr(1);
    let c = DecayBoundAuction::compute_commitment(
        &a.chain_id,
        &a.auction_id,
        &bidder,
        2_000,
        &nonce(0x01),
    );
    a.submit_commitment(bidder, c).unwrap();
    a.close_commits(100).unwrap();
    a.reveal_bid(bidder, 2_000, nonce(0x01)).unwrap();
    // Phase is CommitClosed, not RevealClosed — settle must fail.
    let err = a.settle(150).unwrap_err();
    assert_eq!(
        err,
        AuctionError::NotRevealClosed(AuctionPhase::CommitClosed)
    );
}

/// close_commits cannot be called after auction has already passed Open.
#[test]
fn adversarial_double_close_commits_rejected() {
    let mut a = default_auction();
    a.close_commits(100).unwrap();
    let err = a.close_commits(101).unwrap_err();
    assert_eq!(err, AuctionError::NotOpen(AuctionPhase::CommitClosed));
}

// ── Adversarial: capacity boundary ───────────────────────────────────────────

/// Adding a bidder beyond MAX_BIDDERS_PER_AUCTION must fail deterministically.
/// Verify the exact constant value so future changes are flagged here.
#[test]
fn adversarial_cap_at_max_bidders() {
    assert_eq!(
        MAX_BIDDERS_PER_AUCTION, 1_024,
        "MAX_BIDDERS_PER_AUCTION constant changed — review memory + settle sort bounds"
    );

    let mut a = default_auction();
    for i in 0u64..MAX_BIDDERS_PER_AUCTION as u64 {
        let mut b: AccountAddress = [0u8; 32];
        b[..8].copy_from_slice(&i.to_le_bytes());
        a.submit_commitment(b, BidCommitment([0u8; 32])).unwrap();
    }
    let overflow: AccountAddress = [0xFF; 32];
    let err = a
        .submit_commitment(overflow, BidCommitment([0u8; 32]))
        .unwrap_err();
    assert_eq!(err, AuctionError::TooManyBidders);
}

// ── Doctrine: no-bid settlement ───────────────────────────────────────────────

/// Auction with zero commitments settles with winner=None (no error).
#[test]
fn doctrine_zero_bid_auction_settles_with_no_winner() {
    let mut a = default_auction();
    a.close_commits(100).unwrap();
    a.close_reveals(200).unwrap();
    let winner = a.settle(201).unwrap();
    assert_eq!(winner, None);
    assert_eq!(a.phase, AuctionPhase::Settled);
    assert!(a.clearing_price.is_none());
}

/// All bids below reserve_price → winner=None, phase=Settled (not Evaporated).
#[test]
fn doctrine_all_bids_below_reserve_settles_with_no_winner() {
    let mut a = default_auction();
    for (i, bidder) in [addr(1), addr(2), addr(3)].iter().enumerate() {
        let price = 100 * (i + 1) as u64; // max 300 < reserve 1_000
        let n = nonce(i as u8);
        let c =
            DecayBoundAuction::compute_commitment(&a.chain_id, &a.auction_id, bidder, price, &n);
        a.submit_commitment(*bidder, c).unwrap();
    }
    a.close_commits(100).unwrap();
    for (i, bidder) in [addr(1), addr(2), addr(3)].iter().enumerate() {
        let price = 100 * (i + 1) as u64;
        let n = nonce(i as u8);
        a.reveal_bid(*bidder, price, n).unwrap();
    }
    a.close_reveals(200).unwrap();
    let winner = a.settle(201).unwrap();
    assert_eq!(
        winner, None,
        "no bid meets reserve → no winner but still Settled"
    );
    assert_eq!(a.phase, AuctionPhase::Settled);
}

// ── Doctrine: tie-break on lex-smaller address ────────────────────────────────

/// Three bidders with identical prices — lexicographically smallest address wins.
#[test]
fn doctrine_three_way_tie_lex_smallest_wins() {
    let mut a = default_auction();
    let price = 5_000u64;
    let lo = addr(0x01); // lex smallest
    let mid = addr(0x02);
    let hi = addr(0x03);

    for (bidder, n) in [(lo, nonce(0x01)), (mid, nonce(0x02)), (hi, nonce(0x03))] {
        let c =
            DecayBoundAuction::compute_commitment(&a.chain_id, &a.auction_id, &bidder, price, &n);
        a.submit_commitment(bidder, c).unwrap();
    }
    a.close_commits(100).unwrap();
    a.reveal_bid(lo, price, nonce(0x01)).unwrap();
    a.reveal_bid(mid, price, nonce(0x02)).unwrap();
    a.reveal_bid(hi, price, nonce(0x03)).unwrap();
    a.close_reveals(200).unwrap();
    let winner = a.settle(201).unwrap();
    assert_eq!(
        winner,
        Some(lo),
        "lex smallest address wins on a three-way price tie"
    );
}

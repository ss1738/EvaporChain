//! Clearing rule for SDDC.
//!
//! Given an `Auction` (Open) and an ordered slice of `Bid`s, finds the
//! earliest-submitted bid that satisfies *both*:
//!
//! - `current_price(bid.submitted_at) ≤ bid.max_price` (price axis)
//! - `auction.lot_lambda ≤ bid.lambda_tolerance` (decay axis)
//!
//! and settles the auction at that bid's contemporaneous price.
//!
//! "Earliest-submitted" uses each bid's `submitted_at` epoch — NOT the
//! arrival index in the slice. This keeps clearing canonical across
//! validators even if bids gossip in different orders.
//!
//! Bids submitted before `auction.opened_at` are rejected with
//! `BidBeforeOpen`. Bids past the close window are silently skipped
//! (treated as no-clear); the auction is then marked `Expired` if no
//! valid bid was found before the close.

use evaporchain_types::Energy;
use thiserror::Error;

use crate::auction::{Auction, AuctionStatus, Cleared, LifecycleError};
use crate::bid::Bid;
use crate::pricing::PricingError;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClearError {
    #[error("auction is not open: {0}")]
    Lifecycle(#[from] LifecycleError),
    #[error("pricing failed: {0}")]
    Pricing(#[from] PricingError),
}

/// Try to clear `auction` against the bid stream as of `epoch_now`.
///
/// Returns:
/// - `Ok(Some(Cleared))` and updates `auction.status` to `Cleared(_)`
///   if some bid satisfies both axes.
/// - `Ok(None)` with no state change if the auction is still open and
///   no bid yet clears.
/// - `Ok(None)` and updates `auction.status` to `Expired` if `epoch_now`
///   is past the close window with no clearing bid.
/// - `Err(_)` if the auction is not in `Open` state.
pub fn try_clear(
    auction: &mut Auction,
    bids: &[Bid],
    epoch_now: u64,
) -> Result<Option<Cleared>, ClearError> {
    if !auction.is_open() {
        return Err(LifecycleError::NotOpen.into());
    }

    // Filter to bids submitted within the auction window. Reject bids
    // submitted before opening — those are protocol-invalid (can't bid
    // on a not-yet-open auction).
    for b in bids {
        if b.submitted_at < auction.opened_at {
            return Err(LifecycleError::BidBeforeOpen {
                submitted_at: b.submitted_at,
                opened_at: auction.opened_at,
            }
            .into());
        }
    }

    // Sort by submitted_at; ties broken by bidder address (lex order)
    // so the rule is deterministic across validators.
    let mut ordered: Vec<&Bid> = bids
        .iter()
        .filter(|b| b.submitted_at <= auction.closes_at())
        .collect();
    ordered.sort_by(|a, b| {
        a.submitted_at
            .cmp(&b.submitted_at)
            .then_with(|| a.bidder.cmp(&b.bidder))
    });

    for b in ordered {
        let price_at_bid = auction.price_at(b.submitted_at)?;
        if b.clears(price_at_bid, auction.lot_lambda) {
            auction.settle(b, price_at_bid);
            return Ok(Some(extract_cleared(auction)));
        }
    }

    // No bid cleared. If the window has passed, mark expired.
    if epoch_now > auction.closes_at() {
        auction.expire()?;
    }
    Ok(None)
}

/// Pull the `Cleared` payload out of an auction in `Cleared` status.
/// Caller must have verified status; panics otherwise (internal).
fn extract_cleared(auction: &Auction) -> Cleared {
    match &auction.status {
        AuctionStatus::Cleared(c) => c.clone(),
        _ => unreachable!("settle() guarantees Cleared status"),
    }
}

/// Convenience: peek what `try_clear` *would* settle at, without mutating.
pub fn would_clear_at(auction: &Auction, bid: &Bid) -> Result<Option<Energy>, PricingError> {
    if !auction.is_open() {
        return Ok(None);
    }
    if bid.submitted_at < auction.opened_at || bid.submitted_at > auction.closes_at() {
        return Ok(None);
    }
    let price = auction.price_at(bid.submitted_at)?;
    Ok(if bid.clears(price, auction.lot_lambda) {
        Some(price)
    } else {
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auction::Auction;

    fn aid(b: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    fn open_auction() -> Auction {
        // ceiling=1000, floor=100, λ=20, opens at epoch 0, 100-epoch window.
        Auction::open(aid(1), 1000, 100, 20, 0, 100).unwrap()
    }

    #[test]
    fn no_bids_no_clear_window_open() {
        let mut a = open_auction();
        let cleared = try_clear(&mut a, &[], 50).unwrap();
        assert!(cleared.is_none());
        assert!(a.is_open(), "still open mid-window");
    }

    #[test]
    fn no_bids_past_window_marks_expired() {
        let mut a = open_auction();
        let cleared = try_clear(&mut a, &[], 9999).unwrap();
        assert!(cleared.is_none());
        assert_eq!(a.status, AuctionStatus::Expired);
    }

    #[test]
    fn first_satisfying_bid_clears_at_its_epoch_price() {
        let mut a = open_auction();
        // At epoch 50, price = 550. Bid offers max_price=600 ⇒ clears.
        // λ_tolerance=30 ≥ lot_lambda=20 ⇒ second axis OK.
        let b = Bid::new(aid(0xAA), 600, 30, 50).unwrap();
        let cleared = try_clear(&mut a, &[b], 50).unwrap().unwrap();
        assert_eq!(cleared.winner, aid(0xAA));
        assert_eq!(cleared.price_paid, 550);
        assert_eq!(cleared.cleared_at, 50);
    }

    #[test]
    fn earliest_submitter_among_tied_winners_wins() {
        let mut a = open_auction();
        // Both bids satisfy both axes at their submission epochs:
        // epoch 30 → price 730, epoch 60 → price 460. Both have max_price=800.
        // Earlier submission wins.
        let early = Bid::new(aid(0xBB), 800, 25, 30).unwrap();
        let later = Bid::new(aid(0xCC), 800, 25, 60).unwrap();
        let cleared = try_clear(&mut a, &[later.clone(), early.clone()], 70)
            .unwrap()
            .unwrap();
        assert_eq!(cleared.winner, aid(0xBB));
        assert_eq!(cleared.cleared_at, 30);
        assert_eq!(cleared.price_paid, 730);
    }

    #[test]
    fn price_axis_alone_blocks_clear() {
        let mut a = open_auction();
        // At epoch 0, price = 1000. Bid offers max_price=999 ⇒ rejected even
        // though λ_tolerance=100 is plenty.
        let b = Bid::new(aid(0xAA), 999, 100, 0).unwrap();
        let cleared = try_clear(&mut a, &[b], 50).unwrap();
        assert!(cleared.is_none());
        assert!(a.is_open());
    }

    #[test]
    fn lambda_axis_alone_blocks_clear() {
        let mut a = open_auction();
        // Price at epoch 100 is 100, well within max_price. But λ_tolerance=10
        // is below lot_lambda=20, so the bidder isn't willing to hold this.
        let b = Bid::new(aid(0xAA), 1000, 10, 100).unwrap();
        let cleared = try_clear(&mut a, &[b], 100).unwrap();
        assert!(cleared.is_none());
    }

    #[test]
    fn high_lambda_tolerance_wins_cheaper_lot() {
        // The headline doctrine claim: "high-λ-tolerant bidders win at
        // lower prices." Construct an auction where two bidders submit
        // at different times — the late, cheap bidder wins because the
        // early one had lambda_tolerance below the lot_lambda.
        let mut a = open_auction();
        // Early bidder (epoch 10): can afford the price (700) but
        // tolerates λ ≤ 5 — REJECTED on the λ axis.
        let early_low_tol = Bid::new(aid(0xAA), 1000, 5, 10).unwrap();
        // Late bidder (epoch 80): price at 80 is 280, cheap. λ_tolerance=50
        // ≥ lot_lambda=20 ⇒ both axes satisfied.
        let late_high_tol = Bid::new(aid(0xBB), 300, 50, 80).unwrap();
        let cleared = try_clear(&mut a, &[early_low_tol, late_high_tol], 90)
            .unwrap()
            .unwrap();
        assert_eq!(cleared.winner, aid(0xBB));
        assert_eq!(cleared.price_paid, 280);
    }

    #[test]
    fn bid_before_open_errors() {
        // opened_at = 100; bid submitted at epoch 50.
        let mut a = Auction::open(aid(1), 1000, 100, 20, 100, 100).unwrap();
        let b = Bid::new(aid(0xAA), 1000, 100, 50).unwrap();
        let err = try_clear(&mut a, &[b], 110).unwrap_err();
        assert!(matches!(
            err,
            ClearError::Lifecycle(LifecycleError::BidBeforeOpen { .. })
        ));
    }

    #[test]
    fn bid_past_close_is_silently_skipped() {
        let mut a = open_auction();
        // Bid submitted at epoch 200 — past close at 100. Should be
        // ignored, leaving the auction Expired (epoch_now is also past).
        let b = Bid::new(aid(0xAA), 1000, 100, 200).unwrap();
        let cleared = try_clear(&mut a, &[b], 250).unwrap();
        assert!(cleared.is_none());
        assert_eq!(a.status, AuctionStatus::Expired);
    }

    #[test]
    fn try_clear_on_already_cleared_errors() {
        let mut a = open_auction();
        let b = Bid::new(aid(0xAA), 1000, 100, 50).unwrap();
        try_clear(&mut a, &[b.clone()], 50).unwrap();
        // Auction is now Cleared; further try_clear must error.
        let err = try_clear(&mut a, &[b], 60).unwrap_err();
        assert!(matches!(err, ClearError::Lifecycle(LifecycleError::NotOpen)));
    }

    #[test]
    fn would_clear_at_predicts_settle_price() {
        let a = open_auction();
        let b = Bid::new(aid(0xAA), 1000, 50, 50).unwrap();
        let p = would_clear_at(&a, &b).unwrap().unwrap();
        assert_eq!(p, 550);
    }

    #[test]
    fn would_clear_at_returns_none_when_not_satisfying() {
        let a = open_auction();
        let b = Bid::new(aid(0xAA), 100, 50, 0).unwrap(); // price too low at open
        assert_eq!(would_clear_at(&a, &b).unwrap(), None);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// If a single bid clears, the recorded price must equal the
        /// auction's price-at-submission, and never exceed bid.max_price.
        #[test]
        fn cleared_price_matches_submission_epoch(
            ceiling in 1000u64..1_000_000,
            floor in 0u64..1000,
            duration in 10u64..1_000,
            lot_lambda in 0u64..100,
            tol_extra in 0u64..1_000_000,
            submitted_offset in 0u64..1_000,
            slack in 1u64..1_000_000,
        ) {
            let mut a = Auction::open([0u8; 32], ceiling, floor, lot_lambda, 0, duration).unwrap();
            let submit_at = submitted_offset.min(duration);
            let p_at_submit = a.price_at(submit_at).unwrap();
            let max_price = p_at_submit.saturating_add(slack);
            let lambda_tol = lot_lambda.saturating_add(tol_extra);
            let b = Bid::new([0xAA; 32], max_price, lambda_tol, submit_at).unwrap();
            let cleared = try_clear(&mut a, &[b], submit_at).unwrap().unwrap();
            prop_assert_eq!(cleared.price_paid, p_at_submit);
            prop_assert!(cleared.price_paid <= max_price);
        }
    }
}

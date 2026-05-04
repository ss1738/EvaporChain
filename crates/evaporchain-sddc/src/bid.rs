//! Bidder payload submitted to an SDDC auction.
//!
//! A `Bid` is the two-axis commitment: the maximum price the bidder
//! will pay AND the maximum λ (decay rate) of the asset they're
//! willing to hold. Both axes must clear for the bid to win.
//!
//! Submission timestamp is `Epoch` (consensus time, not wall clock) so
//! validators agree on bid order. First-valid-bid-by-submission wins.

use evaporchain_types::{AccountAddress, DecayRate, Energy, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BidError {
    #[error("max_price must be > 0 to bind a bid")]
    ZeroMaxPrice,
}

/// A two-axis Dutch-auction bid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bid {
    /// Bidder's account.
    pub bidder: AccountAddress,
    /// Cap on price the bidder will pay (in Energy). Bid clears
    /// only when `current_price ≤ max_price`.
    pub max_price: Energy,
    /// Cap on λ (decay rate of the lot) the bidder is willing to
    /// hold. Bid clears only when `lot.lambda ≤ lambda_tolerance`.
    /// Higher tolerance = willing to take faster-decaying assets =
    /// self-selects into riskier (cheaper) clearings.
    pub lambda_tolerance: DecayRate,
    /// Consensus epoch when the bid was accepted into the book.
    /// Determines submission order — earliest bid that satisfies both
    /// axes wins.
    pub submitted_at: Epoch,
}

impl Bid {
    /// Construct a bid, validating non-zero price.
    pub fn new(
        bidder: AccountAddress,
        max_price: Energy,
        lambda_tolerance: DecayRate,
        submitted_at: Epoch,
    ) -> Result<Self, BidError> {
        if max_price == 0 {
            return Err(BidError::ZeroMaxPrice);
        }
        Ok(Self {
            bidder,
            max_price,
            lambda_tolerance,
            submitted_at,
        })
    }

    /// True iff the bid clears against `(current_price, lot_lambda)`.
    /// Pure function; no auction state required.
    pub fn clears(&self, current_price: Energy, lot_lambda: DecayRate) -> bool {
        current_price <= self.max_price && lot_lambda <= self.lambda_tolerance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bidder() -> AccountAddress {
        let mut a = [0u8; 32];
        a[0] = 0xAB;
        a
    }

    #[test]
    fn rejects_zero_max_price() {
        assert_eq!(
            Bid::new(bidder(), 0, 100, 1).unwrap_err(),
            BidError::ZeroMaxPrice
        );
    }

    #[test]
    fn clears_when_both_axes_satisfied() {
        let b = Bid::new(bidder(), 1000, 50, 5).unwrap();
        assert!(b.clears(800, 30));
        assert!(b.clears(1000, 50)); // boundary case — both equal
    }

    #[test]
    fn fails_when_price_too_high() {
        let b = Bid::new(bidder(), 1000, 50, 5).unwrap();
        assert!(!b.clears(1001, 30));
    }

    #[test]
    fn fails_when_lot_decays_faster_than_tolerance() {
        // Asset decays faster than bidder is willing to hold.
        let b = Bid::new(bidder(), 1000, 50, 5).unwrap();
        assert!(!b.clears(800, 51));
    }

    #[test]
    fn round_trip_serde() {
        let b = Bid::new(bidder(), 1234, 77, 99).unwrap();
        let s = serde_json::to_string(&b).unwrap();
        let back: Bid = serde_json::from_str(&s).unwrap();
        assert_eq!(b, back);
    }
}

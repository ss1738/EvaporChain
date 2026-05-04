//! Auction header + lifecycle for SDDC.
//!
//! The auction is a *header* (immutable after `open`) plus a status
//! field tracking whether it is still accepting bids, has cleared, or
//! has expired without clearing. A separate book (e.g. the marketplace
//! crate using SDDC) holds the bid stream — this crate is purely
//! mechanism, not storage.
//!
//! Header invariants (enforced at construction):
//! - `floor ≤ ceiling`
//! - `duration_epochs > 0`
//! - `lot_lambda` carries the published λ of the asset; bidders compare
//!   their `lambda_tolerance` against it.
//!
//! Lifecycle:
//!
//! ```text
//!     open() ────► Open ──── try_clear() ───► Cleared { winning_bid, price }
//!                    │                          │
//!                    │ duration expires         └─► (terminal)
//!                    ▼
//!                  Expired
//! ```

use evaporchain_types::{AccountAddress, DecayRate, Energy, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bid::Bid;
use crate::pricing::{current_price, PricingError};

/// 32-byte opaque auction handle. Caller chooses (often a hash of the
/// lot identifier).
pub type AuctionId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LifecycleError {
    #[error("invalid auction header: {0}")]
    InvalidHeader(#[from] PricingError),
    #[error("auction is not Open")]
    NotOpen,
    #[error("auction window has expired at epoch {now} (closes at {closes_at})")]
    Expired { now: Epoch, closes_at: Epoch },
    #[error("bid was submitted before the auction opened ({submitted_at} < {opened_at})")]
    BidBeforeOpen {
        submitted_at: Epoch,
        opened_at: Epoch,
    },
}

/// Auction lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuctionStatus {
    /// Accepting bids.
    Open,
    /// A bid cleared at the recorded price + epoch.
    Cleared(Cleared),
    /// Window passed without any bid clearing.
    Expired,
}

/// Resolution payload when an auction clears.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cleared {
    pub winner: AccountAddress,
    pub price_paid: Energy,
    pub cleared_at: Epoch,
}

/// SDDC auction header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Auction {
    pub id: AuctionId,
    /// Starting price (highest the seller will accept).
    pub ceiling: Energy,
    /// Floor price after duration elapses.
    pub floor: Energy,
    /// Published λ of the lot — bidders' `lambda_tolerance` is checked
    /// against this. Higher value = lot decays faster.
    pub lot_lambda: DecayRate,
    /// Epoch the auction opened (consensus time).
    pub opened_at: Epoch,
    /// Number of epochs over which the price descends linearly.
    pub duration_epochs: u64,
    /// Lifecycle state.
    pub status: AuctionStatus,
}

impl Auction {
    /// Open an auction with the given parameters. Validates header
    /// invariants up-front.
    pub fn open(
        id: AuctionId,
        ceiling: Energy,
        floor: Energy,
        lot_lambda: DecayRate,
        opened_at: Epoch,
        duration_epochs: u64,
    ) -> Result<Self, LifecycleError> {
        // Run the price function once at open with `epoch_now = opened_at`
        // to fail-fast on bad header (inverted bounds, zero duration).
        current_price(ceiling, floor, opened_at, duration_epochs, opened_at)?;
        Ok(Self {
            id,
            ceiling,
            floor,
            lot_lambda,
            opened_at,
            duration_epochs,
            status: AuctionStatus::Open,
        })
    }

    /// Last epoch the auction is still accepting bids (inclusive).
    pub fn closes_at(&self) -> Epoch {
        self.opened_at.saturating_add(self.duration_epochs)
    }

    /// True iff the auction is currently in `Open` status.
    pub fn is_open(&self) -> bool {
        matches!(self.status, AuctionStatus::Open)
    }

    /// Get the price right now per the descending curve.
    pub fn price_at(&self, epoch_now: Epoch) -> Result<Energy, PricingError> {
        current_price(
            self.ceiling,
            self.floor,
            self.opened_at,
            self.duration_epochs,
            epoch_now,
        )
    }

    /// Mark expired. Idempotent if already expired; errors if cleared.
    pub fn expire(&mut self) -> Result<(), LifecycleError> {
        match &self.status {
            AuctionStatus::Open | AuctionStatus::Expired => {
                self.status = AuctionStatus::Expired;
                Ok(())
            }
            AuctionStatus::Cleared(_) => Err(LifecycleError::NotOpen),
        }
    }

    /// Mark cleared by a winning bid at the resolved price/epoch.
    /// Crate-internal — call sites are in `clearing.rs`.
    pub(crate) fn settle(&mut self, winning: &Bid, price_paid: Energy) {
        self.status = AuctionStatus::Cleared(Cleared {
            winner: winning.bidder,
            price_paid,
            cleared_at: winning.submitted_at,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> AuctionId {
        let mut x = [0u8; 32];
        x[0] = byte;
        x
    }

    #[test]
    fn open_validates_header() {
        let err = Auction::open(id(1), 50, 100, 10, 0, 100).unwrap_err();
        assert!(matches!(
            err,
            LifecycleError::InvalidHeader(PricingError::InvertedBounds)
        ));
    }

    #[test]
    fn open_rejects_zero_duration() {
        let err = Auction::open(id(1), 100, 50, 10, 0, 0).unwrap_err();
        assert!(matches!(
            err,
            LifecycleError::InvalidHeader(PricingError::ZeroDuration)
        ));
    }

    #[test]
    fn open_starts_in_open_state() {
        let a = Auction::open(id(1), 1000, 100, 10, 50, 100).unwrap();
        assert!(a.is_open());
        assert_eq!(a.closes_at(), 150);
    }

    #[test]
    fn price_at_descends_then_clamps() {
        let a = Auction::open(id(1), 1000, 100, 10, 0, 100).unwrap();
        assert_eq!(a.price_at(0).unwrap(), 1000);
        assert_eq!(a.price_at(50).unwrap(), 550);
        assert_eq!(a.price_at(100).unwrap(), 100);
        assert_eq!(a.price_at(9999).unwrap(), 100); // past close, clamps
    }

    #[test]
    fn expire_is_idempotent_from_open_or_expired() {
        let mut a = Auction::open(id(1), 1000, 100, 10, 0, 100).unwrap();
        a.expire().unwrap();
        assert_eq!(a.status, AuctionStatus::Expired);
        // Expiring again is a no-op
        a.expire().unwrap();
        assert_eq!(a.status, AuctionStatus::Expired);
    }

    #[test]
    fn expire_after_clear_errors() {
        let mut a = Auction::open(id(1), 1000, 100, 10, 0, 100).unwrap();
        let bid = Bid::new([0xAB; 32], 600, 20, 50).unwrap();
        a.settle(&bid, 600);
        assert!(matches!(a.expire(), Err(LifecycleError::NotOpen)));
    }

    #[test]
    fn settle_records_winner_and_price() {
        let mut a = Auction::open(id(1), 1000, 100, 10, 0, 100).unwrap();
        let bid = Bid::new([0xCD; 32], 700, 20, 60).unwrap();
        a.settle(&bid, 550);
        match a.status {
            AuctionStatus::Cleared(c) => {
                assert_eq!(c.winner, [0xCD; 32]);
                assert_eq!(c.price_paid, 550);
                assert_eq!(c.cleared_at, 60);
            }
            other => panic!("expected Cleared, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_serde_open() {
        let a = Auction::open(id(2), 1000, 100, 10, 0, 100).unwrap();
        let s = serde_json::to_string(&a).unwrap();
        let back: Auction = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn round_trip_serde_cleared() {
        let mut a = Auction::open(id(2), 1000, 100, 10, 0, 100).unwrap();
        let bid = Bid::new([0x77; 32], 800, 30, 25).unwrap();
        a.settle(&bid, 800);
        let s = serde_json::to_string(&a).unwrap();
        let back: Auction = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }
}

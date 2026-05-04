//! Lease resale via SDDC.
//!
//! Two-axis interpretation:
//! - SDDC `max_price` ⇔ buyer's price cap for the remaining lease term.
//! - SDDC `lambda_tolerance` ⇔ how-much-decay-time-the-buyer-needs (i.e.
//!   buyers with longer-horizon use cases tolerate higher
//!   `lot_lambda` = remaining-term inverse). The `lot_lambda` on the
//!   auction is set to the lease's *epochs_remaining* (so short
//!   remaining time = high λ = only short-horizon buyers clear).
//!
//! Once a buyer clears, the lease's `subject` is updated. The expiry
//! is unchanged.

use evaporchain_sddc::{
    auction::{Auction, AuctionId, Cleared, LifecycleError},
    bid::Bid,
    clearing::{try_clear, ClearError},
};
use evaporchain_types::{AccountAddress, DecayRate, Energy, Epoch};
use thiserror::Error;

use crate::lease::{Lease, LeaseError, LeaseStatus};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MarketError {
    #[error("only the current subject may list the lease for sale")]
    NotCurrentSubject,
    #[error("cannot list an expired lease")]
    Expired,
    #[error("auction setup failed: {0}")]
    AuctionSetup(#[from] LifecycleError),
    #[error("clearing failed: {0}")]
    Clearing(#[from] ClearError),
    #[error("lease subject change failed: {0}")]
    LeaseChange(#[from] LeaseError),
}

/// List the lease for resale. Caller must be the current subject and
/// the lease must be active.
pub fn list_lease_for_sale(
    lease: &Lease,
    caller: AccountAddress,
    auction_id: AuctionId,
    ceiling: Energy,
    floor: Energy,
    opened_at: Epoch,
    duration_epochs: u64,
) -> Result<Auction, MarketError> {
    if caller != lease.subject {
        return Err(MarketError::NotCurrentSubject);
    }
    if lease.effective_status(opened_at) == LeaseStatus::Expired {
        return Err(MarketError::Expired);
    }
    let lot_lambda: DecayRate = lease.epochs_remaining(opened_at);
    Ok(Auction::open(
        auction_id,
        ceiling,
        floor,
        lot_lambda,
        opened_at,
        duration_epochs,
    )?)
}

/// Run SDDC clearing; on win, update the lease's subject to the
/// auction winner.
pub fn settle_lease_resale(
    lease: &mut Lease,
    auction: &mut Auction,
    bids: &[Bid],
    epoch_now: Epoch,
) -> Result<Option<Cleared>, MarketError> {
    let cleared = match try_clear(auction, bids, epoch_now)? {
        Some(c) => c,
        None => return Ok(None),
    };
    let current_subject = lease.subject;
    lease.change_subject(current_subject, cleared.winner)?;
    Ok(Some(cleared))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Capability;

    fn cap() -> Capability {
        Capability::new([0x77; 32], [0xCD; 32]).unwrap()
    }

    fn lease(subject: u8, expires_at: Epoch) -> Lease {
        // Spread the byte across the address so the helper's output
        // matches `[byte; 32]`-style addresses used in test callers.
        Lease::issue([0; 32], cap(), [subject; 32], 0, expires_at).unwrap()
    }

    fn aid(b: u8) -> [u8; 32] {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    #[test]
    fn list_rejects_non_subject() {
        let l = lease(0xAA, 1000);
        let err = list_lease_for_sale(&l, [0xBB; 32], aid(1), 1000, 100, 0, 100)
            .unwrap_err();
        assert_eq!(err, MarketError::NotCurrentSubject);
    }

    #[test]
    fn list_rejects_expired_lease() {
        let l = lease(0xAA, 100);
        // Open auction at epoch 200 — lease already expired.
        let err =
            list_lease_for_sale(&l, [0xAA; 32], aid(1), 1000, 100, 200, 100).unwrap_err();
        assert_eq!(err, MarketError::Expired);
    }

    #[test]
    fn list_lambda_axis_is_epochs_remaining() {
        let l = lease(0xAA, 1000);
        let a = list_lease_for_sale(&l, [0xAA; 32], aid(1), 1000, 100, 100, 50).unwrap();
        assert_eq!(a.lot_lambda, 900); // remaining = 1000 - 100
    }

    #[test]
    fn settle_changes_subject_on_clear() {
        let mut l = lease(0xAA, 10_000);
        let mut a = list_lease_for_sale(&l, [0xAA; 32], aid(1), 1000, 100, 0, 100).unwrap();
        // Bid at epoch 50 — auction price = 550. Bidder accepts max=600
        // and lambda_tolerance=10000 ≥ lot_lambda=10000 (epochs remaining).
        let buyer = [0xCC; 32];
        let b = Bid::new(buyer, 600, 10_000, 50).unwrap();
        let cleared = settle_lease_resale(&mut l, &mut a, &[b], 50).unwrap().unwrap();
        assert_eq!(cleared.winner, buyer);
        assert_eq!(l.subject, buyer);
    }

    #[test]
    fn no_clear_keeps_subject() {
        let mut l = lease(0xAA, 10_000);
        let mut a = list_lease_for_sale(&l, [0xAA; 32], aid(1), 1000, 100, 0, 100).unwrap();
        // Bid below market: max_price=99 at epoch 0 with price=1000 ⇒ no clear.
        let buyer = [0xCC; 32];
        let b = Bid::new(buyer, 99, 100_000, 0).unwrap();
        let res = settle_lease_resale(&mut l, &mut a, &[b], 50).unwrap();
        assert!(res.is_none());
        assert_eq!(l.subject, [0xAA; 32]);
    }
}

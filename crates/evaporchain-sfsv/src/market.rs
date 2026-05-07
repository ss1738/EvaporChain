//! Secondary market — wraps SDDC for selling future-self claims.
//!
//! The market layer is a thin shim. It does NOT re-implement Dutch
//! clearing; it simply translates SFSV-flavoured listing inputs into
//! the SDDC interface, and translates the SDDC clear back into a
//! `transfer_claim` on the underlying vault.
//!
//! Two-axis interpretation for SFSV:
//!
//! - SDDC's `max_price` ⇔ how much the bidder will pay to acquire the
//!   future-self claim (a discount over the eventual payout).
//! - SDDC's `lambda_tolerance` ⇔ the bidder's tolerance for delay until
//!   the vault matures. The lot's `lot_lambda` is set by the listing
//!   creator to reflect how soon they expect the predicate to trip
//!   (higher → more "patient capital required").

use evaporchain_sddc::{
    auction::{Auction, AuctionId, Cleared, LifecycleError},
    bid::Bid,
    clearing::{try_clear, ClearError},
};
use evaporchain_types::{AccountAddress, DecayRate, Energy, Epoch};
use thiserror::Error;

use crate::vault::{Vault, VaultError};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MarketError {
    #[error("only the current holder can list the claim for sale")]
    NotCurrentHolder,
    #[error("vault has already been released — nothing left to sell")]
    VaultReleased,
    #[error("auction setup failed: {0}")]
    AuctionSetup(#[from] LifecycleError),
    #[error("clearing failed: {0}")]
    Clearing(#[from] ClearError),
    #[error("vault transfer failed: {0}")]
    VaultTransfer(#[from] VaultError),
}

/// List the vault's claim for sale, returning a freshly opened SDDC
/// auction. Caller must be the current holder (`vault.current_holder`)
/// — the market doesn't trust a third party to relist someone else's
/// claim.
///
/// `lot_lambda` is the listing's published "patience required" rating.
/// Bidders compare their `lambda_tolerance` against it.
pub fn list_for_sale(
    vault: &Vault,
    caller: AccountAddress,
    auction_id: AuctionId,
    ceiling: Energy,
    floor: Energy,
    lot_lambda: DecayRate,
    opened_at: Epoch,
    duration_epochs: u64,
) -> Result<Auction, MarketError> {
    let holder = vault.current_holder().ok_or(MarketError::VaultReleased)?;
    if caller != holder {
        return Err(MarketError::NotCurrentHolder);
    }
    Ok(Auction::open(
        auction_id,
        ceiling,
        floor,
        lot_lambda,
        opened_at,
        duration_epochs,
    )?)
}

/// Run SDDC clearing against the bid stream and, if a bid wins,
/// transfer the vault claim to the new holder. Returns the SDDC
/// `Cleared` payload on success, `None` if no bid yet clears.
///
/// All-or-nothing: if claim transfer fails after a clear, the auction
/// state is left in `Cleared` but the vault is unchanged — caller
/// should treat this as an internal error and roll back via a higher
/// transaction layer.
pub fn settle_secondary(
    vault: &mut Vault,
    auction: &mut Auction,
    bids: &[Bid],
    epoch_now: Epoch,
) -> Result<Option<Cleared>, MarketError> {
    let cleared = match try_clear(auction, bids, epoch_now)? {
        Some(c) => c,
        None => return Ok(None),
    };
    let holder = vault.current_holder().ok_or(MarketError::VaultReleased)?;
    vault.transfer_claim(holder, cleared.winner)?;
    Ok(Some(cleared))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::predicate::Predicate;
    use crate::vault::Vault;

    fn id(b: u8) -> [u8; 32] {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    fn locked_vault(creator: u8, future_self: u8) -> Vault {
        Vault::create(
            id(0xFF),
            addr(creator),
            addr(future_self),
            1_000,
            Predicate::EpochReached {
                release_epoch: 1_000,
            },
            0,
        )
        .unwrap()
    }

    #[test]
    fn list_for_sale_rejects_non_holder() {
        let v = locked_vault(0xAA, 0xBB);
        // Future-self is the holder; the creator (different address)
        // cannot list someone else's claim.
        let err = list_for_sale(&v, addr(0xAA), id(1), 500, 50, 10, 0, 100).unwrap_err();
        assert_eq!(err, MarketError::NotCurrentHolder);
    }

    #[test]
    fn list_for_sale_opens_auction_for_holder() {
        let v = locked_vault(0xAA, 0xBB);
        let auction = list_for_sale(&v, addr(0xBB), id(1), 500, 50, 10, 0, 100).unwrap();
        assert!(auction.is_open());
    }

    #[test]
    fn settle_secondary_clears_and_transfers_claim() {
        let mut v = locked_vault(0xAA, 0xBB);
        let mut a = list_for_sale(&v, addr(0xBB), id(1), 1000, 100, 20, 0, 100).unwrap();

        // Bid at epoch 50 → price = 550. max_price=600, λ_tol=30 ≥ lot λ=20.
        let bid = Bid::new(addr(0xCC), 600, 30, 50).unwrap();
        let cleared = settle_secondary(&mut v, &mut a, &[bid], 50)
            .unwrap()
            .unwrap();
        assert_eq!(cleared.winner, addr(0xCC));
        assert_eq!(cleared.price_paid, 550);
        // Vault claim is now held by the bidder.
        assert_eq!(v.current_holder(), Some(addr(0xCC)));
    }

    #[test]
    fn settle_secondary_no_clear_keeps_holder() {
        let mut v = locked_vault(0xAA, 0xBB);
        let mut a = list_for_sale(&v, addr(0xBB), id(1), 1000, 100, 20, 0, 100).unwrap();
        // Bid below market (max_price=99 at epoch 0 with price=1000) — no clear.
        let bid = Bid::new(addr(0xCC), 99, 100, 0).unwrap();
        let res = settle_secondary(&mut v, &mut a, &[bid], 50).unwrap();
        assert!(res.is_none());
        assert_eq!(v.current_holder(), Some(addr(0xBB)));
    }

    #[test]
    fn list_for_sale_after_release_errors() {
        let mut v = locked_vault(0xAA, 0xBB);
        v.mark_released(addr(0xBB), 100);
        let err = list_for_sale(&v, addr(0xBB), id(1), 1000, 100, 10, 0, 100).unwrap_err();
        assert_eq!(err, MarketError::VaultReleased);
    }

    #[test]
    fn chained_resale_walks_holder_through_buyers() {
        // Holder chain: future_self (0xBB) → 0xCC → 0xDD.
        let mut v = locked_vault(0xAA, 0xBB);

        // First sale: 0xBB → 0xCC.
        let mut a1 = list_for_sale(&v, addr(0xBB), id(1), 1000, 100, 20, 0, 100).unwrap();
        let bid1 = Bid::new(addr(0xCC), 700, 30, 50).unwrap();
        settle_secondary(&mut v, &mut a1, &[bid1], 50).unwrap();
        assert_eq!(v.current_holder(), Some(addr(0xCC)));

        // Second sale: 0xCC → 0xDD. Notice listing requires holder=0xCC
        // now — the original future_self lost the right to sell.
        let mut a2 = list_for_sale(&v, addr(0xCC), id(2), 800, 80, 15, 200, 100).unwrap();
        let bid2 = Bid::new(addr(0xDD), 500, 20, 250).unwrap();
        settle_secondary(&mut v, &mut a2, &[bid2], 250).unwrap();
        assert_eq!(v.current_holder(), Some(addr(0xDD)));

        // 0xBB cannot list any more — the claim has moved on.
        let err = list_for_sale(&v, addr(0xBB), id(3), 600, 60, 10, 400, 100).unwrap_err();
        assert_eq!(err, MarketError::NotCurrentHolder);
    }
}

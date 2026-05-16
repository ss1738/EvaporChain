//! Vault — the locked deposit + predicate + recipient bundle, plus the
//! on-chain secondary-market **listing state machine**.
//!
//! **EvaporScript-first (2026-05-16, §5 decision A):** the crate now
//! mirrors the `.es` `future_self_vault.es` listing lifecycle so the
//! on-chain listing invariants are adversarially testable in Rust (the
//! `.es` header asserts they are "guarded by the adversarial tests in
//! the SFSV crate"). SDDC still performs Dutch price-clearing *off-chain*
//! (`market.rs`); the *listing state + sale recording* is on-chain here.
//!
//! ```text
//!   create() ──► Locked { holder = future_self }, listing = None
//!       │  list_for_sale(holder,…)         [holder only, not already listed]
//!       ▼
//!   Locked + listing = Some(..)
//!       │  cancel_listing(holder)  → listing = None
//!       │  record_sale(winner, epoch_now)  [listed, not expired]
//!       ▼  → Locked { holder = winner }, listing = None
//!       │  predicate satisfied → payout::payout
//!       ▼
//!   Released { paid_to, payout_at }   (terminal)
//! ```
//!
//! Listing guards mirror the `.es` exactly:
//!   list_for_sale  — not released, caller == holder, not already listed,
//!                     ceiling > floor, duration > 0
//!   cancel_listing — not released, listed, caller == holder
//!   record_sale    — not released, listed, NOT expired
//!                     (`epoch_now > opened_at + duration` ⇒ expired);
//!                     no caller restriction — listing state guards it.

use evaporchain_types::{AccountAddress, Energy, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::predicate::Predicate;

/// 32-byte opaque vault handle. Caller chooses (often a hash of
/// `(creator, deposit, predicate, created_at)`).
pub type VaultId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VaultError {
    #[error("deposit must be > 0")]
    ZeroDeposit,
    #[error("vault is not Locked (already released)")]
    NotLocked,
    #[error("transfer caller {caller:?} is not the current holder {holder:?}")]
    NotCurrentHolder {
        caller: AccountAddress,
        holder: AccountAddress,
    },
    #[error("a listing is already active")]
    AlreadyListed,
    #[error("no active listing")]
    NotListed,
    #[error("listing ceiling must exceed floor")]
    BadListingBounds,
    #[error("listing duration must be > 0")]
    ZeroListingDuration,
    #[error("listing has expired")]
    ListingExpired,
}

/// On-chain secondary-market listing (mirrors the `.es` `list_*` state
/// fields). `None` on the vault means `.es` `listed == false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub ceiling: Energy,
    pub floor: Energy,
    pub opened_at: Epoch,
    pub duration: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VaultStatus {
    /// Vault still holds the deposit; `current_holder` is the address
    /// the payout will go to *if* the predicate trips with no further
    /// secondary trades.
    Locked { current_holder: AccountAddress },
    /// Vault released its deposit to `paid_to` at `payout_at`.
    Released {
        paid_to: AccountAddress,
        payout_at: Epoch,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vault {
    pub id: VaultId,
    pub creator: AccountAddress,
    /// The address the creator wants to pay out by default — typically
    /// `creator`, but a cold-storage / heir flow may target another.
    pub future_self: AccountAddress,
    pub deposit: Energy,
    pub predicate: Predicate,
    pub created_at: Epoch,
    pub status: VaultStatus,
    /// On-chain listing state. `None` ⇔ `.es` `listed == false`.
    pub listing: Option<Listing>,
}

impl Vault {
    /// Create a new vault. Initial holder is `future_self`; not listed.
    pub fn create(
        id: VaultId,
        creator: AccountAddress,
        future_self: AccountAddress,
        deposit: Energy,
        predicate: Predicate,
        created_at: Epoch,
    ) -> Result<Self, VaultError> {
        if deposit == 0 {
            return Err(VaultError::ZeroDeposit);
        }
        Ok(Self {
            id,
            creator,
            future_self,
            deposit,
            predicate,
            created_at,
            status: VaultStatus::Locked {
                current_holder: future_self,
            },
            listing: None,
        })
    }

    pub fn is_locked(&self) -> bool {
        matches!(self.status, VaultStatus::Locked { .. })
    }

    /// `.es` `listed` — false once released or never listed.
    pub fn is_listed(&self) -> bool {
        self.is_locked() && self.listing.is_some()
    }

    /// Currently designated recipient (the holder). `None` if released.
    pub fn current_holder(&self) -> Option<AccountAddress> {
        match self.status {
            VaultStatus::Locked { current_holder } => Some(current_holder),
            VaultStatus::Released { .. } => None,
        }
    }

    /// Transfer the claim to a new holder (bare primitive — caller must
    /// be the current holder). `record_sale` is the listing-guarded
    /// path; this is the lower-level transfer used by it and by tests.
    pub fn transfer_claim(
        &mut self,
        caller: AccountAddress,
        new_holder: AccountAddress,
    ) -> Result<(), VaultError> {
        let holder = self.current_holder().ok_or(VaultError::NotLocked)?;
        if caller != holder {
            return Err(VaultError::NotCurrentHolder { caller, holder });
        }
        self.status = VaultStatus::Locked {
            current_holder: new_holder,
        };
        Ok(())
    }

    /// `.es` `list_for_sale(ceiling, floor, duration)` — open a listing.
    pub fn list_for_sale(
        &mut self,
        caller: AccountAddress,
        ceiling: Energy,
        floor: Energy,
        opened_at: Epoch,
        duration: u64,
    ) -> Result<(), VaultError> {
        let holder = self.current_holder().ok_or(VaultError::NotLocked)?;
        if caller != holder {
            return Err(VaultError::NotCurrentHolder { caller, holder });
        }
        if self.listing.is_some() {
            return Err(VaultError::AlreadyListed);
        }
        if ceiling <= floor {
            return Err(VaultError::BadListingBounds);
        }
        if duration == 0 {
            return Err(VaultError::ZeroListingDuration);
        }
        self.listing = Some(Listing {
            ceiling,
            floor,
            opened_at,
            duration,
        });
        Ok(())
    }

    /// `.es` `cancel_listing()` — only the current holder, must be listed.
    pub fn cancel_listing(&mut self, caller: AccountAddress) -> Result<(), VaultError> {
        let holder = self.current_holder().ok_or(VaultError::NotLocked)?;
        if self.listing.is_none() {
            return Err(VaultError::NotListed);
        }
        if caller != holder {
            return Err(VaultError::NotCurrentHolder { caller, holder });
        }
        self.listing = None;
        Ok(())
    }

    /// `.es` `record_sale(winner_addr)` — record an off-chain-cleared
    /// sale. No caller restriction (the listing state guards it, exactly
    /// as the `.es` notes). Requires: not released, listed, not expired.
    pub fn record_sale(
        &mut self,
        winner: AccountAddress,
        epoch_now: Epoch,
    ) -> Result<(), VaultError> {
        // not released
        if self.current_holder().is_none() {
            return Err(VaultError::NotLocked);
        }
        let listing = self.listing.ok_or(VaultError::NotListed)?;
        // `.es`: expired iff epoch > list_opened_at + list_duration
        if epoch_now > listing.opened_at.saturating_add(listing.duration) {
            return Err(VaultError::ListingExpired);
        }
        self.status = VaultStatus::Locked {
            current_holder: winner,
        };
        self.listing = None;
        Ok(())
    }

    /// Mark released to `paid_to` at `payout_at`. Crate-internal; the
    /// payout module owns the call site. Clears any open listing
    /// (a released vault cannot be sold).
    pub(crate) fn mark_released(&mut self, paid_to: AccountAddress, payout_at: Epoch) {
        self.status = VaultStatus::Released { paid_to, payout_at };
        self.listing = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> VaultId {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn epoch_predicate() -> Predicate {
        Predicate::EpochReached { release_epoch: 100 }
    }

    fn v() -> Vault {
        Vault::create(id(1), addr(0xAA), addr(0xBB), 1000, epoch_predicate(), 10).unwrap()
    }

    #[test]
    fn create_rejects_zero_deposit() {
        let err =
            Vault::create(id(1), addr(0xAA), addr(0xAA), 0, epoch_predicate(), 10).unwrap_err();
        assert_eq!(err, VaultError::ZeroDeposit);
    }

    #[test]
    fn newly_created_holder_is_future_self_and_not_listed() {
        let v = v();
        assert!(v.is_locked());
        assert_eq!(v.current_holder(), Some(addr(0xBB)));
        assert!(!v.is_listed());
        assert!(v.listing.is_none());
    }

    #[test]
    fn transfer_claim_only_by_current_holder() {
        let mut v = v();
        let err = v.transfer_claim(addr(0xAA), addr(0xCC)).unwrap_err();
        assert!(matches!(err, VaultError::NotCurrentHolder { .. }));
        v.transfer_claim(addr(0xBB), addr(0xCC)).unwrap();
        assert_eq!(v.current_holder(), Some(addr(0xCC)));
    }

    #[test]
    fn transfer_after_release_errors() {
        let mut v = v();
        v.mark_released(addr(0xBB), 200);
        let err = v.transfer_claim(addr(0xBB), addr(0xCC)).unwrap_err();
        assert_eq!(err, VaultError::NotLocked);
    }

    // ── listing state machine (mirrors `.es`) ────────────────────────

    #[test]
    fn list_for_sale_happy_path() {
        let mut v = v();
        v.list_for_sale(addr(0xBB), 1000, 100, 5, 50).unwrap();
        assert!(v.is_listed());
        assert_eq!(
            v.listing,
            Some(Listing {
                ceiling: 1000,
                floor: 100,
                opened_at: 5,
                duration: 50
            })
        );
    }

    #[test]
    fn list_for_sale_guards() {
        let mut v = v();
        // non-holder
        assert!(matches!(
            v.list_for_sale(addr(0xAA), 1000, 100, 0, 10).unwrap_err(),
            VaultError::NotCurrentHolder { .. }
        ));
        // ceiling <= floor
        assert_eq!(
            v.list_for_sale(addr(0xBB), 100, 100, 0, 10).unwrap_err(),
            VaultError::BadListingBounds
        );
        // zero duration
        assert_eq!(
            v.list_for_sale(addr(0xBB), 1000, 100, 0, 0).unwrap_err(),
            VaultError::ZeroListingDuration
        );
        // double-list
        v.list_for_sale(addr(0xBB), 1000, 100, 0, 10).unwrap();
        assert_eq!(
            v.list_for_sale(addr(0xBB), 1000, 100, 0, 10).unwrap_err(),
            VaultError::AlreadyListed
        );
    }

    #[test]
    fn cancel_listing_guards() {
        let mut v = v();
        // not listed
        assert_eq!(
            v.cancel_listing(addr(0xBB)).unwrap_err(),
            VaultError::NotListed
        );
        v.list_for_sale(addr(0xBB), 1000, 100, 0, 10).unwrap();
        // non-holder cannot cancel
        assert!(matches!(
            v.cancel_listing(addr(0xAA)).unwrap_err(),
            VaultError::NotCurrentHolder { .. }
        ));
        v.cancel_listing(addr(0xBB)).unwrap();
        assert!(!v.is_listed());
    }

    #[test]
    fn record_sale_transfers_when_listed_and_unexpired() {
        let mut v = v();
        v.list_for_sale(addr(0xBB), 1000, 100, 0, 100).unwrap();
        // epoch 50 ≤ opened_at(0)+duration(100) ⇒ not expired
        v.record_sale(addr(0xCC), 50).unwrap();
        assert_eq!(v.current_holder(), Some(addr(0xCC)));
        assert!(!v.is_listed());
    }

    #[test]
    fn record_sale_guards() {
        let mut v = v();
        // not listed
        assert_eq!(
            v.record_sale(addr(0xCC), 10).unwrap_err(),
            VaultError::NotListed
        );
        v.list_for_sale(addr(0xBB), 1000, 100, 0, 100).unwrap();
        // expired: epoch 101 > 0+100
        assert_eq!(
            v.record_sale(addr(0xCC), 101).unwrap_err(),
            VaultError::ListingExpired
        );
        // boundary epoch == opened+duration is NOT expired
        v.record_sale(addr(0xCC), 100).unwrap();
        assert_eq!(v.current_holder(), Some(addr(0xCC)));
    }

    #[test]
    fn release_clears_listing_and_blocks_market_ops() {
        let mut v = v();
        v.list_for_sale(addr(0xBB), 1000, 100, 0, 100).unwrap();
        v.mark_released(addr(0xBB), 200);
        assert!(!v.is_listed());
        assert_eq!(
            v.list_for_sale(addr(0xBB), 1000, 100, 0, 10).unwrap_err(),
            VaultError::NotLocked
        );
        assert_eq!(
            v.record_sale(addr(0xCC), 1).unwrap_err(),
            VaultError::NotLocked
        );
    }

    #[test]
    fn round_trip_serde() {
        let mut v = Vault::create(id(2), addr(0xAA), addr(0xBB), 777, epoch_predicate(), 5).unwrap();
        v.list_for_sale(addr(0xBB), 900, 90, 1, 9).unwrap();
        let s = serde_json::to_string(&v).unwrap();
        let back: Vault = serde_json::from_str(&s).unwrap();
        assert_eq!(v, back);
    }
}

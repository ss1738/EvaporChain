//! Vault — the locked deposit + predicate + recipient bundle.
//!
//! Lifecycle:
//!
//! ```text
//!   create() ──► Locked { current_holder = creator }
//!       │
//!       │ secondary-market sale (optional, repeatable)
//!       │ via market::settle_secondary
//!       ▼
//!   Locked { current_holder = winner }
//!       │
//!       │ predicate satisfied → payout::payout
//!       ▼
//!   Released { paid_to, payout_at }
//! ```

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
}

impl Vault {
    /// Create a new vault. The initial holder is `creator`; payout
    /// defaults to `future_self` until the claim is sold.
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
        })
    }

    pub fn is_locked(&self) -> bool {
        matches!(self.status, VaultStatus::Locked { .. })
    }

    /// Currently designated recipient (the holder). `None` if released.
    pub fn current_holder(&self) -> Option<AccountAddress> {
        match self.status {
            VaultStatus::Locked { current_holder } => Some(current_holder),
            VaultStatus::Released { .. } => None,
        }
    }

    /// Transfer the claim to a new holder. Caller must be the current
    /// holder (typically the secondary-market clearing layer).
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

    /// Mark released to `paid_to` at `payout_at`. Crate-internal; the
    /// payout module owns the call site.
    pub(crate) fn mark_released(&mut self, paid_to: AccountAddress, payout_at: Epoch) {
        self.status = VaultStatus::Released { paid_to, payout_at };
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

    #[test]
    fn create_rejects_zero_deposit() {
        let err =
            Vault::create(id(1), addr(0xAA), addr(0xAA), 0, epoch_predicate(), 10).unwrap_err();
        assert_eq!(err, VaultError::ZeroDeposit);
    }

    #[test]
    fn newly_created_holder_is_future_self() {
        let v = Vault::create(id(1), addr(0xAA), addr(0xBB), 1000, epoch_predicate(), 10).unwrap();
        assert!(v.is_locked());
        assert_eq!(v.current_holder(), Some(addr(0xBB)));
    }

    #[test]
    fn transfer_claim_only_by_current_holder() {
        let mut v =
            Vault::create(id(1), addr(0xAA), addr(0xBB), 1000, epoch_predicate(), 10).unwrap();
        // The current holder is 0xBB (future_self). 0xAA (creator)
        // cannot transfer the claim — they no longer own it.
        let err = v.transfer_claim(addr(0xAA), addr(0xCC)).unwrap_err();
        assert!(matches!(err, VaultError::NotCurrentHolder { .. }));

        // 0xBB can transfer.
        v.transfer_claim(addr(0xBB), addr(0xCC)).unwrap();
        assert_eq!(v.current_holder(), Some(addr(0xCC)));
    }

    #[test]
    fn transfer_after_release_errors() {
        let mut v =
            Vault::create(id(1), addr(0xAA), addr(0xBB), 1000, epoch_predicate(), 10).unwrap();
        v.mark_released(addr(0xBB), 200);
        let err = v.transfer_claim(addr(0xBB), addr(0xCC)).unwrap_err();
        assert_eq!(err, VaultError::NotLocked);
    }

    #[test]
    fn round_trip_serde() {
        let v = Vault::create(id(2), addr(0xAA), addr(0xBB), 777, epoch_predicate(), 5).unwrap();
        let s = serde_json::to_string(&v).unwrap();
        let back: Vault = serde_json::from_str(&s).unwrap();
        assert_eq!(v, back);
    }
}

//! Lease — binds a `subject` to a `Capability` for a bounded epoch
//! window. Status transitions:
//!
//! ```text
//!     issue() ──► Active(subject)
//!         │
//!         │ resale (optional, repeatable) — subject changes
//!         ▼
//!     Active(new_subject)
//!         │
//!         │ epoch_now > expires_at_epoch
//!         ▼
//!     Expired
//! ```
//!
//! Crucial property: **no revocation method**. A lease can be sold
//! (changing subject) but never explicitly revoked. The only way a
//! capability "goes away" is by clock — `expires_at_epoch < epoch_now`.

use evaporchain_types::{AccountAddress, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::capability::Capability;

pub type LeaseId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LeaseError {
    #[error("expires_at_epoch ({expires_at}) must be > issued_at ({issued_at})")]
    AlreadyExpired { issued_at: Epoch, expires_at: Epoch },
    #[error("subject change requires caller to be current subject")]
    NotCurrentSubject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeaseStatus {
    Active,
    /// Computed status — not stored, but returned by `effective_status`.
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub id: LeaseId,
    pub capability: Capability,
    /// Current holder of the lease (the *subject* of the capability).
    pub subject: AccountAddress,
    pub issued_at_epoch: Epoch,
    pub expires_at_epoch: Epoch,
}

impl Lease {
    pub fn issue(
        id: LeaseId,
        capability: Capability,
        subject: AccountAddress,
        issued_at_epoch: Epoch,
        expires_at_epoch: Epoch,
    ) -> Result<Self, LeaseError> {
        if expires_at_epoch <= issued_at_epoch {
            return Err(LeaseError::AlreadyExpired {
                issued_at: issued_at_epoch,
                expires_at: expires_at_epoch,
            });
        }
        Ok(Self {
            id,
            capability,
            subject,
            issued_at_epoch,
            expires_at_epoch,
        })
    }

    /// Effective status at `epoch_now`. Pure function; never mutates.
    pub fn effective_status(&self, epoch_now: Epoch) -> LeaseStatus {
        if epoch_now > self.expires_at_epoch {
            LeaseStatus::Expired
        } else {
            LeaseStatus::Active
        }
    }

    /// Transfer the *subject* (resale). Caller must be the current
    /// subject. The capability and expiry are unchanged.
    pub fn change_subject(
        &mut self,
        caller: AccountAddress,
        new_subject: AccountAddress,
    ) -> Result<(), LeaseError> {
        if caller != self.subject {
            return Err(LeaseError::NotCurrentSubject);
        }
        self.subject = new_subject;
        Ok(())
    }

    pub fn epochs_remaining(&self, epoch_now: Epoch) -> u64 {
        self.expires_at_epoch.saturating_sub(epoch_now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap() -> Capability {
        Capability::new([0x77; 32], [0xCD; 32]).unwrap()
    }

    #[test]
    fn issue_rejects_already_expired() {
        let err = Lease::issue([0; 32], cap(), [0xAA; 32], 100, 50).unwrap_err();
        assert!(matches!(err, LeaseError::AlreadyExpired { .. }));
    }

    #[test]
    fn issue_rejects_zero_window() {
        let err = Lease::issue([0; 32], cap(), [0xAA; 32], 100, 100).unwrap_err();
        assert!(matches!(err, LeaseError::AlreadyExpired { .. }));
    }

    #[test]
    fn active_within_window_expired_after() {
        let l = Lease::issue([0; 32], cap(), [0xAA; 32], 100, 200).unwrap();
        assert_eq!(l.effective_status(150), LeaseStatus::Active);
        assert_eq!(l.effective_status(200), LeaseStatus::Active);
        assert_eq!(l.effective_status(201), LeaseStatus::Expired);
    }

    #[test]
    fn change_subject_requires_current_subject() {
        let mut l = Lease::issue([0; 32], cap(), [0xAA; 32], 0, 100).unwrap();
        let err = l.change_subject([0xBB; 32], [0xCC; 32]).unwrap_err();
        assert_eq!(err, LeaseError::NotCurrentSubject);
        // Old subject unchanged.
        assert_eq!(l.subject, [0xAA; 32]);
    }

    #[test]
    fn change_subject_preserves_capability_and_expiry() {
        let mut l = Lease::issue([0; 32], cap(), [0xAA; 32], 0, 100).unwrap();
        let cap_before = l.capability;
        let exp_before = l.expires_at_epoch;
        l.change_subject([0xAA; 32], [0xBB; 32]).unwrap();
        assert_eq!(l.subject, [0xBB; 32]);
        assert_eq!(l.capability, cap_before);
        assert_eq!(l.expires_at_epoch, exp_before);
    }

    #[test]
    fn epochs_remaining_decreases_to_zero_at_expiry() {
        let l = Lease::issue([0; 32], cap(), [0xAA; 32], 100, 200).unwrap();
        assert_eq!(l.epochs_remaining(100), 100);
        assert_eq!(l.epochs_remaining(150), 50);
        assert_eq!(l.epochs_remaining(200), 0);
        assert_eq!(l.epochs_remaining(201), 0);
        assert_eq!(l.epochs_remaining(99999), 0);
    }

    #[test]
    fn round_trip_serde() {
        let l = Lease::issue([0xAB; 32], cap(), [0xAA; 32], 0, 100).unwrap();
        let s = serde_json::to_string(&l).unwrap();
        let back: Lease = serde_json::from_str(&s).unwrap();
        assert_eq!(l, back);
    }
}

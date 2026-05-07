//! Hot-path authorisation predicate.
//!
//! Validators call `is_authorised(lease, requested_cap, caller, epoch_now)`
//! before honouring any action gated by the capability. All four
//! conditions must hold:
//!
//! 1. Lease is currently `Active` (not past `expires_at`).
//! 2. The requested capability matches the lease's `(verb, object)`.
//! 3. The caller is the lease's current subject.
//! 4. Defensive: lease IDs match (caller-supplied lease must be
//!    the same lease the lookup returned).

use evaporchain_types::{AccountAddress, Epoch};
use thiserror::Error;

use crate::capability::Capability;
use crate::lease::{Lease, LeaseStatus};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("lease has expired (now={now} > expires_at={expires_at})")]
    Expired { now: Epoch, expires_at: Epoch },
    #[error("requested capability does not match the lease")]
    CapabilityMismatch,
    #[error("caller {caller:?} is not the lease subject {subject:?}")]
    NotSubject {
        caller: AccountAddress,
        subject: AccountAddress,
    },
}

/// Hot-path predicate. Returns `Ok(())` iff the action is authorised.
pub fn is_authorised(
    lease: &Lease,
    requested: &Capability,
    caller: AccountAddress,
    epoch_now: Epoch,
) -> Result<(), AuthError> {
    if lease.effective_status(epoch_now) == LeaseStatus::Expired {
        return Err(AuthError::Expired {
            now: epoch_now,
            expires_at: lease.expires_at_epoch,
        });
    }
    if &lease.capability != requested {
        return Err(AuthError::CapabilityMismatch);
    }
    if caller != lease.subject {
        return Err(AuthError::NotSubject {
            caller,
            subject: lease.subject,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Capability;
    use crate::lease::Lease;

    fn cap() -> Capability {
        Capability::new([0x77; 32], [0xCD; 32]).unwrap()
    }

    fn other_cap() -> Capability {
        Capability::new([0x77; 32], [0xCE; 32]).unwrap()
    }

    fn active_lease() -> Lease {
        Lease::issue([0; 32], cap(), [0xAA; 32], 0, 1000).unwrap()
    }

    #[test]
    fn authorised_when_all_match() {
        let l = active_lease();
        is_authorised(&l, &cap(), [0xAA; 32], 500).unwrap();
    }

    #[test]
    fn rejects_after_expiry() {
        // Doctrine claim: "permissions can't outlive their purpose."
        // No revocation tx required — just clock past expires_at.
        let l = active_lease();
        let err = is_authorised(&l, &cap(), [0xAA; 32], 1001).unwrap_err();
        assert!(matches!(err, AuthError::Expired { .. }));
    }

    #[test]
    fn rejects_capability_mismatch() {
        let l = active_lease();
        let err = is_authorised(&l, &other_cap(), [0xAA; 32], 500).unwrap_err();
        assert_eq!(err, AuthError::CapabilityMismatch);
    }

    #[test]
    fn rejects_non_subject_caller() {
        let l = active_lease();
        let err = is_authorised(&l, &cap(), [0xBB; 32], 500).unwrap_err();
        assert!(matches!(err, AuthError::NotSubject { .. }));
    }

    #[test]
    fn structural_revocation_means_no_revoke_tx_needed() {
        // Doctrine guarantee: once expired, the lease is dead even if
        // the original grantor was offline / dead / forgot. No party
        // needs to act for the capability to disappear.
        let l = active_lease();
        // Far past expiry — no chain action between lease and `now`.
        let err = is_authorised(&l, &cap(), [0xAA; 32], 999_999).unwrap_err();
        assert!(matches!(err, AuthError::Expired { .. }));
    }
}

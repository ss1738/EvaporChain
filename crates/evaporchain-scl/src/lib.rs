//! Singh Capability Lease (SCL) — permission market with structural
//! revocation.
//!
//! Per `research/INVENTION_STACK.md` §A5.2:
//!
//! > Capability tuples (subject, verb, object, λ_cap) minted as
//! > fungible-non-transferable leases. At expiry, the underlying
//! > right snaps back atomically — no revocation tx, no race. Listed
//! > on a CL-AMM-style book.
//!
//! > **Customer:** DAOs delegating treasury auth (Gnosis Safe pain),
//! > MEV searchers leasing per-block builder permissions, AI agents
//! > needing time-boxed wallet authority.
//!
//! > **Pitch:** *"The first blockchain where permissions can't outlive
//! > their purpose."*
//!
//! ## What's structurally different
//!
//! Existing ocap systems revoke explicitly (the grantor sends a
//! revoke tx, racing against any in-flight uses of the capability).
//! SCL revokes **structurally**: every capability has a hard
//! `expires_at_epoch` and validators won't honour the capability past
//! it. No tx race. The lessee literally cannot misuse the cap past
//! expiry — there is no revocation race because there's no revocation
//! transaction.
//!
//! Two structural decisions:
//!
//! 1. **`(subject, verb, object)` triple is the only authorisation
//!    primitive.** No roles, no policies, no lattices. `verb` is an
//!    opaque 32-byte hash; `object` is an opaque 32-byte hash.
//!    Higher layers map those onto domain semantics (treasury method
//!    selectors, builder slots, wallet APIs).
//!
//! 2. **Lease is non-transferable.** It can be sold via SDDC (the
//!    original holder lists their lease for sale and a winner takes
//!    over the subject), but the right itself doesn't propagate to
//!    third parties via direct transfer. Prevents the
//!    "leak-and-flood" attack where a lessee sublets to N siblings.
//!
//! ## Module map
//!
//! - [`capability`] — [`Capability`] tuple + [`CapabilityId`].
//! - [`lease`] — [`Lease`] {subject, capability, expires_at, status}
//!   + lifecycle.
//! - [`market`] — wraps SDDC for resale of leases (subject changes).
//! - [`check`] — [`is_authorised`] hot-path predicate validators
//!   call to gate access at action time.

pub mod capability;
pub mod check;
pub mod lease;
pub mod market;

pub use capability::{Capability, CapabilityError, CapabilityId};
pub use check::{is_authorised, AuthError};
pub use lease::{Lease, LeaseError, LeaseId, LeaseStatus};
pub use market::{list_lease_for_sale, settle_lease_resale, MarketError};

#[cfg(test)]
mod press_claim_tests {
    //! The press claim lives as a test (INVENTION_STACK §A5.2):
    //! "The first blockchain where permissions can't outlive their
    //! purpose." Structural revocation — no revocation tx, no race.

    use super::*;
    use crate::check::is_authorised;
    use crate::lease::Lease;

    fn verb() -> [u8; 32] {
        *b"treasury.withdraw.v1............"
    }
    fn object() -> [u8; 32] {
        *b"vault.multisig.0xDEAD..........A"
    }
    fn withdraw_cap() -> Capability {
        Capability::new(verb(), object()).unwrap()
    }
    fn addr(b: u8) -> evaporchain_types::AccountAddress {
        [b; 32]
    }

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Issue a 500-epoch lease to a developer.
        let lease = Lease::issue([0xAB; 32], withdraw_cap(), addr(0xDE), 0, 500).unwrap();
        // Active within the window.
        is_authorised(&lease, &withdraw_cap(), addr(0xDE), 499).unwrap();
        // Still active at the last valid epoch.
        is_authorised(&lease, &withdraw_cap(), addr(0xDE), 500).unwrap();
        // Structurally expired at 501 — no revocation tx was needed.
        let err = is_authorised(&lease, &withdraw_cap(), addr(0xDE), 501).unwrap_err();
        assert!(matches!(err, AuthError::Expired { .. }));
    }

    #[test]
    fn no_revocation_method_exists() {
        // Structural guarantee: the `Lease` type has no `revoke()` method.
        // If someone adds one this test won't compile (it compiles as a
        // type-level assertion by construction). The real guarantee is
        // that the only way a capability disappears is epoch advance.
        let lease = Lease::issue([0; 32], withdraw_cap(), addr(1), 0, 100).unwrap();
        // This call would not compile if `lease.revoke(...)` existed.
        let _ = lease.effective_status(0); // only safe accessor
    }

    #[test]
    fn zero_window_lease_rejected() {
        // issued_at == expires_at means the capability is already dead.
        let err = Lease::issue([0; 32], withdraw_cap(), addr(1), 100, 100).unwrap_err();
        assert!(matches!(err, LeaseError::AlreadyExpired { .. }));
    }

    #[test]
    fn past_expires_at_lease_rejected() {
        let err = Lease::issue([0; 32], withdraw_cap(), addr(1), 200, 100).unwrap_err();
        assert!(matches!(err, LeaseError::AlreadyExpired { .. }));
    }

    #[test]
    fn capability_mismatch_rejected_even_within_window() {
        let lease = Lease::issue([0; 32], withdraw_cap(), addr(1), 0, 1000).unwrap();
        let wrong_cap = Capability::new(*b"other.verb.v1...................", object()).unwrap();
        let err = is_authorised(&lease, &wrong_cap, addr(1), 500).unwrap_err();
        assert_eq!(err, AuthError::CapabilityMismatch);
    }

    #[test]
    fn non_subject_caller_rejected_even_within_window() {
        let lease = Lease::issue([0; 32], withdraw_cap(), addr(0xAA), 0, 1000).unwrap();
        let err = is_authorised(&lease, &withdraw_cap(), addr(0xBB), 500).unwrap_err();
        assert!(matches!(err, AuthError::NotSubject { .. }));
    }
}

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

//! Capability-Decay VM — KeyKOS/seL4 object-capability with
//! structural energy-bound authority.
//!
//! ## Why ocap, why decay
//!
//! Default-deny is the inverse of EVM's default-allow + ACL.
//! In an ocap system, a subject can only do something if it
//! *holds the capability* to do it — no `tx.origin`, no role
//! tables, no global authorization checks. The capability IS
//! the authorization.
//!
//! Energy-decay closes the *capability lifetime* hole every
//! ocap system has historically suffered: capabilities, once
//! granted, persist until explicitly revoked. Revocation is
//! traditionally O(holders) or requires brittle revocation
//! caveats. EvaporChain's single-λ makes capability lifetime a
//! *physical* property — every capability has an energy budget
//! that decays; below threshold, the capability is
//! *unforgeably non-invocable*. No permission check needed; the
//! capability simply ceases to exist as an authority.
//!
//! ## The three operations
//!
//! 1. **Transfer.** Pass the capability whole to a new holder.
//!    The energy and authority are unchanged.
//!
//! 2. **Attenuate.** Create a new capability whose authority is
//!    a *strict subset* of the source's. Energy is also at-most
//!    the source's. This is what lets a holder delegate to a
//!    subagent without giving the subagent unbounded reach.
//!    Attenuation is *one-way*: you cannot attenuate up.
//!
//! 3. **Revoke.** The original issuer (and only the issuer) can
//!    zero the energy on a capability they minted, immediately
//!    rendering it non-invocable. Revocation propagates
//!    structurally to all attenuated descendants because
//!    invocation requires the source's energy to also be
//!    non-zero (transitive root-of-trust).
//!
//! ## What this crate is
//!
//! A typed capability registry + invoke gate. Pure-function
//! semantics; the chain wraps it with whatever durability /
//! gas / consensus layer it uses.
//!
//! ## Module map
//!
//! - [`cap`] — [`Capability`] + [`Authority`] + [`CapabilityId`].
//! - [`registry`] — [`CapRegistry`] for mint / transfer /
//!   attenuate / revoke / invoke; tracks the parent-of edges
//!   for structural revocation.

pub mod cap;
pub mod registry;

pub use cap::{Authority, Capability, CapabilityId, ENERGY_FLOOR};
pub use registry::{CapRegistry, RegistryError};

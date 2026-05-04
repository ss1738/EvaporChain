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

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Capability-Decay VM is ocap with structural
    /// energy-bound authority. Mint produces an invocable root.
    /// Attenuate creates a strict-subset descendant; the issuer's
    /// `revoke` propagates STRUCTURALLY to that descendant via the
    /// parent-chain walk — no holder enumeration."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let issuer = [0xAA; 32];
        let bob = [0xBB; 32];
        let carol = [0xCC; 32];
        let parent_authority = Authority {
            verb: "read".into(),
            object: [1; 32],
            max_invokes_per_epoch: 100,
        };
        let child_authority = Authority {
            verb: "read".into(),
            object: [1; 32],
            max_invokes_per_epoch: 50, // strict subset
        };

        let mut r = CapRegistry::new();
        let parent_id = r
            .mint(issuer, parent_authority, 1_000, [1u8; 32], 0)
            .unwrap();
        // Root is invocable.
        r.invoke_gate(parent_id).unwrap();

        // Attenuate: parent → child, holder = bob.
        let child_id = r
            .attenuate(issuer, parent_id, child_authority.clone(), 500, bob, [2u8; 32], 0)
            .unwrap();
        r.invoke_gate(child_id).unwrap();

        // Bob can transfer the child to carol.
        r.transfer(bob, child_id, carol).unwrap();

        // Issuer revokes the PARENT — descendant must now fail
        // invocation through the parent-chain walk.
        r.revoke(issuer, parent_id).unwrap();
        assert!(matches!(
            r.invoke_gate(child_id),
            Err(RegistryError::AncestorRevoked)
        ));

        // Wrong-shape attenuation rejected: child must be a strict
        // subset of parent.
        let mut r2 = CapRegistry::new();
        let pid = r2
            .mint(
                issuer,
                Authority {
                    verb: "read".into(),
                    object: [1; 32],
                    max_invokes_per_epoch: 100,
                },
                1_000,
                [1u8; 32],
                0,
            )
            .unwrap();
        let res = r2.attenuate(
            issuer,
            pid,
            Authority {
                verb: "read".into(),
                object: [1; 32],
                max_invokes_per_epoch: 100, // equal, not strict subset
            },
            500,
            bob,
            [3u8; 32],
            0,
        );
        assert!(matches!(res, Err(RegistryError::AttenuationNotStrictSubset)));
    }
}

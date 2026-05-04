//! `CapRegistry` — the typed capability table + invoke gate.
//!
//! Pure-function semantics: every operation takes `&mut self` and
//! returns `Result<_, RegistryError>` deterministically. The
//! chain wraps this with whatever durability layer it uses.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::cap::{Authority, Capability, CapabilityId, ENERGY_FLOOR};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("capability {0:?} does not exist")]
    NotFound(CapabilityId),
    #[error("capability {0:?} already exists (mint of an existing id)")]
    AlreadyExists(CapabilityId),
    #[error("not the holder: caller is not the current holder of capability {0:?}")]
    NotHolder(CapabilityId),
    #[error("not the issuer: caller is not the original issuer of capability {0:?}")]
    NotIssuer(CapabilityId),
    #[error("attenuation must produce a strict subset of the parent's authority")]
    AttenuationNotStrictSubset,
    #[error("attenuation cannot grant more energy than the parent has")]
    AttenuationEnergyExceedsParent,
    #[error("capability {0:?} is no longer invocable (energy ≤ floor)")]
    NonInvocable(CapabilityId),
    #[error("transitive non-invocable: a parent in the chain has decayed below floor")]
    AncestorRevoked,
}

/// Typed capability registry.
///
/// **Audit fix H3**: backed by `BTreeMap` (not `HashMap`) so serde
/// serialisation is canonical (key-sorted). When the chain hashes
/// the registry into state, validators agree on byte-exact bytes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapRegistry {
    /// All known capabilities, indexed by id.
    caps: BTreeMap<CapabilityId, Capability>,
}

impl CapRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.caps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.caps.is_empty()
    }

    pub fn get(&self, id: &CapabilityId) -> Option<&Capability> {
        self.caps.get(id)
    }

    /// Mint a new root capability. The issuer is also the initial
    /// holder.
    ///
    /// `nonce` is a 32-byte unique nonce the chain provides to
    /// prevent two mints with the same `(issuer, authority)` from
    /// colliding on id.
    pub fn mint(
        &mut self,
        issuer: [u8; 32],
        authority: Authority,
        initial_energy: u64,
        nonce: [u8; 32],
        minted_at_epoch: u64,
    ) -> Result<CapabilityId, RegistryError> {
        let id = Capability::derive_id(&issuer, &nonce, &authority);
        if self.caps.contains_key(&id) {
            return Err(RegistryError::AlreadyExists(id));
        }
        let cap = Capability {
            id,
            issuer,
            holder: issuer,
            authority,
            energy: initial_energy,
            parent: None,
            minted_at_epoch,
        };
        self.caps.insert(id, cap);
        Ok(id)
    }

    /// Pass a capability whole to `new_holder`. Caller must be
    /// the current holder.
    pub fn transfer(
        &mut self,
        caller: [u8; 32],
        cap_id: CapabilityId,
        new_holder: [u8; 32],
    ) -> Result<(), RegistryError> {
        let cap = self
            .caps
            .get_mut(&cap_id)
            .ok_or(RegistryError::NotFound(cap_id))?;
        if cap.holder != caller {
            return Err(RegistryError::NotHolder(cap_id));
        }
        cap.holder = new_holder;
        Ok(())
    }

    /// Create an attenuated child capability. The child's
    /// authority must be a *strict subset* of the parent's; the
    /// child's energy must be ≤ the parent's. Caller must hold
    /// the parent.
    ///
    /// The child becomes a fresh capability with the caller as
    /// its issuer-of-record (separately from the root issuer
    /// stored as `parent`'s issuer); revocation by the *root
    /// issuer* still propagates structurally because invocation
    /// walks the parent chain.
    ///
    /// Important: the child's `issuer` field is set to the
    /// *original root issuer* (the source's issuer), not the
    /// caller. This preserves the invariant that "only the root
    /// issuer can revoke." Caller cannot revoke a capability they
    /// merely attenuated from someone else.
    pub fn attenuate(
        &mut self,
        caller: [u8; 32],
        parent_id: CapabilityId,
        child_authority: Authority,
        child_energy: u64,
        new_holder: [u8; 32],
        nonce: [u8; 32],
        minted_at_epoch: u64,
    ) -> Result<CapabilityId, RegistryError> {
        let parent = self
            .caps
            .get(&parent_id)
            .ok_or(RegistryError::NotFound(parent_id))?
            .clone();
        if parent.holder != caller {
            return Err(RegistryError::NotHolder(parent_id));
        }
        if !child_authority.is_strict_subset_of(&parent.authority) {
            return Err(RegistryError::AttenuationNotStrictSubset);
        }
        if child_energy > parent.energy {
            return Err(RegistryError::AttenuationEnergyExceedsParent);
        }
        let id = Capability::derive_id(&parent.issuer, &nonce, &child_authority);
        if self.caps.contains_key(&id) {
            return Err(RegistryError::AlreadyExists(id));
        }
        let child = Capability {
            id,
            issuer: parent.issuer,
            holder: new_holder,
            authority: child_authority,
            energy: child_energy,
            parent: Some(parent_id),
            minted_at_epoch,
        };
        self.caps.insert(id, child);
        Ok(id)
    }

    /// Zero the energy on a capability. Only the original issuer
    /// can revoke. Revocation is structural — descendants
    /// invoking will fail at the parent-chain walk in
    /// [`invoke_gate`].
    pub fn revoke(
        &mut self,
        caller: [u8; 32],
        cap_id: CapabilityId,
    ) -> Result<(), RegistryError> {
        let cap = self
            .caps
            .get_mut(&cap_id)
            .ok_or(RegistryError::NotFound(cap_id))?;
        if cap.issuer != caller {
            return Err(RegistryError::NotIssuer(cap_id));
        }
        cap.energy = 0;
        Ok(())
    }

    /// Mutate a capability's energy directly — used by the chain's
    /// decay tick. The chain calls this once per epoch with the
    /// post-decay energy.
    pub fn set_energy(
        &mut self,
        cap_id: CapabilityId,
        energy: u64,
    ) -> Result<(), RegistryError> {
        let cap = self
            .caps
            .get_mut(&cap_id)
            .ok_or(RegistryError::NotFound(cap_id))?;
        cap.energy = energy;
        Ok(())
    }

    /// Check whether a capability is currently invocable. Returns
    /// Ok iff:
    /// - The cap exists.
    /// - Its energy is > floor.
    /// - Every ancestor in the parent chain (transitively) is
    ///   also live. This is what makes revocation structural —
    ///   revoking a parent kills every descendant for free, with
    ///   no holder enumeration.
    pub fn invoke_gate(&self, cap_id: CapabilityId) -> Result<(), RegistryError> {
        let mut current_id = cap_id;
        loop {
            let cap = self
                .caps
                .get(&current_id)
                .ok_or(RegistryError::NotFound(current_id))?;
            if cap.energy <= ENERGY_FLOOR {
                if current_id == cap_id {
                    return Err(RegistryError::NonInvocable(cap_id));
                } else {
                    return Err(RegistryError::AncestorRevoked);
                }
            }
            match cap.parent {
                None => return Ok(()),
                Some(parent_id) => current_id = parent_id,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth(verb: &str, object: u8, max: u64) -> Authority {
        Authority {
            verb: verb.into(),
            object: [object; 32],
            max_invokes_per_epoch: max,
        }
    }

    fn alice() -> [u8; 32] { [0xAA; 32] }
    fn bob()   -> [u8; 32] { [0xBB; 32] }
    fn carol() -> [u8; 32] { [0xCC; 32] }

    fn nonce(byte: u8) -> [u8; 32] { [byte; 32] }

    // ── mint + invoke ─────────────────────────────────────────────

    #[test]
    fn mint_creates_invocable_capability() {
        let mut r = CapRegistry::new();
        let id = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        r.invoke_gate(id).unwrap();
        assert_eq!(r.get(&id).unwrap().holder, alice());
    }

    #[test]
    fn duplicate_mint_rejected() {
        let mut r = CapRegistry::new();
        r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        let err = r.mint(alice(), auth("read", 1, 100), 500, nonce(1), 0).unwrap_err();
        assert!(matches!(err, RegistryError::AlreadyExists(_)));
    }

    #[test]
    fn unknown_id_invoke_fails() {
        let r = CapRegistry::new();
        let err = r.invoke_gate(CapabilityId([0xFF; 32])).unwrap_err();
        assert!(matches!(err, RegistryError::NotFound(_)));
    }

    // ── transfer ──────────────────────────────────────────────────

    #[test]
    fn transfer_changes_holder() {
        let mut r = CapRegistry::new();
        let id = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        r.transfer(alice(), id, bob()).unwrap();
        assert_eq!(r.get(&id).unwrap().holder, bob());
    }

    #[test]
    fn transfer_by_non_holder_rejected() {
        let mut r = CapRegistry::new();
        let id = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        let err = r.transfer(bob(), id, carol()).unwrap_err();
        assert!(matches!(err, RegistryError::NotHolder(_)));
    }

    #[test]
    fn transfer_does_not_change_authority_or_energy() {
        let mut r = CapRegistry::new();
        let id = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        r.transfer(alice(), id, bob()).unwrap();
        let cap = r.get(&id).unwrap();
        assert_eq!(cap.energy, 1000);
        assert_eq!(cap.authority.max_invokes_per_epoch, 100);
    }

    // ── attenuate ─────────────────────────────────────────────────

    #[test]
    fn attenuation_to_strict_subset_succeeds() {
        let mut r = CapRegistry::new();
        let parent_id = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        let child_id = r.attenuate(
            alice(),
            parent_id,
            auth("read", 1, 50),
            500,
            bob(),
            nonce(2),
            10,
        ).unwrap();
        assert_ne!(parent_id, child_id);
        assert_eq!(r.get(&child_id).unwrap().holder, bob());
        // Attenuated child preserves the root issuer.
        assert_eq!(r.get(&child_id).unwrap().issuer, alice());
    }

    #[test]
    fn attenuation_equal_authority_rejected() {
        // Equal is NOT strict subset — keeps the graph minimal.
        let mut r = CapRegistry::new();
        let parent_id = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        let err = r.attenuate(
            alice(), parent_id, auth("read", 1, 100), 500, bob(), nonce(2), 10,
        ).unwrap_err();
        assert_eq!(err, RegistryError::AttenuationNotStrictSubset);
    }

    #[test]
    fn attenuation_more_authority_rejected() {
        let mut r = CapRegistry::new();
        let parent_id = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        let err = r.attenuate(
            alice(), parent_id, auth("read", 1, 200), 500, bob(), nonce(2), 10,
        ).unwrap_err();
        assert_eq!(err, RegistryError::AttenuationNotStrictSubset);
    }

    #[test]
    fn attenuation_different_verb_rejected() {
        let mut r = CapRegistry::new();
        let parent_id = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        let err = r.attenuate(
            alice(), parent_id, auth("write", 1, 50), 500, bob(), nonce(2), 10,
        ).unwrap_err();
        assert_eq!(err, RegistryError::AttenuationNotStrictSubset);
    }

    #[test]
    fn attenuation_energy_over_parent_rejected() {
        let mut r = CapRegistry::new();
        let parent_id = r.mint(alice(), auth("read", 1, 100), 500, nonce(1), 0).unwrap();
        let err = r.attenuate(
            alice(), parent_id, auth("read", 1, 50), 600, bob(), nonce(2), 10,
        ).unwrap_err();
        assert_eq!(err, RegistryError::AttenuationEnergyExceedsParent);
    }

    #[test]
    fn attenuation_by_non_holder_rejected() {
        let mut r = CapRegistry::new();
        let parent_id = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        let err = r.attenuate(
            bob(), parent_id, auth("read", 1, 50), 500, carol(), nonce(2), 10,
        ).unwrap_err();
        assert!(matches!(err, RegistryError::NotHolder(_)));
    }

    // ── revoke + structural revocation ────────────────────────────

    #[test]
    fn revoke_zeros_energy() {
        let mut r = CapRegistry::new();
        let id = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        r.revoke(alice(), id).unwrap();
        assert_eq!(r.get(&id).unwrap().energy, 0);
        let err = r.invoke_gate(id).unwrap_err();
        assert!(matches!(err, RegistryError::NonInvocable(_)));
    }

    #[test]
    fn non_issuer_cannot_revoke() {
        let mut r = CapRegistry::new();
        let id = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        // Even bob, the holder after a transfer, cannot revoke.
        r.transfer(alice(), id, bob()).unwrap();
        let err = r.revoke(bob(), id).unwrap_err();
        assert!(matches!(err, RegistryError::NotIssuer(_)));
    }

    #[test]
    fn revoke_propagates_structurally_to_descendants() {
        // Alice mints. Alice attenuates to bob. Alice revokes
        // the parent. Bob's child capability becomes
        // unforgeably non-invocable WITHOUT touching the child.
        let mut r = CapRegistry::new();
        let parent = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        let child = r.attenuate(
            alice(), parent, auth("read", 1, 50), 500, bob(), nonce(2), 10,
        ).unwrap();
        r.invoke_gate(child).unwrap(); // child is live before revocation
        r.revoke(alice(), parent).unwrap();
        let err = r.invoke_gate(child).unwrap_err();
        assert_eq!(err, RegistryError::AncestorRevoked);
    }

    #[test]
    fn revoke_propagates_through_two_levels_of_attenuation() {
        // Alice → Bob → Carol. Alice revokes the root; Carol's
        // grandchild is also dead.
        let mut r = CapRegistry::new();
        let l0 = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        let l1 = r.attenuate(alice(), l0, auth("read", 1, 50), 500, bob(), nonce(2), 0).unwrap();
        let l2 = r.attenuate(bob(), l1, auth("read", 1, 25), 250, carol(), nonce(3), 0).unwrap();
        r.invoke_gate(l2).unwrap();
        r.revoke(alice(), l0).unwrap();
        let err = r.invoke_gate(l2).unwrap_err();
        assert_eq!(err, RegistryError::AncestorRevoked);
    }

    // ── decay → structural non-invocability ───────────────────────

    #[test]
    fn decay_below_floor_makes_non_invocable_without_revoke_call() {
        // The chain's decay tick writes energy = 0; no revoke
        // command was issued. The capability is structurally
        // dead anyway.
        let mut r = CapRegistry::new();
        let id = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        r.set_energy(id, 0).unwrap();
        let err = r.invoke_gate(id).unwrap_err();
        assert!(matches!(err, RegistryError::NonInvocable(_)));
    }

    #[test]
    fn decay_propagates_to_descendants_too() {
        let mut r = CapRegistry::new();
        let parent = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        let child = r.attenuate(
            alice(), parent, auth("read", 1, 50), 500, bob(), nonce(2), 10,
        ).unwrap();
        // Parent decays to 0 — the child is structurally dead
        // even though its own energy is 500.
        r.set_energy(parent, 0).unwrap();
        let err = r.invoke_gate(child).unwrap_err();
        assert_eq!(err, RegistryError::AncestorRevoked);
        // But the child's own energy is still 500 — the chain
        // doesn't have to track or update descendants on parent
        // decay. That's the whole point of structural revocation.
        assert_eq!(r.get(&child).unwrap().energy, 500);
    }

    // ── attenuated chain with intermediate decay ──────────────────

    #[test]
    fn intermediate_decay_kills_children_but_not_root() {
        // Alice → Bob. Alice's root is fine; Bob's intermediate
        // dies. Alice's root stays invocable.
        let mut r = CapRegistry::new();
        let l0 = r.mint(alice(), auth("read", 1, 100), 1000, nonce(1), 0).unwrap();
        let l1 = r.attenuate(alice(), l0, auth("read", 1, 50), 500, bob(), nonce(2), 0).unwrap();
        let l2 = r.attenuate(bob(), l1, auth("read", 1, 25), 250, carol(), nonce(3), 0).unwrap();
        r.set_energy(l1, 0).unwrap();
        // l0 is still alive — root is unaffected by l1 decay.
        r.invoke_gate(l0).unwrap();
        // l2 is dead because l1 (an ancestor) is dead.
        let err = r.invoke_gate(l2).unwrap_err();
        assert_eq!(err, RegistryError::AncestorRevoked);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "EvaporChain capabilities have *physical*
        // lifetime — when the energy decays below floor, the
        // capability becomes unforgeably non-invocable. No
        // revocation list, no holder enumeration, no permission
        // re-check on every call. The capability simply ceases
        // to exist as an authority."
        let mut r = CapRegistry::new();
        let id = r.mint(alice(), auth("read", 1, 100), 100, nonce(1), 0).unwrap();
        // Initially invocable.
        r.invoke_gate(id).unwrap();
        // Time passes; the chain's decay model writes the energy
        // down to 0. The chain did NOT call revoke. Yet:
        r.set_energy(id, 0).unwrap();
        assert!(r.invoke_gate(id).is_err());
        // Restoring energy (e.g., a re-mint scheme) makes it
        // invocable again — but in practice, the chain's
        // single-λ decay is unidirectional. This tests the
        // mechanism works either way.
        r.set_energy(id, 1).unwrap();
        r.invoke_gate(id).unwrap();
    }

    proptest::proptest! {
        #[test]
        fn property_attenuated_authority_is_at_most_parent(
            parent_max in 2u64..1000u64,
            attenuation in 1u64..1000u64,
        ) {
            // For any parent_max and any attenuation, the
            // resulting child is rejected if attenuation >=
            // parent_max (not strict subset) and accepted if
            // attenuation < parent_max.
            let mut r = CapRegistry::new();
            let parent = r.mint(alice(), auth("read", 1, parent_max), 1000, nonce(1), 0).unwrap();
            let child_max = attenuation;
            let result = r.attenuate(
                alice(), parent, auth("read", 1, child_max), 500, bob(), nonce(2), 0,
            );
            if child_max < parent_max {
                proptest::prop_assert!(result.is_ok());
            } else {
                proptest::prop_assert_eq!(
                    result.unwrap_err(),
                    RegistryError::AttenuationNotStrictSubset
                );
            }
        }

        #[test]
        fn property_revocation_is_structural_for_any_chain_depth(depth in 1usize..6usize) {
            // Build a chain alice → ... → final. Revoke the
            // root. Every descendant fails invoke_gate.
            let mut r = CapRegistry::new();
            let mut current_holder = alice();
            let mut current_max = 100;
            let mut current_id = r.mint(alice(), auth("read", 1, current_max), 1000, nonce(0), 0).unwrap();
            let root = current_id;
            let mut chain = vec![current_id];
            for d in 0..depth {
                let next_holder = if d % 2 == 0 { bob() } else { carol() };
                current_max -= 10;
                let next = r.attenuate(
                    current_holder,
                    current_id,
                    auth("read", 1, current_max),
                    500,
                    next_holder,
                    nonce((d + 10) as u8),
                    0,
                ).unwrap();
                chain.push(next);
                current_id = next;
                current_holder = next_holder;
            }
            // All live before revocation.
            for id in &chain {
                proptest::prop_assert!(r.invoke_gate(*id).is_ok());
            }
            r.revoke(alice(), root).unwrap();
            // Root: NonInvocable (it IS the cap). Descendants: AncestorRevoked.
            proptest::prop_assert!(matches!(
                r.invoke_gate(root).unwrap_err(),
                RegistryError::NonInvocable(_)
            ));
            for id in &chain[1..] {
                proptest::prop_assert_eq!(
                    r.invoke_gate(*id).unwrap_err(),
                    RegistryError::AncestorRevoked
                );
            }
        }
    }
}

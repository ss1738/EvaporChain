//! End-to-end integration tests for cap-decay-vm.
//!
//! Non-trivial fixture: a 3-level storage access delegation chain modelling
//! a cloud storage API hierarchy:
//!
//!   root (company) → dept_head (engineering) → developer (individual)
//!
//! Each delegation is an attenuation: max_invokes_per_epoch decreases at
//! every level. The test simulates epoch decay by calling `set_energy`
//! (what the chain calls each epoch). When the root's energy crosses the
//! floor, `invoke_gate` on the developer's leaf capability must fail with
//! `AncestorRevoked` — no holder enumeration needed.
//!
//! Adversarial fixture: an attacker tries to mint a capability with the
//! same id as an existing one, tries to invoke a nonexistent id, and tries
//! to revoke a capability they don't own.
//!
//! INVENTION_STACK §4.2: Capability-Decay VM substrate.

use evaporchain_cap_decay_vm::{Authority, CapRegistry, CapabilityId, RegistryError, ENERGY_FLOOR};

// ── Helpers ───────────────────────────────────────────────────────────────

fn storage_authority(max_invokes: u64) -> Authority {
    Authority {
        verb: "read".into(),
        object: [0x53; 32], // storage bucket handle
        max_invokes_per_epoch: max_invokes,
    }
}

const COMPANY: [u8; 32] = [0xC0; 32];
const DEPT_HEAD: [u8; 32] = [0xD0; 32];
const DEVELOPER: [u8; 32] = [0xDE; 32];
const ATTACKER: [u8; 32] = [0xAA; 32];

// ── Non-trivial fixture ───────────────────────────────────────────────────

struct StorageDelegationChain {
    registry: CapRegistry,
    root_id: CapabilityId,
    dept_id: CapabilityId,
    dev_id: CapabilityId,
}

impl StorageDelegationChain {
    fn build() -> Self {
        let mut registry = CapRegistry::new();

        // Company mints root capability: 1000 invokes/epoch, 10_000 energy.
        let root_id = registry
            .mint(COMPANY, storage_authority(1000), 10_000, [0x01; 32], 0)
            .expect("root mint must succeed");

        // Company attenuates to dept_head: 500 invokes/epoch, 5_000 energy.
        let dept_id = registry
            .attenuate(
                COMPANY,
                root_id,
                storage_authority(500),
                5_000,
                DEPT_HEAD,
                [0x02; 32],
                1,
            )
            .expect("dept attenuation must succeed");

        // Dept_head attenuates to developer: 100 invokes/epoch, 1_000 energy.
        let dev_id = registry
            .attenuate(
                DEPT_HEAD,
                dept_id,
                storage_authority(100),
                1_000,
                DEVELOPER,
                [0x03; 32],
                2,
            )
            .expect("developer attenuation must succeed");

        Self {
            registry,
            root_id,
            dept_id,
            dev_id,
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[test]
fn delegation_chain_all_invocable_at_genesis() {
    let chain = StorageDelegationChain::build();
    let r = &chain.registry;
    r.invoke_gate(chain.root_id)
        .expect("root must be invocable");
    r.invoke_gate(chain.dept_id)
        .expect("dept must be invocable");
    r.invoke_gate(chain.dev_id)
        .expect("developer must be invocable");
}

#[test]
fn root_energy_decay_to_floor_makes_entire_chain_non_invocable() {
    let mut chain = StorageDelegationChain::build();

    // Simulate epoch decay: chain writes root's energy to 0.
    chain
        .registry
        .set_energy(chain.root_id, ENERGY_FLOOR)
        .expect("set_energy must succeed");

    // Root itself: non-invocable.
    assert!(
        matches!(
            chain.registry.invoke_gate(chain.root_id),
            Err(RegistryError::NonInvocable(_))
        ),
        "root must be non-invocable at ENERGY_FLOOR"
    );

    // Dept: ancestor (root) revoked → AncestorRevoked propagates.
    assert!(
        matches!(
            chain.registry.invoke_gate(chain.dept_id),
            Err(RegistryError::AncestorRevoked)
        ),
        "dept must see AncestorRevoked when root is at floor"
    );

    // Developer (two hops from root): still AncestorRevoked.
    assert!(
        matches!(
            chain.registry.invoke_gate(chain.dev_id),
            Err(RegistryError::AncestorRevoked)
        ),
        "developer must see AncestorRevoked when root is at floor"
    );
}

#[test]
fn mid_chain_decay_only_affects_descendants() {
    let mut chain = StorageDelegationChain::build();

    // Only dept_id decays to floor; root is still live.
    chain
        .registry
        .set_energy(chain.dept_id, ENERGY_FLOOR)
        .expect("set_energy must succeed");

    // Root is still invocable (no ancestor of root decayed).
    chain
        .registry
        .invoke_gate(chain.root_id)
        .expect("root must still be invocable");

    // Dept: non-invocable directly.
    assert!(matches!(
        chain.registry.invoke_gate(chain.dept_id),
        Err(RegistryError::NonInvocable(_))
    ));

    // Developer: ancestor (dept) revoked.
    assert!(matches!(
        chain.registry.invoke_gate(chain.dev_id),
        Err(RegistryError::AncestorRevoked)
    ));
}

#[test]
fn issuer_explicit_revoke_propagates_to_descendants() {
    let mut chain = StorageDelegationChain::build();

    // Company (issuer of root) revokes root.
    chain
        .registry
        .revoke(COMPANY, chain.root_id)
        .expect("issuer revoke must succeed");

    // Developer (leaf, two hops) must see AncestorRevoked.
    assert!(matches!(
        chain.registry.invoke_gate(chain.dev_id),
        Err(RegistryError::AncestorRevoked)
    ));
}

#[test]
fn transfer_changes_holder_and_old_holder_loses_transfer_right() {
    let mut chain = StorageDelegationChain::build();

    let new_dev: [u8; 32] = [0xED; 32];

    // Developer transfers dev_id to new_dev.
    chain
        .registry
        .transfer(DEVELOPER, chain.dev_id, new_dev)
        .expect("transfer to new_dev must succeed");

    // New holder is now new_dev; capability is still invocable.
    chain
        .registry
        .invoke_gate(chain.dev_id)
        .expect("dev_id must be invocable after transfer");

    // Old developer can no longer transfer (they no longer hold it).
    let result = chain.registry.transfer(DEVELOPER, chain.dev_id, ATTACKER);
    assert!(
        matches!(result, Err(RegistryError::NotHolder(_))),
        "old holder must not be able to transfer again"
    );
}

// ── Adversarial tests ────────────────────────────────────────────────────

#[test]
fn adversarial_mint_duplicate_id_rejected() {
    let mut chain = StorageDelegationChain::build();

    // Try to mint a capability with the same nonce/issuer/authority
    // that produces a collision — registry must reject second mint.
    let result = chain.registry.mint(
        COMPANY,
        storage_authority(1000),
        10_000,
        [0x01; 32], // same params → same computed id
        0,
    );
    assert!(
        matches!(result, Err(RegistryError::AlreadyExists(_))),
        "duplicate mint must be rejected"
    );
}

#[test]
fn adversarial_invoke_nonexistent_id_rejected() {
    let chain = StorageDelegationChain::build();
    let phantom = CapabilityId([0xFF; 32]);
    assert!(matches!(
        chain.registry.invoke_gate(phantom),
        Err(RegistryError::NotFound(_))
    ));
}

#[test]
fn adversarial_revoke_by_non_issuer_rejected() {
    let mut chain = StorageDelegationChain::build();

    // Attacker tries to revoke the root (they are not the issuer).
    let result = chain.registry.revoke(ATTACKER, chain.root_id);
    assert!(
        matches!(result, Err(RegistryError::NotIssuer(_))),
        "non-issuer must not be able to revoke"
    );

    // Root must still be invocable.
    chain
        .registry
        .invoke_gate(chain.root_id)
        .expect("root must survive attempted non-issuer revoke");
}

#[test]
fn adversarial_attenuation_exceeding_parent_authority_rejected() {
    let mut r = CapRegistry::new();
    let root = r
        .mint(COMPANY, storage_authority(100), 1_000, [0x10; 32], 0)
        .unwrap();

    // Try to attenuate UP: child requests 200 invokes > parent's 100.
    let result = r.attenuate(
        COMPANY,
        root,
        storage_authority(200),
        500,
        DEPT_HEAD,
        [0x11; 32],
        0,
    );
    assert!(
        matches!(result, Err(RegistryError::AttenuationNotStrictSubset)),
        "upward attenuation must be rejected"
    );
}

#[test]
fn adversarial_attenuation_exceeding_parent_energy_rejected() {
    let mut r = CapRegistry::new();
    let root = r
        .mint(COMPANY, storage_authority(100), 1_000, [0x20; 32], 0)
        .unwrap();

    // Child requests 2000 energy — more than parent's 1000.
    let result = r.attenuate(
        COMPANY,
        root,
        storage_authority(50),
        2_000,
        DEPT_HEAD,
        [0x21; 32],
        0,
    );
    assert!(
        matches!(result, Err(RegistryError::AttenuationEnergyExceedsParent)),
        "energy-exceeding attenuation must be rejected"
    );
}

#[test]
fn energy_partial_decay_preserves_invocability_until_floor() {
    let mut r = CapRegistry::new();
    let id = r
        .mint(COMPANY, storage_authority(100), 1_000, [0x30; 32], 0)
        .unwrap();

    // Decay to 1 (above floor=0) — still invocable.
    r.set_energy(id, 1).unwrap();
    r.invoke_gate(id).expect("energy=1 must still be invocable");

    // Decay to floor — non-invocable.
    r.set_energy(id, ENERGY_FLOOR).unwrap();
    assert!(matches!(
        r.invoke_gate(id),
        Err(RegistryError::NonInvocable(_))
    ));
}

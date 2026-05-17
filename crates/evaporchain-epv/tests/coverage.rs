//! Coverage tests for Evaporative Protocol Versioning (EPV) — Tier-0
//! theorem-grade primitive per `INVENTION_STACK.md §A1.2 T5`. Old
//! protocol versions decay below E_min and become physically
//! un-runnable.
//!
//! Existing in-module tests cover the press-claim happy path. This
//! file adds:
//!
//!   - `ProtocolVersion::new` field-assignment pin
//!   - `EpvRegistry` lookup helpers (`get`, `contains`, `len`,
//!     `is_empty`, `iter`, `entries`)
//!   - `is_runnable` boundary cases (exact e_min, u64::MAX e_min,
//!     pre-activation t)
//!   - `prune_evaporated` boundary cases (no-op when below cap, all-
//!     drop, multiple-version order)
//!   - Serde round-trips for `ProtocolVersion`, `EpvRegistry`,
//!     `PruneOutcome`
//!   - `RegistryError` Display + Eq

use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_epv::{
    prune_evaporated, EpvRegistry, ProtocolVersion, PruneOutcome, RegistryError,
};

fn lambda_100() -> ChainLambda {
    ChainLambda::new(Lambda::from_epochs(100))
}

// =================================================================
// ProtocolVersion::new + field invariants
// =================================================================

#[test]
fn protocol_version_new_assigns_all_fields() {
    let v = ProtocolVersion::new(42, 7_777, 12_345);
    assert_eq!(v.id, 42);
    assert_eq!(v.seed_energy, 7_777);
    assert_eq!(v.activated_epoch, 12_345);
}

#[test]
fn remaining_at_clamps_to_seed_for_t_equal_to_activation() {
    let v = ProtocolVersion::new(1, 1_000, 100);
    assert_eq!(v.remaining_at(lambda_100(), 100), 1_000);
}

// =================================================================
// EpvRegistry lookup helpers
// =================================================================

#[test]
fn empty_registry_helpers() {
    let r = EpvRegistry::new();
    assert_eq!(r.len(), 0);
    assert!(r.is_empty());
    assert!(r.get(0).is_none());
    assert!(!r.contains(0));
    assert_eq!(r.iter().count(), 0);
    assert_eq!(r.entries().count(), 0);
}

#[test]
fn registry_get_and_contains_for_registered_version() {
    let mut r = EpvRegistry::new();
    let v = ProtocolVersion::new(7, 1_000, 0);
    r.register(v).unwrap();
    assert_eq!(r.get(7), Some(&v));
    assert!(r.contains(7));
    assert!(!r.contains(8));
    assert_eq!(r.get(8), None);
}

#[test]
fn registry_len_and_is_empty_track_register() {
    let mut r = EpvRegistry::new();
    r.register(ProtocolVersion::new(1, 1_000, 0)).unwrap();
    assert_eq!(r.len(), 1);
    assert!(!r.is_empty());
    r.register(ProtocolVersion::new(2, 500, 0)).unwrap();
    assert_eq!(r.len(), 2);
}

#[test]
fn iter_returns_versions_sorted_by_id() {
    let mut r = EpvRegistry::new();
    // Register out of order.
    r.register(ProtocolVersion::new(5, 100, 0)).unwrap();
    r.register(ProtocolVersion::new(1, 100, 0)).unwrap();
    r.register(ProtocolVersion::new(3, 100, 0)).unwrap();
    let ids: Vec<u32> = r.iter().map(|v| v.id).collect();
    assert_eq!(ids, vec![1, 3, 5], "BTreeMap iter is sorted by VersionId");
}

#[test]
fn entries_returns_id_version_pairs_sorted() {
    let mut r = EpvRegistry::new();
    r.register(ProtocolVersion::new(2, 100, 0)).unwrap();
    r.register(ProtocolVersion::new(1, 200, 5)).unwrap();
    let pairs: Vec<(u32, u64)> = r
        .entries()
        .map(|(id, v)| (*id, v.seed_energy))
        .collect();
    assert_eq!(pairs, vec![(1, 200), (2, 100)]);
}

// =================================================================
// is_runnable boundary cases
// =================================================================

#[test]
fn is_runnable_strict_greater_than_e_min() {
    let mut r = EpvRegistry::new();
    r.register(ProtocolVersion::new(1, 1_000, 0)).unwrap();
    // At t=0, remaining = 1000. With e_min = 999 → 1000 > 999 → runnable.
    assert!(r.is_runnable(1, lambda_100(), 0, 999));
    // With e_min = 1000 → 1000 > 1000 is false → NOT runnable.
    assert!(!r.is_runnable(1, lambda_100(), 0, 1_000));
    // With e_min = 1001 → 1000 > 1001 is false → NOT runnable.
    assert!(!r.is_runnable(1, lambda_100(), 0, 1_001));
}

#[test]
fn is_runnable_with_u64_max_e_min_always_false() {
    let mut r = EpvRegistry::new();
    r.register(ProtocolVersion::new(1, u64::MAX, 0)).unwrap();
    // Even with max seed, e_min = u64::MAX means remaining > MAX is
    // unreachable.
    assert!(!r.is_runnable(1, lambda_100(), 0, u64::MAX));
}

#[test]
fn is_runnable_pre_activation_uses_seed_clamp() {
    let mut r = EpvRegistry::new();
    r.register(ProtocolVersion::new(1, 1_000, 100)).unwrap();
    // t < activated_epoch → remaining clamps to seed → 1000 > 999 → runnable.
    assert!(r.is_runnable(1, lambda_100(), 50, 999));
    assert!(r.is_runnable(1, lambda_100(), 0, 999));
}

// =================================================================
// prune_evaporated boundary cases
// =================================================================

#[test]
fn prune_with_e_min_zero_drops_only_fully_evaporated() {
    let mut r = EpvRegistry::new();
    r.register(ProtocolVersion::new(1, 1_000, 0)).unwrap();
    // At t = 10_000 with half_life=100 → 100 halvings → 0 (well below 0+1).
    // e_min=0 means "drop iff remaining ≤ 0". Pinning that ≤ is the
    // operator gate, not <.
    let out = prune_evaporated(&mut r, lambda_100(), 10_000, 0);
    assert_eq!(out.pruned, vec![1]);
    assert!(!r.contains(1));
}

#[test]
fn prune_with_high_e_min_drops_all_fresh_versions() {
    let mut r = EpvRegistry::new();
    r.register(ProtocolVersion::new(1, 1_000, 0)).unwrap();
    r.register(ProtocolVersion::new(2, 500, 0)).unwrap();
    r.register(ProtocolVersion::new(3, 2_000, 0)).unwrap();
    // e_min = 5000 → every version's seed is below → all pruned at t=0.
    let out = prune_evaporated(&mut r, lambda_100(), 0, 5_000);
    assert_eq!(out.pruned, vec![1, 2, 3], "all three dropped in id order");
    assert!(r.is_empty());
}

#[test]
fn prune_returns_pruned_ids_in_sorted_order() {
    let mut r = EpvRegistry::new();
    // Register out of order.
    r.register(ProtocolVersion::new(7, 10, 0)).unwrap();
    r.register(ProtocolVersion::new(3, 10, 0)).unwrap();
    r.register(ProtocolVersion::new(5, 10, 0)).unwrap();
    let out = prune_evaporated(&mut r, lambda_100(), 0, 100);
    // Iter walks BTreeMap → sorted IDs.
    assert_eq!(out.pruned, vec![3, 5, 7]);
}

// =================================================================
// Serde round-trips
// =================================================================

#[test]
fn protocol_version_serde_round_trips() {
    let v = ProtocolVersion::new(42, 1_234, 5_678);
    let json = serde_json::to_string(&v).unwrap();
    let back: ProtocolVersion = serde_json::from_str(&json).unwrap();
    assert_eq!(back, v);
}

#[test]
fn epv_registry_serde_round_trip_preserves_versions() {
    let mut r = EpvRegistry::new();
    r.register(ProtocolVersion::new(1, 100, 0)).unwrap();
    r.register(ProtocolVersion::new(2, 200, 5)).unwrap();
    let json = serde_json::to_string(&r).unwrap();
    let back: EpvRegistry = serde_json::from_str(&json).unwrap();
    assert_eq!(back.len(), 2);
    assert_eq!(back.get(1).unwrap().seed_energy, 100);
    assert_eq!(back.get(2).unwrap().activated_epoch, 5);
}

#[test]
fn prune_outcome_serde_round_trips() {
    let p = PruneOutcome { pruned: vec![1, 2, 3] };
    let json = serde_json::to_string(&p).unwrap();
    let back: PruneOutcome = serde_json::from_str(&json).unwrap();
    assert_eq!(back, p);
}

// =================================================================
// RegistryError
// =================================================================

#[test]
fn registry_error_displays_both_variants() {
    let a = RegistryError::AlreadyRegistered(7).to_string();
    let u = RegistryError::UnknownVersion(99).to_string();
    assert!(a.contains("7"), "got: {a}");
    assert!(u.contains("99"), "got: {u}");
}

#[test]
fn registry_error_eq_discriminates() {
    assert_eq!(
        RegistryError::AlreadyRegistered(1),
        RegistryError::AlreadyRegistered(1)
    );
    assert_ne!(
        RegistryError::AlreadyRegistered(1),
        RegistryError::AlreadyRegistered(2)
    );
    assert_ne!(
        RegistryError::AlreadyRegistered(1),
        RegistryError::UnknownVersion(1)
    );
}

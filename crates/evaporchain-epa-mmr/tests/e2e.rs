//! End-to-end integration tests for evaporchain-epa-mmr.
//!
//! Non-trivial fixture: "8-leaf MMR with energy decay floor".
//!
//!   Build an 8-leaf MMR. Leaves 0-7: value_hash=[leaf_num;32],
//!   initial energy=1_000, minted_at_epoch=0.
//!
//!   8 leaves -> one peak of size 8, height 3.
//!   Each sibling path has exactly 3 elements.
//!
//!   Phase 1 -- all proofs pass at floor=500 (energy 1_000 > 500).
//!   Phase 2 -- prove leaf 3 at floor=2_000 fails (1_000 < 2_000).
//!              same proof at floor=0 still passes (hash chain valid).
//!   Phase 3 -- decay tick: update_energy(3, 50).
//!              root changes (leaf hash binds energy).
//!              new proof for leaf 3 at floor=100 fails (50 < 100).
//!              new proof for leaf 3 at floor=25 passes (50 >= 25).
//!              old proof against new root -> RootMismatch.
//!              leaves 0,1,2 unaffected: proof at floor=500 still passes.
//!   Phase 4 -- tamper detection:
//!              flip one byte of sibling_path[0] -> RootMismatch.
//!              tamper leaf.energy (pump it) -> RootMismatch (hash chain breaks).
//!
//! Doctrine claim (INVENTION_STACK.md §4.3 / EPA-MMR):
//! "EPA-MMR is an append-only log with O(log N) inclusion proofs that
//!  become STRUCTURALLY INVALID once a leaf's energy decays below a
//!  verifier-supplied floor. Decayed leaves are unprovable, not just
//!  hidden. Provability is a function of energy, not just topology.
//!  Domain-separated leaf/inner/peak/bag hashes prevent second-preimage
//!  attacks across tree levels."
//!
//! Adversarial fixture: EnergyBelowFloor (energy-floor gate fires before
//! hash check), RootMismatch (tampered proof), MalformedPath (wrong path
//! length), LeafOutOfRange.
//!
//! INVENTION_STACK §4.3: EPA-MMR (Tier-3 App-Layer).

use evaporchain_epa_mmr::{
    verify_inclusion, EnergyLeaf, EpaMmr, InclusionProof, MmrError, ProofError,
};
use evaporchain_epa_mmr::proof::{inner_hash, peak_bag_hash};

// -- Helpers ------------------------------------------------------------------

fn leaf(v: u8, energy: u64) -> EnergyLeaf {
    EnergyLeaf::new([v; 32], energy, 0)
}

fn build_8_leaf_mmr() -> EpaMmr {
    let mut m = EpaMmr::new();
    for i in 1..=8u8 {
        m.append(leaf(i, 1_000));
    }
    m
}

// -- Non-trivial fixture ------------------------------------------------------

#[test]
fn fixture_8_leaf_all_proofs_above_floor() {
    // Phase 1: all 8 leaves provable at floor=500.
    let m = build_8_leaf_mmr();
    let root = m.root().unwrap();

    // 8 leaves -> single peak of size 8, height 3: path length 3 for every leaf.
    assert_eq!(m.leaf_count(), 8);
    assert_eq!(m.peaks().len(), 1, "8 = 2^3 -> one peak");

    for i in 0..8 {
        let proof = InclusionProof::build(&m, i).unwrap();
        assert_eq!(proof.sibling_path.len(), 3, "leaf {i}: path must be height-3");
        verify_inclusion(&proof, &root, 500).unwrap();
    }
}

#[test]
fn fixture_floor_above_leaf_energy_rejects() {
    // Phase 2: leaf 3 energy=1_000; floor=2_000 -> EnergyBelowFloor.
    let m = build_8_leaf_mmr();
    let root = m.root().unwrap();

    let proof3 = InclusionProof::build(&m, 3).unwrap();

    // Floor > energy: rejected.
    let err = verify_inclusion(&proof3, &root, 2_000).unwrap_err();
    assert_eq!(
        err,
        ProofError::EnergyBelowFloor { energy: 1_000, floor: 2_000 }
    );

    // Floor == energy: still passes (energy >= floor).
    verify_inclusion(&proof3, &root, 1_000).unwrap();

    // Floor = 0: always passes.
    verify_inclusion(&proof3, &root, 0).unwrap();
}

#[test]
fn fixture_decay_tick_invalidates_proofs() {
    // Phase 3: update_energy(3, 50) -> root changes.
    let mut m = build_8_leaf_mmr();
    let root_before = m.root().unwrap();
    let old_proof3 = InclusionProof::build(&m, 3).unwrap();

    // Simulate decay tick.
    m.update_energy(3, 50).unwrap();
    let root_after = m.root().unwrap();
    assert_ne!(root_before, root_after, "root must change after energy update");

    // New proof for leaf 3 with new root.
    let new_proof3 = InclusionProof::build(&m, 3).unwrap();
    assert_eq!(new_proof3.leaf.energy, 50);

    // Floor=100 > 50: EnergyBelowFloor.
    let err = verify_inclusion(&new_proof3, &root_after, 100).unwrap_err();
    assert_eq!(
        err,
        ProofError::EnergyBelowFloor { energy: 50, floor: 100 }
    );

    // Floor=25 <= 50: passes.
    verify_inclusion(&new_proof3, &root_after, 25).unwrap();

    // Old proof against new root -> RootMismatch (leaf hash uses old energy=1_000).
    let err2 = verify_inclusion(&old_proof3, &root_after, 0).unwrap_err();
    assert_eq!(err2, ProofError::RootMismatch);

    // Leaves other than 3 are unaffected.
    let proof0 = InclusionProof::build(&m, 0).unwrap();
    verify_inclusion(&proof0, &root_after, 500).unwrap();
    let proof7 = InclusionProof::build(&m, 7).unwrap();
    verify_inclusion(&proof7, &root_after, 500).unwrap();
}

#[test]
fn fixture_floor_check_fires_before_hash_check() {
    // Even with a bogus root, EnergyBelowFloor fires first -- not RootMismatch.
    let m = build_8_leaf_mmr();
    let proof = InclusionProof::build(&m, 0).unwrap();

    // Replace root with garbage AND ask for floor above energy.
    let err = verify_inclusion(&proof, &[0xFF; 32], 2_000).unwrap_err();
    assert!(
        matches!(err, ProofError::EnergyBelowFloor { .. }),
        "floor check must fire before hash check; got {err:?}"
    );
}

// -- Doctrine tests -----------------------------------------------------------

#[test]
fn doctrine_decayed_leaf_unprovable_healthy_leaf_remains_provable() {
    // The load-bearing decay-native invariant: a leaf below the floor
    // is structurally unprovable while other leaves remain provable.
    let mut m = EpaMmr::new();
    for i in 0..4u8 {
        m.append(leaf(i, 1_000));
    }
    let root_initial = m.root().unwrap();

    // All four proofs pass above floor=100.
    for i in 0..4 {
        let p = InclusionProof::build(&m, i).unwrap();
        verify_inclusion(&p, &root_initial, 100).unwrap();
    }

    // Decay tick on leaf 1: energy 1_000 -> 30.
    m.update_energy(1, 30).unwrap();
    let root2 = m.root().unwrap();

    // Leaf 1 is unprovable above floor=100.
    let p1 = InclusionProof::build(&m, 1).unwrap();
    let err = verify_inclusion(&p1, &root2, 100).unwrap_err();
    assert!(matches!(err, ProofError::EnergyBelowFloor { energy: 30, floor: 100 }));

    // Leaves 0, 2, 3 remain provable.
    for i in [0usize, 2, 3] {
        let p = InclusionProof::build(&m, i).unwrap();
        verify_inclusion(&p, &root2, 100).unwrap();
    }
}

#[test]
fn doctrine_domain_separation_inner_vs_peak_bag() {
    // The two domain tags must produce different hashes for identical inputs.
    let a = [0x11u8; 32];
    let b = [0x22u8; 32];
    assert_ne!(
        inner_hash(&a, &b),
        peak_bag_hash(&a, &b),
        "inner and peak-bag domain tags must differ"
    );
    // Also non-commutative.
    assert_ne!(inner_hash(&a, &b), inner_hash(&b, &a));
    assert_ne!(peak_bag_hash(&a, &b), peak_bag_hash(&b, &a));
}

#[test]
fn doctrine_update_energy_visible_via_get() {
    let mut m = build_8_leaf_mmr();
    assert_eq!(m.get(5).unwrap().energy, 1_000);
    m.update_energy(5, 300).unwrap();
    assert_eq!(m.get(5).unwrap().energy, 300);
}

#[test]
fn doctrine_root_deterministic_across_equivalent_mmrs() {
    let m1 = build_8_leaf_mmr();
    let m2 = build_8_leaf_mmr();
    assert_eq!(m1.root().unwrap(), m2.root().unwrap());
}

// -- Adversarial fixture ------------------------------------------------------

#[test]
fn adversarial_leaf_out_of_range_rejected() {
    let m = build_8_leaf_mmr();
    let err = InclusionProof::build(&m, 100).unwrap_err();
    assert!(matches!(err, ProofError::LeafOutOfRange { idx: 100, len: 8 }));
}

#[test]
fn adversarial_tampered_sibling_byte_rejected() {
    let m = build_8_leaf_mmr();
    let root = m.root().unwrap();
    let mut proof = InclusionProof::build(&m, 3).unwrap();

    proof.sibling_path[0][0] ^= 0xFF;
    let err = verify_inclusion(&proof, &root, 0).unwrap_err();
    assert_eq!(err, ProofError::RootMismatch);
}

#[test]
fn adversarial_padded_sibling_path_yields_malformed() {
    let m = build_8_leaf_mmr();
    let root = m.root().unwrap();
    let mut proof = InclusionProof::build(&m, 3).unwrap();

    // Original path len is 3 (height-3 tree).
    assert_eq!(proof.sibling_path.len(), 3);

    // Pad with an extra entry.
    proof.sibling_path.push([0x42u8; 32]);
    let err = verify_inclusion(&proof, &root, 0).unwrap_err();
    assert_eq!(err, ProofError::MalformedPath);

    // Truncate instead.
    let mut short = InclusionProof::build(&m, 3).unwrap();
    short.sibling_path.pop();
    let err = verify_inclusion(&short, &root, 0).unwrap_err();
    assert_eq!(err, ProofError::MalformedPath);
}

#[test]
fn adversarial_tampered_leaf_energy_rejected_via_root_mismatch() {
    // Pumping the energy in the proof changes the leaf hash ->
    // the reconstructed peak no longer bags to the real root.
    let m = build_8_leaf_mmr();
    let root = m.root().unwrap();
    let mut proof = InclusionProof::build(&m, 3).unwrap();

    proof.leaf.energy = 999_999_999;
    let err = verify_inclusion(&proof, &root, 0).unwrap_err();
    // Hash chain breaks -- RootMismatch, not EnergyBelowFloor.
    assert_eq!(err, ProofError::RootMismatch);
}

#[test]
fn adversarial_update_out_of_range_rejected() {
    let mut m = build_8_leaf_mmr();
    let err = m.update_energy(100, 0).unwrap_err();
    assert!(matches!(err, MmrError::LeafOutOfRange { idx: 100, .. }));
}

// -- Cross-cuts ---------------------------------------------------------------

#[test]
fn serde_round_trip_preserves_mmr() {
    let mut m = build_8_leaf_mmr();
    m.update_energy(2, 400).unwrap();
    let json = serde_json::to_string(&m).unwrap();
    let back: EpaMmr = serde_json::from_str(&json).unwrap();
    assert_eq!(m.root().unwrap(), back.root().unwrap());
    assert_eq!(back.get(2).unwrap().energy, 400);
}

#[test]
fn serde_round_trip_preserves_inclusion_proof() {
    let m = build_8_leaf_mmr();
    let proof = InclusionProof::build(&m, 4).unwrap();
    let json = serde_json::to_string(&proof).unwrap();
    let back: InclusionProof = serde_json::from_str(&json).unwrap();
    let root = m.root().unwrap();
    verify_inclusion(&back, &root, 0).unwrap();
}

#[test]
fn non_power_of_two_leaves_all_provable() {
    // 11 = 0b1011 -> peaks of sizes 8, 2, 1. All leaves provable.
    let mut m = EpaMmr::new();
    for i in 1..=11u8 {
        m.append(leaf(i, 2_000));
    }
    let root = m.root().unwrap();
    assert_eq!(m.peaks().len(), 3);

    for i in 0..11 {
        let proof = InclusionProof::build(&m, i).unwrap();
        verify_inclusion(&proof, &root, 1_000).unwrap();
    }
}

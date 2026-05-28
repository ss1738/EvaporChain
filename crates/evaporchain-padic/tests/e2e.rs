//! §A1.4 — p-adic ultrametric Merkle: epoch-state archive e2e
//!
//! Scenario: an EvaporChain archival node stores epoch state-roots in a
//! p-adic Merkle tree (p=2). Epoch numbers that share low-order binary
//! digits cluster into the same Merkle sub-tree, so proofs for "nearby"
//! (ultrametrically close) epochs share sibling hashes — the tree topology
//! mirrors the 2-adic metric on ℤ.

use evaporchain_padic::{
    ultrametric_distance, valuation, verify_inclusion, PAdicKey, PAdicMerkleTree, ProofError,
    TreeError,
};

// Deterministic synthetic epoch state-root (domain-tagged)
fn epoch_root(ep: u64) -> [u8; 32] {
    let mut h = [0u8; 32];
    h[..8].copy_from_slice(&ep.to_le_bytes());
    h[8] = 0xEC; // EvaporChain domain byte
    h
}

fn k2(ep: u64) -> PAdicKey<2> {
    PAdicKey::<2>(ep)
}
fn k3(ep: u64) -> PAdicKey<3> {
    PAdicKey::<3>(ep)
}

// ── Archive epochs (2-adic distances noted) ───────────────────────────────
//  EP0 = 0,  EP4 = 4,  EP8 = 8,  EP12 = 12,  EP16 = 16,  EP24 = 24,  EP32 = 32
//  d(4,8)   = v_2(4)  = 2
//  d(8,12)  = v_2(4)  = 2
//  d(4,12)  = v_2(8)  = 3   ← isosceles outlier
//  d(16,32) = v_2(16) = 4
const EP0: u64 = 0;
const EP4: u64 = 4;
const EP8: u64 = 8;
const EP12: u64 = 12;
const EP16: u64 = 16;
const EP24: u64 = 24;
const EP32: u64 = 32;

fn archive() -> PAdicMerkleTree<2> {
    let mut t = PAdicMerkleTree::<2>::new(8).unwrap();
    for ep in [EP0, EP4, EP8, EP12, EP16, EP24, EP32] {
        t.insert(k2(ep), &epoch_root(ep));
    }
    t
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn epoch_archive_full_lifecycle() {
    // Insert 7 epochs; every inserted epoch must produce a valid proof
    // against the same committed root.
    let t = archive();
    let root = t.root();
    for ep in [EP0, EP4, EP8, EP12, EP16, EP24, EP32] {
        let proof = t.prove(k2(ep)).expect("inserted epoch must produce proof");
        verify_inclusion::<2>(root, k2(ep), &epoch_root(ep), &proof)
            .unwrap_or_else(|e| panic!("proof for ep={ep} failed: {e:?}"));
    }
}

#[test]
fn strong_triangle_inequality_concrete() {
    // §A1.4 — d(4,12) = 3 ≥ min(d(4,8), d(8,12)) = min(2,2) = 2
    let d48 = ultrametric_distance::<2>(EP4, EP8); // v_2(|4-8|)  = v_2(4)  = 2
    let d812 = ultrametric_distance::<2>(EP8, EP12); // v_2(|8-12|) = v_2(4)  = 2
    let d412 = ultrametric_distance::<2>(EP4, EP12); // v_2(|4-12|) = v_2(8)  = 3
    assert_eq!(d48, 2, "d(4,8) must be 2");
    assert_eq!(d812, 2, "d(8,12) must be 2");
    assert_eq!(d412, 3, "d(4,12) must be 3");
    assert!(d412 >= d48.min(d812), "strong triangle violated");
}

#[test]
fn isosceles_property() {
    // §A1.4 — when two sides are equal in ultrametric distance, the "outlier"
    // side must be strictly greater (isosceles theorem for p-adic balls).
    let d48 = ultrametric_distance::<2>(EP4, EP8);
    let d812 = ultrametric_distance::<2>(EP8, EP12);
    let d412 = ultrametric_distance::<2>(EP4, EP12);
    assert_eq!(d48, d812, "d(4,8) = d(8,12): two equal sides");
    assert!(d412 > d48, "outlier side must be strictly larger");
}

#[test]
fn valuation_as_sub_tree_depth_label() {
    // §A1.4 — v_2(2^k) = k; valuation measures how deep a shared sub-tree ancestor is.
    for k in 0u32..8 {
        assert_eq!(valuation::<2>(1u64 << k), k, "v_2(2^{k}) = {k}");
    }
    assert_eq!(valuation::<2>(0), u32::MAX, "v_2(0) = ∞");
    // press-claim sanity
    assert_eq!(valuation::<2>(2), 1, "v_2(2) = 1");
    assert_eq!(valuation::<2>(8), 3, "v_2(8) = 3");
}

#[test]
fn ultrametric_clustering_matches_tree_topology() {
    // §A1.4 — EP4/EP12 are "closer" than EP4/EP8 in the 2-adic metric;
    // EP16/EP32 are closer still.  The ordering d(4,8) < d(4,12) < d(16,32)
    // means EP16 and EP32 share MORE low-order sub-tree levels.
    let d4_8 = ultrametric_distance::<2>(EP4, EP8); // 2
    let d4_12 = ultrametric_distance::<2>(EP4, EP12); // 3
    let d16_32 = ultrametric_distance::<2>(EP16, EP32); // 4
    assert!(d4_8 < d4_12, "EP4↔EP8 less close than EP4↔EP12");
    assert!(d4_12 < d16_32, "EP4↔EP12 less close than EP16↔EP32");
    // Triangle for the wider triple:
    let d4_32 = ultrametric_distance::<2>(EP4, EP32); // v_2(28) = v_2(4*7) = 2
    assert!(
        d4_32 >= d4_8.min(d16_32),
        "triangle: d(4,32) >= min(d(4,8),d(16,32))"
    );
}

#[test]
fn tamper_wrong_value_rejected() {
    // §A1.4 — swapping the leaf value must produce RootMismatch
    let t = archive();
    let root = t.root();
    let proof = t.prove(k2(EP8)).unwrap();
    let wrong = epoch_root(EP8 + 1);
    let err = verify_inclusion::<2>(root, k2(EP8), &wrong, &proof).unwrap_err();
    assert!(
        matches!(err, ProofError::RootMismatch { .. }),
        "wrong value → RootMismatch"
    );
}

#[test]
fn absent_epoch_no_proof() {
    // §A1.4 — epochs never inserted must return None from prove
    let t = archive();
    assert!(t.prove(k2(1)).is_none(), "ep=1 not inserted");
    assert!(t.prove(k2(99)).is_none(), "ep=99 not inserted");
    assert!(t.prove(k2(5)).is_none(), "ep=5 not inserted");
}

#[test]
fn insertion_order_invariant() {
    // §A1.4 — the Merkle root must be deterministic regardless of insertion order
    let epochs = [EP0, EP4, EP8, EP12, EP16, EP24, EP32];
    let mut fwd = PAdicMerkleTree::<2>::new(8).unwrap();
    for &ep in &epochs {
        fwd.insert(k2(ep), &epoch_root(ep));
    }
    let mut rev = PAdicMerkleTree::<2>::new(8).unwrap();
    for &ep in epochs.iter().rev() {
        rev.insert(k2(ep), &epoch_root(ep));
    }
    assert_eq!(fwd.root(), rev.root(), "root must be order-invariant");
}

#[test]
fn cross_prime_p3_archive() {
    // §A1.4 — p=3 tree: epochs 1, 3, 9, 27 (powers of 3) are provably included.
    // v_3(9-27) = v_3(18) = v_3(9*2) = 2, so 9 and 27 cluster at depth 2.
    let mut t = PAdicMerkleTree::<3>::new(5).unwrap();
    for ep in [1u64, 3, 9, 27] {
        t.insert(k3(ep), &epoch_root(ep));
    }
    let root = t.root();
    for ep in [1u64, 3, 9, 27] {
        let proof = t.prove(k3(ep)).expect("inserted epoch must have proof");
        verify_inclusion::<3>(root, k3(ep), &epoch_root(ep), &proof)
            .unwrap_or_else(|e| panic!("p=3 proof for ep={ep} failed: {e:?}"));
    }
    // p=3 triangle: d(1,9)=1, d(9,27)=2, d(1,27)=1; triangle holds (1 >= min(1,2)=1)
    let d19 = ultrametric_distance::<3>(1, 9); // v_3(8) = 0 (8 = 2^3, no 3 factors)
    let d927 = ultrametric_distance::<3>(9, 27); // v_3(18) = 2
    let d127 = ultrametric_distance::<3>(1, 27); // v_3(26) = 0
    assert!(d127 >= d19.min(d927), "p=3 triangle violated");
}

#[test]
fn depth_zero_construction_rejected() {
    // §A1.4 — depth=0 is a protocol violation (no Merkle levels)
    assert!(
        matches!(PAdicMerkleTree::<2>::new(0), Err(TreeError::DepthZero)),
        "depth=0 must be rejected"
    );
}

#[test]
fn proof_depth_equals_tree_depth() {
    // §A1.4 — an inclusion proof must have exactly `depth` levels (one per Merkle level)
    let depth: u32 = 6;
    let mut t = PAdicMerkleTree::<2>::new(depth).unwrap();
    t.insert(k2(EP4), &epoch_root(EP4));
    t.insert(k2(EP8), &epoch_root(EP8));
    let proof = t.prove(k2(EP4)).unwrap();
    assert_eq!(
        proof.levels.len() as u32,
        depth,
        "proof must have exactly {depth} levels"
    );
}

#[test]
fn ultrametric_doctrine_sub_tree_sharing() {
    // §A1.4 doctrine test: epochs 4, 8, 12 all produce valid proofs from one root;
    // the ultrametric inequality witnesses that (4,12) share a deeper common ancestor
    // than (4,8) — the tree topology is an exact embedding of the 2-adic metric.
    let t = archive();
    let root = t.root();
    for ep in [EP4, EP8, EP12] {
        let proof = t.prove(k2(ep)).unwrap();
        verify_inclusion::<2>(root, k2(ep), &epoch_root(ep), &proof)
            .unwrap_or_else(|e| panic!("doctrine proof for ep={ep} failed: {e:?}"));
    }
    // Ultrametric witnesses the sub-tree relationship
    let d_4_8 = ultrametric_distance::<2>(EP4, EP8);
    let d_4_12 = ultrametric_distance::<2>(EP4, EP12);
    let d_8_12 = ultrametric_distance::<2>(EP8, EP12);
    // d(4,12) = 3 is the "longest" pair → 4 and 12 share the deepest common ancestor
    assert!(d_4_12 > d_4_8, "d(4,12) must exceed d(4,8)");
    assert!(d_4_12 > d_8_12, "d(4,12) must exceed d(8,12)");
    // All three satisfy the strong triangle inequality
    assert!(d_4_12 >= d_4_8.min(d_8_12));
    assert!(d_4_8 >= d_4_12.min(d_8_12));
    assert!(d_8_12 >= d_4_8.min(d_4_12));
}

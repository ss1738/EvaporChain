//! Concurrency / partial-order relations on a `LightCone`.
//!
//! - `precedes(a, b)` — true iff `a ∈ causal_past(b)`.
//! - `comparable(a, b)` — true iff one precedes the other, OR a = b.
//! - `is_concurrent(a, b)` — true iff neither precedes the other AND
//!   they are distinct. The mempool's *antichain* operation in the
//!   sister crate `evaporchain-antichain-mempool` rests on this.

use crate::block::BlockId;
use crate::dag::{causal_past, LightCone};

/// `a` precedes `b` iff `a` is in `b`'s causal past.
pub fn precedes(lc: &LightCone, a: BlockId, b: BlockId) -> bool {
    if a == b {
        return false;
    }
    causal_past(lc, b).contains(&a)
}

/// `a` and `b` are comparable iff one precedes the other or they're
/// equal. This is reflexive/transitive on the partial order.
pub fn comparable(lc: &LightCone, a: BlockId, b: BlockId) -> bool {
    if a == b {
        return true;
    }
    precedes(lc, a, b) || precedes(lc, b, a)
}

/// `a` and `b` are concurrent iff neither precedes the other AND
/// they are distinct.
pub fn is_concurrent(lc: &LightCone, a: BlockId, b: BlockId) -> bool {
    if a == b {
        return false;
    }
    !precedes(lc, a, b) && !precedes(lc, b, a)
}

/// Phase 4.2 of `LIGHT_CONE_FULL_DAG_PLAN.md` — antichain test.
/// A set of `BlockId`s is an **antichain** iff every pair of
/// distinct elements is concurrent (neither precedes the other).
/// Empty sets and singletons are vacuously antichains.
///
/// Phase 4.2's antichain finality rule consumes this: a set finalizes
/// only when it's an antichain AND every block in the set has 2f+1
/// precommits. The full predicate lives in tendermint.rs; this
/// helper is the substrate primitive.
pub fn is_antichain(lc: &LightCone, set: &[BlockId]) -> bool {
    for i in 0..set.len() {
        for j in (i + 1)..set.len() {
            if comparable(lc, set[i], set[j]) {
                return false;
            }
        }
    }
    true
}

/// Phase 4.2 — minimal closing antichain at the current DAG state.
/// Returns the DAG's leaves: every leaf is concurrent with every
/// other leaf by definition (a leaf has no descendants in the DAG,
/// so no leaf can precede another). Sorted by `BTreeMap` order for
/// validator-determinism — every validator computes the same set
/// from the same DAG.
///
/// This is the **default** closing antichain. The full Phase 4.2
/// predicate may select a sub-antichain when only some leaves have
/// 2f+1 precommits; that selection logic lives in tendermint.rs.
pub fn closing_antichain(lc: &LightCone) -> Vec<BlockId> {
    lc.leaves().collect()
}

/// Phase 4.4 of `LIGHT_CONE_FULL_DAG_PLAN.md` — antichain commit-cert
/// digest. Deterministic 32-byte fingerprint of an antichain set,
/// used for cross-validator agreement on antichain finality.
///
/// **Contract**:
/// - **Deterministic**: same set of `BlockId`s → same digest on every
///   validator (sort before hashing — input vec order is irrelevant).
/// - **Domain-separated**: prefixed with `b"evaporchain-antichain-
///   digest-v1"` so this digest can never collide with any other
///   blake3-derived chain digest (block hash, state root, MEV digest,
///   etc.).
/// - **Empty-set sentinel**: `digest_antichain(&[])` returns the
///   blake3 of the domain tag alone — a stable well-known constant
///   that operators can recognise as "no finalized antichain yet".
/// - **Collision-resistant** (192-bit security from blake3-256
///   truncated to the first 32 bytes — full output here, no
///   truncation).
///
/// **Use site**: every validator runs this against its local
/// `closing_antichain(lc)` (or the Phase 4.2 sub-antichain selected
/// by the precommit-quorum predicate); cluster operators compare the
/// digests across validators (via `/api/light_cone/antichain_digest`)
/// to confirm they agree on finalized antichain membership without
/// having to ship the full block-id list around.
///
/// Pairs with `mev_state_digest` (Crooks-MEV Phase 3.2) — both are
/// the canonical inter-validator digests for their respective
/// substrate-level state.
pub fn digest_antichain(antichain: &[BlockId]) -> [u8; 32] {
    const DOMAIN_TAG: &[u8] = b"evaporchain-antichain-digest-v1";
    let mut sorted: Vec<BlockId> = antichain.to_vec();
    sorted.sort();
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN_TAG);
    for id in &sorted {
        hasher.update(id);
    }
    *hasher.finalize().as_bytes()
}

/// Phase 4.4 — convenience: digest of the current closing antichain
/// for a given light cone. Equivalent to
/// `digest_antichain(&closing_antichain(lc))` but expresses the
/// "default validator-deterministic finality digest" idiom in one
/// call.
pub fn closing_antichain_digest(lc: &LightCone) -> [u8; 32] {
    digest_antichain(&closing_antichain(lc))
}

/// First-divergence point returned by `find_first_divergence`. The
/// `local_*` / `remote_*` naming is a convenience for the typical
/// caller pattern (operator's own validator vs a peer); the function
/// itself is symmetric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DivergencePoint {
    pub height: u64,
    pub local_digest: [u8; 32],
    pub remote_digest: [u8; 32],
}

/// Phase 4.4 — first-divergence point between two antichain-digest
/// histories. Each history is a slice of `(block_height, digest)`
/// pairs (typically from
/// `TendermintConsensus::antichain_digest_history()` on two
/// validators). Returns the lowest `block_height` at which both
/// histories have entries AND the digests differ; `None` if no
/// divergence is found in the overlap.
///
/// **Why this primitive exists.** Without it, operators
/// cross-comparing cluster validators have to write per-height diff
/// logic in jq/Python against `/api/light_cone/antichain_digest_history`
/// responses. With it, the chain ships the comparison primitive
/// directly: this function is the canonical substrate for
/// cross-validator antichain-finality disagreement detection.
///
/// **Semantics:**
/// - Histories need not be the same length or cover the same height
///   range — function operates on the OVERLAP (heights present in
///   both).
/// - Histories may have heights in any order; function does not
///   assume sorted input. Internally indexes by height.
/// - If a height appears multiple times in one history, the LAST
///   occurrence wins (consistent with FIFO eviction semantics — the
///   most-recent observation at that height).
/// - Returns `None` when no overlap exists OR when all overlapping
///   heights agree.
pub fn find_first_divergence(
    a: &[(u64, [u8; 32])],
    b: &[(u64, [u8; 32])],
) -> Option<DivergencePoint> {
    use std::collections::BTreeMap;
    let a_by_height: BTreeMap<u64, [u8; 32]> = a.iter().copied().collect();
    let b_by_height: BTreeMap<u64, [u8; 32]> = b.iter().copied().collect();
    // BTreeMap iteration is height-ascending; first divergent overlap
    // is the lowest divergent height by construction.
    for (&height, &a_digest) in &a_by_height {
        if let Some(&b_digest) = b_by_height.get(&height) {
            if a_digest != b_digest {
                return Some(DivergencePoint {
                    height,
                    local_digest: a_digest,
                    remote_digest: b_digest,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;

    fn id(b: u8) -> BlockId {
        [b; 32]
    }

    fn diamond() -> LightCone {
        // A → B, A → C, B → D, C → D
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 800, 2))
            .unwrap();
        lc
    }

    #[test]
    fn precedes_examples() {
        let lc = diamond();
        assert!(precedes(&lc, id(0), id(3)));
        assert!(precedes(&lc, id(1), id(3)));
        assert!(!precedes(&lc, id(1), id(2))); // concurrent, neither precedes
        assert!(!precedes(&lc, id(0), id(0))); // not reflexive
    }

    #[test]
    fn concurrent_pair_in_diamond() {
        let lc = diamond();
        assert!(is_concurrent(&lc, id(1), id(2)));
        assert!(is_concurrent(&lc, id(2), id(1))); // symmetric
        assert!(!is_concurrent(&lc, id(0), id(1))); // ordered
        assert!(!is_concurrent(&lc, id(1), id(1))); // not reflexive
    }

    #[test]
    fn comparable_is_reflexive() {
        let lc = diamond();
        for b in [0, 1, 2, 3] {
            assert!(comparable(&lc, id(b), id(b)));
        }
    }

    #[test]
    fn comparable_via_precedes() {
        let lc = diamond();
        assert!(comparable(&lc, id(0), id(3)));
        assert!(comparable(&lc, id(3), id(0))); // either direction
        assert!(!comparable(&lc, id(1), id(2))); // concurrent
    }

    /// Phase 4.2 — empty + singleton sets are vacuously antichains.
    #[test]
    fn is_antichain_empty_and_singleton() {
        let lc = diamond();
        assert!(is_antichain(&lc, &[]));
        assert!(is_antichain(&lc, &[id(0)]));
        assert!(is_antichain(&lc, &[id(3)]));
    }

    /// Phase 4.2 — concurrent pair forms an antichain.
    #[test]
    fn is_antichain_concurrent_pair() {
        let lc = diamond();
        assert!(is_antichain(&lc, &[id(1), id(2)]));
        assert!(is_antichain(&lc, &[id(2), id(1)])); // order-independent
    }

    /// Phase 4.2 — ordered pair (parent + descendant) is NOT an
    /// antichain. Locks the soundness contract.
    #[test]
    fn is_antichain_rejects_comparable_pair() {
        let lc = diamond();
        assert!(!is_antichain(&lc, &[id(0), id(3)]));
        assert!(!is_antichain(&lc, &[id(0), id(1)]));
        assert!(!is_antichain(&lc, &[id(1), id(3)]));
    }

    /// Phase 4.2 — three concurrent blocks form an antichain
    /// (transitivity of concurrency at the antichain level).
    #[test]
    fn is_antichain_three_concurrent_blocks() {
        // A → B, A → C, A → D (three siblings of A — pairwise concurrent).
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(0)], 900, 1)).unwrap();
        assert!(is_antichain(&lc, &[id(1), id(2), id(3)]));
    }

    /// Phase 4.2 — `closing_antichain` returns the DAG's leaves
    /// (which are pairwise concurrent by definition).
    #[test]
    fn closing_antichain_in_diamond() {
        let lc = diamond();
        let ac = closing_antichain(&lc);
        // Diamond's leaves: just D (id(3)) since A->B->D and A->C->D.
        assert_eq!(ac, vec![id(3)]);
        assert!(is_antichain(&lc, &ac));
    }

    /// Phase 4.2 robustness — invariant: for any randomly-generated
    /// DAG, `closing_antichain` is always actually an antichain.
    /// Proptest sweeps random DAG shapes (linear chains + tree-like
    /// branching) and verifies the postcondition.
    proptest::proptest! {
        #[test]
        fn closing_antichain_is_always_an_antichain(
            // Sweep DAG sizes 1..=20 with random branching factors.
            // Each block (after genesis) picks 1..=2 random parents
            // from the existing DAG.
            seed in 0u64..1000,
            n_blocks in 1usize..=20,
        ) {
            use proptest::prop_assert;

            // Deterministic synthetic DAG generator from (seed, n_blocks).
            // Block i's parents = pick from { 0..i } based on seed-derived hash.
            let mut lc = LightCone::new();
            // Genesis (block 0) — no parents.
            lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
            for i in 1..n_blocks {
                // Number of parents: 1 or 2 based on hash bit.
                let h = (seed.wrapping_mul(i as u64).wrapping_add(31)).wrapping_mul(2654435761);
                let two_parents = (h & 1) == 1 && i >= 2;
                let mut parents = Vec::new();
                let p1 = (h.wrapping_div(7) as usize) % i;
                parents.push(id(p1 as u8));
                if two_parents {
                    let p2 = (h.wrapping_div(11) as usize) % i;
                    if p2 != p1 {
                        parents.push(id(p2 as u8));
                    }
                }
                // Inserting a block whose parents include itself is
                // impossible by construction (p1, p2 < i = block_id).
                let _ = lc.insert(Block::new(id(i as u8), parents, 100, i as u64));
            }

            // Postcondition: closing_antichain is always an antichain.
            let ac = closing_antichain(&lc);
            prop_assert!(
                is_antichain(&lc, &ac),
                "closing_antichain returned a non-antichain set: {:?}",
                ac
            );

            // Additional invariant: every leaf appears in the
            // closing antichain (the antichain IS the leaves).
            let mut sorted_leaves: Vec<BlockId> = lc.leaves().collect();
            sorted_leaves.sort();
            let mut sorted_ac = ac.clone();
            sorted_ac.sort();
            prop_assert!(
                sorted_leaves == sorted_ac,
                "closing_antichain ≠ leaves: ac={:?}, leaves={:?}",
                sorted_ac,
                sorted_leaves
            );
        }
    }

    /// Phase 4.2 — `closing_antichain` on a 3-sibling DAG returns
    /// all three siblings (they're the leaves and pairwise
    /// concurrent).
    #[test]
    fn closing_antichain_three_siblings() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(0)], 900, 1)).unwrap();
        let ac = closing_antichain(&lc);
        // Genesis (id(0)) has 3 children → not a leaf. Three siblings
        // are all leaves.
        let mut sorted = ac.clone();
        sorted.sort();
        assert_eq!(sorted, vec![id(1), id(2), id(3)]);
        assert!(is_antichain(&lc, &ac));
    }

    // ── Phase 4.4 — antichain commit-cert digest ───────────────────

    /// Phase 4.4 — digest is order-independent. Sorting before
    /// hashing makes the input vec's permutation irrelevant. Locks
    /// the cross-validator determinism contract: validator A's
    /// `[id(2), id(1), id(3)]` and validator B's `[id(1), id(2),
    /// id(3)]` MUST produce the same digest.
    #[test]
    fn digest_antichain_is_order_independent() {
        let a = digest_antichain(&[id(1), id(2), id(3)]);
        let b = digest_antichain(&[id(3), id(1), id(2)]);
        let c = digest_antichain(&[id(2), id(3), id(1)]);
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    /// Phase 4.4 — different sets produce different digests.
    /// Collision-resistance contract: the digest separates antichain
    /// membership at the cluster level. Without this, two validators
    /// could agree on a digest while disagreeing on the actual
    /// finalized antichain — the very condition this primitive
    /// is designed to detect.
    #[test]
    fn digest_antichain_separates_distinct_sets() {
        let a = digest_antichain(&[id(1), id(2)]);
        let b = digest_antichain(&[id(1), id(3)]);
        let c = digest_antichain(&[id(1), id(2), id(3)]);
        assert_ne!(a, b, "{{1,2}} and {{1,3}} must hash differently");
        assert_ne!(a, c, "{{1,2}} and {{1,2,3}} must hash differently");
        assert_ne!(b, c, "{{1,3}} and {{1,2,3}} must hash differently");
    }

    /// Phase 4.4 — empty antichain hashes to the domain-tag-only
    /// digest. Stable well-known constant — operators can recognise
    /// this digest as "no finalized antichain yet" without having to
    /// look up which sub-antichain was selected.
    #[test]
    fn digest_antichain_empty_set_is_domain_tag_only() {
        let d = digest_antichain(&[]);
        // Recompute the expected sentinel manually.
        let mut h = blake3::Hasher::new();
        h.update(b"evaporchain-antichain-digest-v1");
        let expected = *h.finalize().as_bytes();
        assert_eq!(d, expected);
    }

    /// Phase 4.4 — domain separation: feeding the antichain bytes to
    /// blake3 directly (without the domain tag) MUST produce a
    /// different digest. Locks the contract that this primitive
    /// cannot collide with raw blake3-of-block-id-bytes used
    /// elsewhere on the chain.
    #[test]
    fn digest_antichain_is_domain_separated() {
        let antichain = [id(1), id(2)];
        let with_tag = digest_antichain(&antichain);
        // Hash without the tag.
        let mut h = blake3::Hasher::new();
        for i in &antichain {
            h.update(i);
        }
        let without_tag = *h.finalize().as_bytes();
        assert_ne!(with_tag, without_tag);
    }

    /// Phase 4.4 — `closing_antichain_digest` is the convenience
    /// composition. Validates that the wired idiom matches the
    /// step-by-step result: same DAG → same digest.
    #[test]
    fn closing_antichain_digest_matches_step_by_step() {
        let lc = diamond();
        let stepped = digest_antichain(&closing_antichain(&lc));
        let composed = closing_antichain_digest(&lc);
        assert_eq!(stepped, composed);
    }

    /// Phase 4.4 — two validators with the same DAG produce the same
    /// digest; two with diverging DAGs (an extra block in one)
    /// produce different digests. This is the CORE inter-validator
    /// agreement check this primitive is built for.
    #[test]
    fn closing_antichain_digest_separates_diverging_dags() {
        // Validator A — diamond.
        let lc_a = diamond();
        // Validator B — diamond + an extra leaf on top of D (id(3)).
        let mut lc_b = diamond();
        lc_b.insert(Block::new(id(4), vec![id(3)], 700, 3)).unwrap();
        let digest_a = closing_antichain_digest(&lc_a);
        let digest_b = closing_antichain_digest(&lc_b);
        assert_ne!(
            digest_a, digest_b,
            "diverging DAGs must produce diverging digests"
        );

        // Two independent diamonds (constructed identically) → same
        // digest, regardless of insertion order.
        let lc_a2 = diamond();
        assert_eq!(
            closing_antichain_digest(&lc_a),
            closing_antichain_digest(&lc_a2),
            "identical DAG state → identical digest"
        );
    }

    // ── Phase 4.4 — find_first_divergence ───────────────────────────

    fn d(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    /// Phase 4.4 — identical histories produce no divergence.
    #[test]
    fn find_first_divergence_identical_returns_none() {
        let h = vec![(1u64, d(1)), (2, d(2)), (3, d(3))];
        assert!(find_first_divergence(&h, &h).is_none());
    }

    /// Phase 4.4 — empty inputs produce no divergence (no overlap).
    #[test]
    fn find_first_divergence_empty_returns_none() {
        assert!(find_first_divergence(&[], &[]).is_none());
        let h = vec![(1u64, d(1))];
        assert!(find_first_divergence(&h, &[]).is_none());
        assert!(find_first_divergence(&[], &h).is_none());
    }

    /// Phase 4.4 — disjoint heights (no overlap) produce no
    /// divergence even though the digests differ at every height
    /// each history covers. Operators with non-overlapping windows
    /// (one validator caught up later than the other) get a clean
    /// "no comparable data" signal instead of a false divergence.
    #[test]
    fn find_first_divergence_disjoint_heights_returns_none() {
        let a = vec![(1u64, d(1)), (2, d(2))];
        let b = vec![(10u64, d(10)), (11, d(11))];
        assert!(find_first_divergence(&a, &b).is_none());
    }

    /// Phase 4.4 — the lowest overlapping divergent height is
    /// returned, NOT the lowest height in either input alone. Locks
    /// the contract for operators triaging cluster splits: the
    /// reported height is the *first point of disagreement*, not
    /// the first point at which both validators have data.
    #[test]
    fn find_first_divergence_returns_lowest_divergent_overlap() {
        // Heights 5 and 6 overlap; agree at 5, disagree at 6.
        // Heights 7 and 8 also disagree but height 6 must win.
        let a = vec![(5u64, d(50)), (6, d(60)), (7, d(70)), (8, d(80))];
        let b = vec![(5, d(50)), (6, d(99)), (7, d(70)), (8, d(88))];
        let divergence = find_first_divergence(&a, &b).expect("divergence present");
        assert_eq!(divergence.height, 6);
        assert_eq!(divergence.local_digest, d(60));
        assert_eq!(divergence.remote_digest, d(99));
    }

    /// Phase 4.4 — symmetric over input order. Swapping `a` and `b`
    /// flips `local_digest`/`remote_digest` but keeps the same
    /// height. Operators calling `find_first_divergence(local, peer)`
    /// vs `find_first_divergence(peer, local)` get consistent height
    /// reports.
    #[test]
    fn find_first_divergence_height_is_symmetric() {
        let a = vec![(10u64, d(1)), (11, d(2))];
        let b = vec![(10, d(1)), (11, d(99))];
        let ab = find_first_divergence(&a, &b).expect("divergence");
        let ba = find_first_divergence(&b, &a).expect("divergence");
        assert_eq!(ab.height, ba.height);
        assert_eq!(ab.local_digest, ba.remote_digest);
        assert_eq!(ab.remote_digest, ba.local_digest);
    }

    /// Phase 4.4 — sparse / out-of-order input is handled. The
    /// function does not assume input is sorted; it indexes by
    /// height internally.
    #[test]
    fn find_first_divergence_handles_unsorted_input() {
        // Same data, different order in `a` and `b`.
        let a = vec![(3u64, d(30)), (1, d(10)), (2, d(20))];
        let b = vec![(2u64, d(20)), (3, d(99)), (1, d(10))];
        let divergence = find_first_divergence(&a, &b).expect("divergence");
        assert_eq!(divergence.height, 3);
    }

    /// Phase 4.4 — duplicate-height inputs use the last occurrence
    /// (consistent with FIFO-eviction semantics: the most-recent
    /// observation at a given height wins).
    #[test]
    fn find_first_divergence_duplicate_height_uses_last() {
        let a = vec![(1u64, d(1)), (1, d(2))]; // last wins → d(2)
        let b = vec![(1u64, d(2))];
        // After applying "last wins", a[1] → d(2) matches b → no
        // divergence.
        assert!(find_first_divergence(&a, &b).is_none());

        // And the other way: explicit divergence.
        let c = vec![(1u64, d(1)), (1, d(99))]; // last wins → d(99)
        let d_hist = vec![(1u64, d(1))];
        let div = find_first_divergence(&c, &d_hist).expect("divergence");
        assert_eq!(div.height, 1);
        assert_eq!(div.local_digest, d(99));
        assert_eq!(div.remote_digest, d(1));
    }
}

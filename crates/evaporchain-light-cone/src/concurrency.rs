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
}

//! Light-Cone V2 — causal-cone Merkle proofs.
//!
//! V1 (`evaporchain-light-cone`) ships `causal_past(lc, block_id)`
//! as a reachability set computed by BFS over parent edges. Correct
//! math, but every verifier needs the full DAG to evaluate it.
//!
//! V2 closes that gap with a Merkle commitment over the sorted
//! ancestor list:
//!
//! 1. `causal_root(lc, block_id)` — BLAKE3 Merkle root over the
//!    canonical (BTreeSet-sorted) `causal_past(lc, block_id)`.
//!    Empty causal past → sentinel "empty cone" root. Domain-separated
//!    leaf and inner tags.
//!
//! 2. `prove_ancestry(lc, descendant, ancestor)` — produces a
//!    `MerklePath` from `ancestor`'s leaf to `causal_root(descendant)`,
//!    or `None` if `ancestor` is not in the causal past.
//!
//! 3. `verify_ancestry(causal_root, ancestor_id, proof)` — pure
//!    function of (32-byte root, 32-byte candidate id, proof bytes).
//!    Verifier never touches the DAG.
//!
//! ## Holographic reduction
//!
//! V2 is the substrate for **light-client ancestry queries**: a
//! light client holding only the latest block's `causal_root` can
//! verify any ancestry claim a full node sends in O(log N) hashes.
//! Same shape as the Merkle-Patricia state proofs Ethereum uses for
//! account inclusion, but over the partial-order DAG instead of a
//! contiguous trie.

pub mod merkle;
pub mod proof;

pub use merkle::{causal_root, EMPTY_CAUSAL_ROOT};
pub use proof::{prove_ancestry, verify_ancestry, AncestryError, MerklePath};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_light_cone::{Block, BlockId, LightCone};

    fn id(b: u8) -> BlockId {
        let mut x = [0u8; 32];
        x[31] = b;
        x
    }

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Light-Cone V2 lets a light client holding only
    /// a 32-byte `causal_root` verify any ancestry claim in O(log N)
    /// hashes — no DAG access. Non-ancestors yield no proof, and
    /// flipping the candidate id makes the verifier reject."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Linear chain: g ← b1 ← b2 ← b3.
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1_000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 1_000, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(1)], 1_000, 2)).unwrap();
        lc.insert(Block::new(id(3), vec![id(2)], 1_000, 3)).unwrap();

        let root = causal_root(&lc, id(3));
        let proof = prove_ancestry(&lc, id(3), id(1)).unwrap();

        // Verifier holds only (root, ancestor_id, proof). No DAG.
        assert!(verify_ancestry(&root, &id(1), &proof).unwrap());

        // Flipping the candidate id breaks the proof.
        assert!(!verify_ancestry(&root, &id(2), &proof).unwrap());

        // Non-ancestor: id(3) is not an ancestor of id(1).
        let res = prove_ancestry(&lc, id(1), id(3));
        assert!(matches!(res, Err(AncestryError::NotAnAncestor { .. })));

        // Genesis has empty causal past — its root is the empty
        // sentinel, distinct from any non-empty causal-past root.
        let genesis_root = causal_root(&lc, id(0));
        assert_ne!(genesis_root, root, "genesis vs non-genesis roots must differ");

        // Empty-sentinel root is reproducible for any genesis-like
        // block (a fresh DAG with a single block has empty causal past).
        let mut lc2 = LightCone::new();
        lc2.insert(Block::new(id(7), vec![], 1_000, 0)).unwrap();
        assert_eq!(causal_root(&lc2, id(7)), genesis_root);
    }
}

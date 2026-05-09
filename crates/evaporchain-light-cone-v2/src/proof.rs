//! Ancestry proof: prover + verifier.

use evaporchain_light_cone::{causal_past, BlockId, LightCone};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::merkle::{empty_root, inner_hash, leaf_hash};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerklePath {
    /// Ordered list of sibling hashes from leaf up to root.
    pub siblings: Vec<[u8; 32]>,
    /// Per-level direction: `false` = leaf is left (sibling is right);
    /// `true` = leaf is right (sibling is left). Length matches
    /// `siblings`.
    pub directions: Vec<bool>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AncestryError {
    #[error("descendant block {0:?} not in DAG")]
    DescendantNotFound(BlockId),
    #[error("ancestor {ancestor:?} not in causal past of descendant {descendant:?}")]
    NotAnAncestor {
        descendant: BlockId,
        ancestor: BlockId,
    },
    #[error(
        "Merkle path length mismatch: siblings has {siblings} entries, directions has {directions}"
    )]
    PathShapeMismatch { siblings: usize, directions: usize },
}

/// Build a Merkle path proving that `ancestor ∈ causal_past(descendant)`.
/// Returns `Err(NotAnAncestor)` if the ancestor relation does not hold.
pub fn prove_ancestry(
    lc: &LightCone,
    descendant: BlockId,
    ancestor: BlockId,
) -> Result<MerklePath, AncestryError> {
    if !lc.contains(&descendant) {
        return Err(AncestryError::DescendantNotFound(descendant));
    }
    let past = causal_past(lc, descendant);
    if !past.contains(&ancestor) {
        return Err(AncestryError::NotAnAncestor {
            descendant,
            ancestor,
        });
    }

    // Reproduce the leaf-hash list in canonical (BTreeSet-sorted)
    // order, then walk the tree.
    let leaves: Vec<[u8; 32]> = past.iter().map(leaf_hash).collect();
    let target_idx = past
        .iter()
        .position(|id| id == &ancestor)
        .expect("ancestor was just confirmed in past");

    let mut layer = leaves;
    let mut idx = target_idx;
    let mut siblings: Vec<[u8; 32]> = Vec::new();
    let mut directions: Vec<bool> = Vec::new();

    while layer.len() > 1 {
        let pair_start = idx & !1usize; // even-aligned
        let is_right = idx & 1 == 1;
        let sibling = if is_right {
            layer[pair_start]
        } else if pair_start + 1 < layer.len() {
            layer[pair_start + 1]
        } else {
            // odd layer: leaf is duplicated, sibling = self
            layer[idx]
        };
        siblings.push(sibling);
        directions.push(is_right);

        // Build next layer.
        let mut next: Vec<[u8; 32]> = Vec::with_capacity(layer.len().div_ceil(2));
        let mut i = 0;
        while i < layer.len() {
            let l = layer[i];
            let r = if i + 1 < layer.len() {
                layer[i + 1]
            } else {
                layer[i]
            };
            next.push(inner_hash(&l, &r));
            i += 2;
        }
        layer = next;
        idx /= 2;
    }

    Ok(MerklePath {
        siblings,
        directions,
    })
}

/// Verify a Merkle path. Returns `true` iff applying `proof` to
/// `leaf_hash(ancestor_id)` reproduces `expected_root`.
///
/// Pure function — does not touch any DAG. The verifier holds only
/// `(expected_root, ancestor_id, proof)`.
pub fn verify_ancestry(
    expected_root: &[u8; 32],
    ancestor_id: &BlockId,
    proof: &MerklePath,
) -> Result<bool, AncestryError> {
    if proof.siblings.len() != proof.directions.len() {
        return Err(AncestryError::PathShapeMismatch {
            siblings: proof.siblings.len(),
            directions: proof.directions.len(),
        });
    }

    // Empty proof: ancestor *is* the root (single-leaf tree). Confirm
    // by hashing the leaf and comparing.
    if proof.siblings.is_empty() {
        return Ok(&leaf_hash(ancestor_id) == expected_root);
    }

    // Special "empty cone" sanity: if expected_root is the empty
    // sentinel, no ancestry can possibly verify. Reject.
    if expected_root == &empty_root() {
        return Ok(false);
    }

    let mut current = leaf_hash(ancestor_id);
    for (sibling, &is_right) in proof.siblings.iter().zip(proof.directions.iter()) {
        current = if is_right {
            inner_hash(sibling, &current)
        } else {
            inner_hash(&current, sibling)
        };
    }
    Ok(&current == expected_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merkle::causal_root;
    use evaporchain_light_cone::Block;

    fn id(b: u8) -> BlockId {
        let mut x = [0u8; 32];
        x[31] = b;
        x
    }

    fn build_chain() -> LightCone {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 1000, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(1)], 1000, 2)).unwrap();
        lc.insert(Block::new(id(3), vec![id(2)], 1000, 3)).unwrap();
        lc.insert(Block::new(id(4), vec![id(3)], 1000, 4)).unwrap();
        lc
    }

    fn build_dag() -> LightCone {
        // Diamond DAG: 0 ← {1, 2} ← 3 (3 has parents 1 and 2).
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 1000, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 1000, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 1000, 2))
            .unwrap();
        lc
    }

    // ── prover error paths ───────────────────────────────────────

    #[test]
    fn prove_rejects_missing_descendant() {
        let lc = build_chain();
        let err = prove_ancestry(&lc, id(99), id(0)).unwrap_err();
        assert_eq!(err, AncestryError::DescendantNotFound(id(99)));
    }

    #[test]
    fn prove_rejects_non_ancestor() {
        let lc = build_chain();
        // id(0) is genesis; id(2) is descendant. id(2) is NOT in
        // causal past of id(0).
        let err = prove_ancestry(&lc, id(0), id(2)).unwrap_err();
        assert!(matches!(err, AncestryError::NotAnAncestor { .. }));
    }

    #[test]
    fn prove_rejects_self_ancestry() {
        // A block is not in its own causal past.
        let lc = build_chain();
        let err = prove_ancestry(&lc, id(2), id(2)).unwrap_err();
        assert!(matches!(err, AncestryError::NotAnAncestor { .. }));
    }

    // ── verifier happy path ──────────────────────────────────────

    #[test]
    fn round_trip_proves_ancestor_in_chain() {
        let lc = build_chain();
        let root = causal_root(&lc, id(4));
        let proof = prove_ancestry(&lc, id(4), id(2)).unwrap();
        assert!(verify_ancestry(&root, &id(2), &proof).unwrap());
    }

    #[test]
    fn round_trip_proves_genesis_via_long_chain() {
        let lc = build_chain();
        let root = causal_root(&lc, id(4));
        let proof = prove_ancestry(&lc, id(4), id(0)).unwrap();
        assert!(verify_ancestry(&root, &id(0), &proof).unwrap());
    }

    #[test]
    fn round_trip_in_diamond_dag() {
        let lc = build_dag();
        let root = causal_root(&lc, id(3));
        // id(3)'s causal past = {0, 1, 2}.
        for ancestor in [id(0), id(1), id(2)] {
            let proof = prove_ancestry(&lc, id(3), ancestor).unwrap();
            assert!(
                verify_ancestry(&root, &ancestor, &proof).unwrap(),
                "ancestor = {:?}",
                ancestor
            );
        }
    }

    // ── verifier rejection paths ─────────────────────────────────

    #[test]
    fn verify_rejects_wrong_ancestor() {
        let lc = build_chain();
        let root = causal_root(&lc, id(4));
        let proof = prove_ancestry(&lc, id(4), id(2)).unwrap();
        // Reuse proof but claim ancestor is id(99) (not in DAG, but
        // the verifier doesn't know that — it just hashes).
        assert!(!verify_ancestry(&root, &id(99), &proof).unwrap());
    }

    #[test]
    fn verify_rejects_tampered_sibling() {
        let lc = build_chain();
        let root = causal_root(&lc, id(4));
        let mut proof = prove_ancestry(&lc, id(4), id(2)).unwrap();
        if !proof.siblings.is_empty() {
            proof.siblings[0][0] ^= 0xff;
        }
        assert!(!verify_ancestry(&root, &id(2), &proof).unwrap());
    }

    #[test]
    fn verify_rejects_tampered_direction() {
        let lc = build_chain();
        let root = causal_root(&lc, id(4));
        let mut proof = prove_ancestry(&lc, id(4), id(2)).unwrap();
        if !proof.directions.is_empty() {
            proof.directions[0] = !proof.directions[0];
        }
        // Flipping a direction will (almost surely) produce a wrong
        // root, so verification rejects.
        assert!(!verify_ancestry(&root, &id(2), &proof).unwrap());
    }

    #[test]
    fn verify_rejects_wrong_root() {
        let lc = build_chain();
        let proof = prove_ancestry(&lc, id(4), id(2)).unwrap();
        let wrong_root = [0xffu8; 32];
        assert!(!verify_ancestry(&wrong_root, &id(2), &proof).unwrap());
    }

    #[test]
    fn verify_rejects_path_shape_mismatch() {
        let proof = MerklePath {
            siblings: vec![[0u8; 32], [1u8; 32]],
            directions: vec![false],
        };
        let err = verify_ancestry(&[0u8; 32], &id(0), &proof).unwrap_err();
        assert!(matches!(err, AncestryError::PathShapeMismatch { .. }));
    }

    #[test]
    fn verify_against_empty_root_always_rejects() {
        let proof = MerklePath {
            siblings: vec![[0u8; 32]],
            directions: vec![false],
        };
        let empty = crate::merkle::empty_root();
        assert!(!verify_ancestry(&empty, &id(0), &proof).unwrap());
    }

    // ── press claim ──────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Light-Cone V2 ships causal-cone Merkle proofs.
        // Each block has a causal_root over the BTreeSet-sorted ids
        // of its ancestors. A descendant produces an O(log N) proof
        // that an ancestor is in its causal past; the verifier needs
        // only (causal_root, ancestor_id, proof) — never the DAG
        // itself. Domain-separated leaf and inner hashes prevent
        // second-preimage confusion. Tampering any field of the
        // proof causes verification to reject."

        let lc = build_chain();
        let root4 = causal_root(&lc, id(4));

        // Round-trip every ancestor of block 4.
        for ancestor in [id(0), id(1), id(2), id(3)] {
            let proof = prove_ancestry(&lc, id(4), ancestor).unwrap();
            assert!(verify_ancestry(&root4, &ancestor, &proof).unwrap());
        }

        // Non-ancestor → no proof.
        assert!(prove_ancestry(&lc, id(2), id(4)).is_err());

        // Tampering rejects.
        let mut proof = prove_ancestry(&lc, id(4), id(0)).unwrap();
        proof.siblings[0][0] ^= 1;
        assert!(!verify_ancestry(&root4, &id(0), &proof).unwrap());
    }
}

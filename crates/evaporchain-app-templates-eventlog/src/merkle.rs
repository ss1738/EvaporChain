//! Merkle root over the canonical bytes of a deploy-event log.
//!
//! Light clients verify that a particular receipt is in the log
//! without holding the full log: they fetch the receipt, the
//! Merkle root (block-header summary), and a path of sibling
//! hashes. They recompute the root and compare.
//!
//! Three structural decisions:
//!
//! 1. **Hash the receipt's `canonical_bytes()`, not its
//!    `event_id()`.** A leaf hash that's "the receipt's own
//!    event_id hashed again" gives nothing — the event_id is
//!    already a BLAKE3. We instead use a domain-separated leaf
//!    encoding so leaves can never be confused with internal
//!    nodes.
//!
//! 2. **Domain-separated leaf vs node tags.** A second-pre-image
//!    attack would craft a receipt whose canonical bytes equal an
//!    internal node hash; the leaf tag prevents this.
//!
//! 3. **Empty log returns the all-zero hash.** A sentinel for
//!    "nothing to prove yet"; light clients short-circuit.

use evaporchain_app_templates_receipt::DeployReceipt;

const LEAF_TAG: &[u8] = b"evaporchain:eventlog:leaf:v1\0";
const NODE_TAG: &[u8] = b"evaporchain:eventlog:node:v1\0";

/// Compute the Merkle root over the receipts. Empty input returns
/// `[0; 32]` as the "no receipts yet" sentinel.
pub fn merkle_root(receipts: &[DeployReceipt]) -> [u8; 32] {
    if receipts.is_empty() {
        return [0; 32];
    }
    let mut layer: Vec<[u8; 32]> = receipts.iter().map(leaf_hash).collect();
    while layer.len() > 1 {
        layer = layer
            .chunks(2)
            .map(|pair| match pair {
                [a, b] => node_hash(a, b),
                // Odd tail: duplicate the last leaf — standard Merkle
                // tree convention.
                [a] => node_hash(a, a),
                _ => unreachable!(),
            })
            .collect();
    }
    layer[0]
}

/// Verify that `receipt` is at index `idx` in a tree whose root is
/// `root`, given the sibling-hash `path` (leaf-to-root order).
pub fn verify_inclusion(
    receipt: &DeployReceipt,
    idx: usize,
    path: &[[u8; 32]],
    root: &[u8; 32],
) -> bool {
    let mut current = leaf_hash(receipt);
    let mut i = idx;
    for sibling in path {
        current = if i & 1 == 0 {
            node_hash(&current, sibling)
        } else {
            node_hash(sibling, &current)
        };
        i >>= 1;
    }
    &current == root
}

fn leaf_hash(receipt: &DeployReceipt) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(LEAF_TAG);
    h.update(&receipt.canonical_bytes());
    *h.finalize().as_bytes()
}

fn node_hash(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(NODE_TAG);
    h.update(a);
    h.update(b);
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_app_templates::class::{MAYFLY, SINGH_SABI};

    fn receipt(commit_byte: u8, height: u64, deployer_byte: u8) -> DeployReceipt {
        DeployReceipt::new(
            [commit_byte; 32],
            [0x22; 32],
            SINGH_SABI,
            [deployer_byte; 32],
            1_400,
            500,
            height,
        )
        .unwrap()
    }

    #[test]
    fn empty_log_root_is_zero_sentinel() {
        assert_eq!(merkle_root(&[]), [0; 32]);
    }

    #[test]
    fn single_receipt_root_is_its_leaf_hash() {
        let r = receipt(1, 100, 1);
        assert_eq!(merkle_root(std::slice::from_ref(&r)), leaf_hash(&r));
    }

    #[test]
    fn root_changes_when_receipt_changes() {
        let a = receipt(1, 100, 1);
        let b = receipt(2, 100, 1);
        assert_ne!(merkle_root(&[a]), merkle_root(&[b]));
    }

    #[test]
    fn root_is_deterministic() {
        let receipts: Vec<DeployReceipt> =
            (0..5).map(|i| receipt(i, 100 + i as u64, i + 1)).collect();
        assert_eq!(merkle_root(&receipts), merkle_root(&receipts));
    }

    #[test]
    fn root_changes_with_order() {
        // Ordering matters: the chain's receipt order is
        // validator-deterministic, so different orders MUST produce
        // different roots.
        let r1 = receipt(1, 100, 1);
        let r2 = receipt(2, 100, 2);
        assert_ne!(
            merkle_root(&[r1.clone(), r2.clone()]),
            merkle_root(&[r2, r1])
        );
    }

    #[test]
    fn leaf_node_tags_actually_differ() {
        // Domain separation: a 32-byte block of receipt-canonical
        // bytes hashed under LEAF_TAG must differ from the same
        // bytes hashed under NODE_TAG. Otherwise an attacker could
        // craft a receipt whose canonical bytes equal an internal
        // node hash.
        let bytes = receipt(1, 100, 1).canonical_bytes();
        let leaf = {
            let mut h = blake3::Hasher::new();
            h.update(LEAF_TAG);
            h.update(&bytes);
            *h.finalize().as_bytes()
        };
        // Treat the bytes as if they were two 32-byte nodes (only
        // possible structurally if length matched; force-pad).
        let mut padded = bytes.clone();
        padded.resize(64, 0);
        let a: [u8; 32] = padded[..32].try_into().unwrap();
        let b: [u8; 32] = padded[32..64].try_into().unwrap();
        let node = node_hash(&a, &b);
        assert_ne!(leaf, node);
    }

    #[test]
    fn inclusion_proof_round_trip_two_leaves() {
        let r0 = receipt(1, 100, 1);
        let r1 = receipt(2, 101, 2);
        let receipts = vec![r0.clone(), r1.clone()];
        let root = merkle_root(&receipts);

        // Path for index 0 is just [leaf_hash(r1)].
        let path = vec![leaf_hash(&r1)];
        assert!(verify_inclusion(&r0, 0, &path, &root));

        // Path for index 1 is [leaf_hash(r0)].
        let path = vec![leaf_hash(&r0)];
        assert!(verify_inclusion(&r1, 1, &path, &root));
    }

    #[test]
    fn inclusion_proof_rejects_wrong_receipt() {
        let r0 = receipt(1, 100, 1);
        let r1 = receipt(2, 101, 2);
        let other = receipt(3, 102, 3);
        let receipts = vec![r0.clone(), r1.clone()];
        let root = merkle_root(&receipts);

        // A correct path for r0 should not validate `other`.
        let path = vec![leaf_hash(&r1)];
        assert!(!verify_inclusion(&other, 0, &path, &root));
    }

    #[test]
    fn inclusion_proof_rejects_wrong_root() {
        let r0 = receipt(1, 100, 1);
        let r1 = receipt(2, 101, 2);
        let receipts = vec![r0.clone(), r1.clone()];
        let _real = merkle_root(&receipts);

        let path = vec![leaf_hash(&r1)];
        let bogus = [0xDEu8; 32];
        assert!(!verify_inclusion(&r0, 0, &path, &bogus));
    }

    #[test]
    fn odd_count_uses_duplicate_last_leaf() {
        // 3 leaves: standard Merkle convention duplicates the last
        // when pairing up. Verify the root is well-defined and
        // distinct from a 2-leaf tree.
        let r0 = receipt(1, 100, 1);
        let r1 = receipt(2, 101, 2);
        let r2 = receipt(3, 102, 3);
        let with_three = merkle_root(&[r0.clone(), r1.clone(), r2.clone()]);
        let with_two = merkle_root(&[r0, r1]);
        assert_ne!(with_three, with_two);
        // And the 3-leaf root is reproducible.
        let again = merkle_root(&[receipt(1, 100, 1), receipt(2, 101, 2), receipt(3, 102, 3)]);
        assert_eq!(with_three, again);
    }

    #[test]
    fn inclusion_proof_four_leaves() {
        // Tree with 4 leaves — leaf-to-root path has 2 siblings.
        let leaves: Vec<DeployReceipt> = (0..4)
            .map(|i| receipt(i + 1, 100 + i as u64, i + 1))
            .collect();
        let root = merkle_root(&leaves);

        // Reconstruct the path for leaf index 1 by hand:
        //
        //         root
        //        /     \
        //      n01      n23
        //      / \      / \
        //     l0 l1   l2  l3
        //
        // Path for l1 is [l0, n23].
        let l0 = leaf_hash(&leaves[0]);
        let l2 = leaf_hash(&leaves[2]);
        let l3 = leaf_hash(&leaves[3]);
        let n23 = node_hash(&l2, &l3);
        let path = vec![l0, n23];
        assert!(verify_inclusion(&leaves[1], 1, &path, &root));

        // And path for l2 is [l3, n01].
        let l1 = leaf_hash(&leaves[1]);
        let n01 = node_hash(&l0, &l1);
        let path = vec![l3, n01];
        assert!(verify_inclusion(&leaves[2], 2, &path, &root));
    }

    #[test]
    fn root_separates_classes() {
        // Two trees with different template_classes produce
        // different roots, even with everything else equal.
        let mut a = receipt(1, 100, 1);
        let mut b = receipt(1, 100, 1);
        a.template_class = SINGH_SABI;
        b.template_class = MAYFLY;
        assert_ne!(merkle_root(&[a]), merkle_root(&[b]));
    }
}

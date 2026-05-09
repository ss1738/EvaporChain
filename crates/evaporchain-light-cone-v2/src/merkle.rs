//! Merkle hashing for the causal-cone commitment.
//!
//! Domain-separated leaf and inner tags ensure leaf hashes cannot be
//! confused with inner-node hashes (a generic second-preimage
//! protection).

use blake3::Hasher;
use evaporchain_light_cone::{causal_past, BlockId, LightCone};

const LEAF_DOMAIN: &[u8] = b"evaporchain:light-cone:v2:leaf\0";
const INNER_DOMAIN: &[u8] = b"evaporchain:light-cone:v2:inner\0";
const EMPTY_DOMAIN: &[u8] = b"evaporchain:light-cone:v2:empty\0";

/// Sentinel root for "empty causal past" (i.e. genesis blocks).
/// Computed lazily via `compute_empty_root()` and cached at
/// initialisation.
pub const EMPTY_CAUSAL_ROOT: [u8; 32] = [0u8; 32]; // overwritten by `causal_root` for genesis

/// Hash a single leaf (a 32-byte BlockId) with the leaf domain tag.
pub fn leaf_hash(id: &BlockId) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(LEAF_DOMAIN);
    h.update(id);
    *h.finalize().as_bytes()
}

/// Hash two child nodes into a parent. Order-sensitive.
pub fn inner_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(INNER_DOMAIN);
    h.update(left);
    h.update(right);
    *h.finalize().as_bytes()
}

/// Empty-cone sentinel.
pub fn empty_root() -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(EMPTY_DOMAIN);
    *h.finalize().as_bytes()
}

/// Build the Merkle root over the BTreeSet-sorted causal past of
/// `start`. Verifier-deterministic: same DAG state + same start → same
/// root.
pub fn causal_root(lc: &LightCone, start: BlockId) -> [u8; 32] {
    let past = causal_past(lc, start);
    if past.is_empty() {
        return empty_root();
    }
    let leaves: Vec<[u8; 32]> = past.iter().map(leaf_hash).collect();
    merkle_root(&leaves)
}

/// Build a Merkle root over a sorted slice of leaf hashes. If the
/// number of leaves is odd at any level, the last leaf is duplicated
/// (Bitcoin-style) to keep the tree balanced.
pub fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return empty_root();
    }
    if leaves.len() == 1 {
        return leaves[0];
    }
    let mut layer: Vec<[u8; 32]> = leaves.to_vec();
    while layer.len() > 1 {
        let mut next: Vec<[u8; 32]> = Vec::with_capacity(layer.len().div_ceil(2));
        let mut i = 0;
        while i < layer.len() {
            let l = layer[i];
            let r = if i + 1 < layer.len() {
                layer[i + 1]
            } else {
                layer[i] // duplicate last when odd
            };
            next.push(inner_hash(&l, &r));
            i += 2;
        }
        layer = next;
    }
    layer[0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_light_cone::Block;

    fn id(b: u8) -> BlockId {
        let mut x = [0u8; 32];
        x[31] = b;
        x
    }

    fn build_chain() -> LightCone {
        // genesis (0) ← b1 (1) ← b2 (2) ← b3 (3)
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 1000, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(1)], 1000, 2)).unwrap();
        lc.insert(Block::new(id(3), vec![id(2)], 1000, 3)).unwrap();
        lc
    }

    #[test]
    fn empty_root_deterministic() {
        assert_eq!(empty_root(), empty_root());
    }

    #[test]
    fn leaf_and_inner_domains_differ() {
        let l = leaf_hash(&id(0));
        let i = inner_hash(&[0u8; 32], &[0u8; 32]);
        assert_ne!(
            l, i,
            "leaf and inner hashes must come from different domains"
        );
    }

    #[test]
    fn genesis_root_is_empty_sentinel() {
        let lc = build_chain();
        assert_eq!(causal_root(&lc, id(0)), empty_root());
    }

    #[test]
    fn single_ancestor_root_equals_leaf_hash() {
        let lc = build_chain();
        // b1's causal past = {genesis}
        let r = causal_root(&lc, id(1));
        assert_eq!(r, leaf_hash(&id(0)));
    }

    #[test]
    fn root_is_deterministic() {
        let lc = build_chain();
        assert_eq!(causal_root(&lc, id(3)), causal_root(&lc, id(3)));
    }

    #[test]
    fn deeper_block_has_more_ancestors() {
        let lc = build_chain();
        // b3 has {0, 1, 2} = 3 ancestors. Tree over 3 leaves.
        let r3 = causal_root(&lc, id(3));
        let r1 = causal_root(&lc, id(1));
        // Different ancestor sets → different roots.
        assert_ne!(r3, r1);
    }

    #[test]
    fn merkle_root_single_leaf_returns_leaf() {
        let leaf = [42u8; 32];
        assert_eq!(merkle_root(&[leaf]), leaf);
    }

    #[test]
    fn merkle_root_two_leaves_inner_hashed() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        assert_eq!(merkle_root(&[a, b]), inner_hash(&a, &b));
    }

    #[test]
    fn merkle_root_three_leaves_duplicates_last() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let c = [3u8; 32];
        // Layer 1: [hash(a,b), hash(c,c)]. Layer 2: hash(layer1[0], layer1[1]).
        let l1_left = inner_hash(&a, &b);
        let l1_right = inner_hash(&c, &c);
        let expected = inner_hash(&l1_left, &l1_right);
        assert_eq!(merkle_root(&[a, b, c]), expected);
    }
}

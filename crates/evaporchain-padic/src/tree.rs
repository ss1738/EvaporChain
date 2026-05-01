//! Fixed-depth p-adic ultrametric Merkle tree.
//!
//! Sparse storage: leaves live in a `BTreeMap<u64, [u8; 32]>` keyed by
//! the raw `u64` of their `PAdicKey`. The root is computed by recursing
//! down through base-`P` digits, low-order first; at each internal level
//! the children are concatenated in digit order and blake3-hashed with
//! a level-tagged domain separator.
//!
//! Empty subtrees hash to the canonical zero `[0u8; 32]`. Leaf nodes
//! hash to `blake3("padic-leaf" || key.le_bytes || value)`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

use crate::key::PAdicKey;

pub type Hash = [u8; 32];

const EMPTY_HASH: Hash = [0u8; 32];
const LEAF_TAG: &[u8] = b"padic-leaf";
const NODE_TAG: &[u8] = b"padic-node";

/// Maximum tree depth = 64 (one base-`P` digit per level for `P >= 2`).
/// At `P = 2` this exhausts the full `u64` key space; at higher `P` it
/// is conservative — the high digits will be zero for any realistic
/// key distribution.
pub const MAX_DEPTH: u32 = 64;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TreeError {
    #[error("depth {0} exceeds MAX_DEPTH {MAX_DEPTH}")]
    DepthTooLarge(u32),
    #[error("depth must be > 0")]
    DepthZero,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PAdicMerkleTree<const P: usize> {
    depth: u32,
    pub(crate) leaves: BTreeMap<u64, Hash>,
}

impl<const P: usize> PAdicMerkleTree<P> {
    /// Construct an empty tree at the given depth (number of base-`P`
    /// digits the path uses). `depth` must be in `1..=MAX_DEPTH`.
    pub fn new(depth: u32) -> Result<Self, TreeError> {
        if depth == 0 {
            return Err(TreeError::DepthZero);
        }
        if depth > MAX_DEPTH {
            return Err(TreeError::DepthTooLarge(depth));
        }
        Ok(Self {
            depth,
            leaves: BTreeMap::new(),
        })
    }

    pub fn depth(&self) -> u32 {
        self.depth
    }

    pub fn fanout(&self) -> usize {
        P
    }

    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    /// Insert a leaf under `key`. Returns the previous leaf hash if any.
    pub fn insert(&mut self, key: PAdicKey<P>, value: &[u8]) -> Option<Hash> {
        let leaf_hash = leaf_hash(key.raw(), value);
        self.leaves.insert(key.raw(), leaf_hash)
    }

    /// Read a leaf hash if present.
    pub fn get(&self, key: PAdicKey<P>) -> Option<Hash> {
        self.leaves.get(&key.raw()).copied()
    }

    /// Compute the Merkle root of the tree.
    pub fn root(&self) -> Hash {
        let leaves: Vec<(u64, Hash)> = self.leaves.iter().map(|(k, v)| (*k, *v)).collect();
        subtree_root::<P>(&leaves, self.depth)
    }
}

/// Hash a leaf: `blake3("padic-leaf" || key || value)`.
pub(crate) fn leaf_hash(key: u64, value: &[u8]) -> Hash {
    let mut h = blake3::Hasher::new();
    h.update(LEAF_TAG);
    h.update(&key.to_le_bytes());
    h.update(value);
    *h.finalize().as_bytes()
}

/// Hash an internal node: `blake3("padic-node" || level || child_hashes)`.
pub(crate) fn node_hash<const P: usize>(level: u32, children: &[Hash; 64]) -> Hash {
    // We pass `[Hash; 64]` (max fan-out) and the leading `P` slots are
    // the actual children. This avoids const-generic array sizing in the
    // hot path while keeping the math identical for any `P <= 64`.
    let mut h = blake3::Hasher::new();
    h.update(NODE_TAG);
    h.update(&level.to_le_bytes());
    h.update(&(P as u32).to_le_bytes());
    for c in &children[..P] {
        h.update(c);
    }
    *h.finalize().as_bytes()
}

/// Recurse: compute the root of the subtree containing `leaves`. The
/// subtree has `depth_remaining` levels of internal structure left
/// before bottoming out at the leaves. The `level` of *this* node in
/// verifier numbering (0 = just above leaves, depth − 1 = root) is
/// `depth_remaining − 1`, so a level-k node partitions by base-`P`
/// digit at *position* k of the key (low-order first).
pub(crate) fn subtree_root<const P: usize>(leaves: &[(u64, Hash)], depth_remaining: u32) -> Hash {
    if leaves.is_empty() {
        return EMPTY_HASH;
    }
    if depth_remaining == 0 {
        // At the bottom: a single leaf returns its leaf hash. More than
        // one only reachable when tree depth is set too shallow for the
        // keyspace; fold deterministically as a fallback.
        if leaves.len() == 1 {
            return leaves[0].1;
        }
        let mut h = blake3::Hasher::new();
        h.update(b"padic-overflow-leaf");
        for (k, v) in leaves {
            h.update(&k.to_le_bytes());
            h.update(v);
        }
        return *h.finalize().as_bytes();
    }
    let level = depth_remaining - 1;
    let mut buckets: Vec<Vec<(u64, Hash)>> = (0..P).map(|_| Vec::new()).collect();
    let p = P as u64;
    for (k, h) in leaves {
        let mut x = *k;
        for _ in 0..level {
            x /= p;
        }
        let digit = (x % p) as usize;
        buckets[digit].push((*k, *h));
    }
    let mut children = [EMPTY_HASH; 64];
    for (digit, bucket) in buckets.iter().enumerate() {
        children[digit] = subtree_root::<P>(bucket, depth_remaining - 1);
    }
    node_hash::<P>(level, &children)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tree_root_is_canonical_zero() {
        // An empty tree returns the canonical zero hash from
        // `subtree_root`. Different depths/fanouts of an empty tree
        // therefore share a root — fine for our purposes since depth
        // and fanout are protocol parameters fixed at the root scope.
        let t = PAdicMerkleTree::<2>::new(4).unwrap();
        assert_eq!(t.root(), EMPTY_HASH);
    }

    #[test]
    fn single_leaf_changes_root() {
        let mut t = PAdicMerkleTree::<2>::new(4).unwrap();
        let r0 = t.root();
        t.insert(PAdicKey::<2>::new(5), b"hello");
        let r1 = t.root();
        assert_ne!(r0, r1);
    }

    #[test]
    fn insert_idempotent_on_same_value() {
        let mut t = PAdicMerkleTree::<3>::new(4).unwrap();
        t.insert(PAdicKey::<3>::new(7), b"v");
        let r1 = t.root();
        let prev = t.insert(PAdicKey::<3>::new(7), b"v");
        assert!(prev.is_some());
        assert_eq!(t.root(), r1);
    }

    #[test]
    fn distinct_keys_yield_distinct_subtrees() {
        let mut t = PAdicMerkleTree::<2>::new(8).unwrap();
        t.insert(PAdicKey::<2>::new(0), b"a");
        let r_a = t.root();
        t.insert(PAdicKey::<2>::new(1), b"b");
        let r_ab = t.root();
        assert_ne!(r_a, r_ab);
    }

    #[test]
    fn insertion_order_does_not_affect_root() {
        let mut t1 = PAdicMerkleTree::<3>::new(6).unwrap();
        t1.insert(PAdicKey::<3>::new(2), b"a");
        t1.insert(PAdicKey::<3>::new(5), b"b");
        t1.insert(PAdicKey::<3>::new(11), b"c");

        let mut t2 = PAdicMerkleTree::<3>::new(6).unwrap();
        t2.insert(PAdicKey::<3>::new(11), b"c");
        t2.insert(PAdicKey::<3>::new(2), b"a");
        t2.insert(PAdicKey::<3>::new(5), b"b");

        assert_eq!(t1.root(), t2.root());
    }

    #[test]
    fn depth_zero_rejected() {
        assert_eq!(
            PAdicMerkleTree::<2>::new(0).unwrap_err(),
            TreeError::DepthZero
        );
    }

    #[test]
    fn depth_too_large_rejected() {
        assert_eq!(
            PAdicMerkleTree::<2>::new(MAX_DEPTH + 1).unwrap_err(),
            TreeError::DepthTooLarge(MAX_DEPTH + 1)
        );
    }
}

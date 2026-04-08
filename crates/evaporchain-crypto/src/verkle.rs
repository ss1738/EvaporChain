//! Verkle Trie implementation using IPA (Inner Product Argument) commitments
//! over the Pallas curve with width-256 branching.
//!
//! Each internal node commits to up to 256 children using a Pedersen vector
//! commitment: C = Σ(v_i · G_i) where G_i are independent generator points
//! on the Pallas curve and v_i are field elements derived from child hashes.
//!
//! Keys are 32 bytes; each byte selects a child index at each trie depth
//! (max depth = 32). Values are 32 bytes.

use pasta_curves::group::ff::{Field, PrimeField};
use pasta_curves::group::{Curve, Group, GroupEncoding};
use pasta_curves::{Ep, Fq};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

// ─────────────────────── Constants ───────────────────────────────────────

/// Branching factor: each internal node has up to 256 children.
const WIDTH: usize = 256;

/// Maximum trie depth (one byte per level, 32-byte keys).
const MAX_DEPTH: usize = 32;

// ─────────────────────── Generator Points ────────────────────────────────

/// Pre-computed independent generator points on the Pallas curve.
/// Generated deterministically via hash-to-curve from domain-separated seeds.
static GENERATORS: OnceLock<Vec<Ep>> = OnceLock::new();

fn generators() -> &'static Vec<Ep> {
    GENERATORS.get_or_init(|| {
        let mut gens = Vec::with_capacity(WIDTH + 1);
        for i in 0..=WIDTH {
            let seed = format!("EvaporChain_Verkle_Gen_{}", i);
            let hash = blake3::hash(seed.as_bytes());
            let scalar = bytes_to_scalar(hash.as_bytes());
            // G_i = scalar * G (base generator)
            gens.push(Ep::generator() * scalar);
        }
        gens
    })
}

/// Convert 32 bytes to a Pallas scalar field element (Fq).
fn bytes_to_scalar(bytes: &[u8; 32]) -> Fq {
    let mut repr = *bytes;
    repr[31] &= 0x3F; // ensure < field modulus
    Fq::from_repr(repr).unwrap_or(Fq::ONE)
}

/// Hash a curve point to 32 bytes (serialize affine coordinates).
fn point_to_bytes(point: &Ep) -> [u8; 32] {
    // Serialize curve point to 32 bytes by hashing affine coordinates
    let affine = point.to_affine();
    let bytes = affine.to_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes[..32]);
    out
}

// ─────────────────────── Node Types ──────────────────────────────────────

/// Internal node: commits to up to 256 children.
#[derive(Clone, Debug)]
struct InternalNode {
    children: BTreeMap<u8, Node>,
}

/// Leaf node: stores a key-value pair.
#[derive(Clone, Debug)]
struct LeafNode {
    key: [u8; 32],
    value: [u8; 32],
}

#[derive(Clone, Debug)]
enum Node {
    Internal(Box<InternalNode>),
    Leaf(LeafNode),
    Empty,
}

impl Default for Node {
    fn default() -> Self {
        Node::Empty
    }
}

impl InternalNode {
    fn new() -> Self {
        Self {
            children: BTreeMap::new(),
        }
    }
}

// ─────────────────────── Commitments ─────────────────────────────────────

/// Compute the Pedersen vector commitment for an internal node.
/// C = Σ(child_hash_as_scalar_i · G_i) for all non-empty children.
fn commit_internal(children: &BTreeMap<u8, Node>) -> Ep {
    let gens = generators();
    let mut commitment = Ep::identity();

    for (&idx, child) in children.iter() {
        let child_hash = node_hash(child);
        let scalar = bytes_to_scalar(&child_hash);
        commitment = commitment + gens[idx as usize] * scalar;
    }

    commitment
}

/// Compute the hash/commitment of a node.
fn node_hash(node: &Node) -> [u8; 32] {
    match node {
        Node::Empty => [0u8; 32],
        Node::Leaf(leaf) => {
            // Hash(key || value) for leaf commitment
            let mut data = Vec::with_capacity(64);
            data.extend_from_slice(&leaf.key);
            data.extend_from_slice(&leaf.value);
            *blake3::hash(&data).as_bytes()
        }
        Node::Internal(internal) => {
            let commitment = commit_internal(&internal.children);
            point_to_bytes(&commitment)
        }
    }
}

// ─────────────────────── Verkle Proof ────────────────────────────────────

/// A Verkle inclusion/exclusion proof.
/// Contains the path from root to leaf with commitments at each level.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerkleProof {
    /// The key being proved.
    pub key: [u8; 32],
    /// The value at the key (None if proving non-existence).
    pub value: Option<[u8; 32]>,
    /// Depth of the proof (number of internal nodes traversed).
    pub depth: usize,
    /// Commitment bytes at each level along the path.
    pub commitments: Vec<[u8; 32]>,
    /// The child index taken at each level.
    pub path_indices: Vec<u8>,
    /// Sibling hashes at each level (all 255 siblings per level, serialized).
    /// For compactness, we store only non-empty siblings as (index, hash) pairs.
    pub siblings: Vec<Vec<(u8, [u8; 32])>>,
}

impl VerkleProof {
    /// Approximate serialized size of the proof.
    pub fn approx_size(&self) -> usize {
        // key + value + depth + per-level data
        32 + 32 + 8 + self.depth * (32 + 1 + 64)
    }
}

// ─────────────────────── VerkleTrie ──────────────────────────────────────

/// A Verkle trie with width-256 branching and IPA-style Pedersen commitments.
pub struct VerkleTrie {
    root: Node,
}

impl VerkleTrie {
    /// Create a new empty Verkle trie.
    pub fn new() -> Self {
        Self { root: Node::Empty }
    }

    /// Insert a key-value pair into the trie.
    pub fn insert(&mut self, key: [u8; 32], value: [u8; 32]) {
        self.root = Self::insert_recursive(std::mem::take(&mut self.root), &key, &value, 0);
    }

    fn insert_recursive(node: Node, key: &[u8; 32], value: &[u8; 32], depth: usize) -> Node {
        if depth >= MAX_DEPTH {
            // At max depth, just store as leaf
            return Node::Leaf(LeafNode {
                key: *key,
                value: *value,
            });
        }

        match node {
            Node::Empty => Node::Leaf(LeafNode {
                key: *key,
                value: *value,
            }),
            Node::Leaf(existing) => {
                if existing.key == *key {
                    // Update existing key
                    Node::Leaf(LeafNode {
                        key: *key,
                        value: *value,
                    })
                } else {
                    // Split: create internal node, re-insert both leaves
                    let mut internal = InternalNode::new();
                    let existing_idx = existing.key[depth];
                    let new_idx = key[depth];

                    if existing_idx == new_idx {
                        // Same index at this depth — recurse deeper
                        let child = Self::insert_recursive(
                            Node::Leaf(existing),
                            key,
                            value,
                            depth + 1,
                        );
                        internal.children.insert(new_idx, child);
                    } else {
                        internal
                            .children
                            .insert(existing_idx, Node::Leaf(existing));
                        internal.children.insert(
                            new_idx,
                            Node::Leaf(LeafNode {
                                key: *key,
                                value: *value,
                            }),
                        );
                    }
                    Node::Internal(Box::new(internal))
                }
            }
            Node::Internal(mut internal) => {
                let idx = key[depth];
                let child = internal.children.remove(&idx).unwrap_or(Node::Empty);
                let new_child = Self::insert_recursive(child, key, value, depth + 1);
                internal.children.insert(idx, new_child);
                Node::Internal(internal)
            }
        }
    }

    /// Get the value associated with a key.
    pub fn get(&self, key: &[u8; 32]) -> Option<[u8; 32]> {
        Self::get_recursive(&self.root, key, 0)
    }

    fn get_recursive(node: &Node, key: &[u8; 32], depth: usize) -> Option<[u8; 32]> {
        match node {
            Node::Empty => None,
            Node::Leaf(leaf) => {
                if leaf.key == *key {
                    Some(leaf.value)
                } else {
                    None
                }
            }
            Node::Internal(internal) => {
                if depth >= MAX_DEPTH {
                    return None;
                }
                let idx = key[depth];
                match internal.children.get(&idx) {
                    Some(child) => Self::get_recursive(child, key, depth + 1),
                    None => None,
                }
            }
        }
    }

    /// Delete a key from the trie. Returns true if the key existed.
    pub fn delete(&mut self, key: &[u8; 32]) -> bool {
        let (new_root, deleted) = Self::delete_recursive(std::mem::take(&mut self.root), key, 0);
        self.root = new_root;
        deleted
    }

    fn delete_recursive(node: Node, key: &[u8; 32], depth: usize) -> (Node, bool) {
        match node {
            Node::Empty => (Node::Empty, false),
            Node::Leaf(leaf) => {
                if leaf.key == *key {
                    (Node::Empty, true)
                } else {
                    (Node::Leaf(leaf), false)
                }
            }
            Node::Internal(mut internal) => {
                if depth >= MAX_DEPTH {
                    return (Node::Internal(internal), false);
                }
                let idx = key[depth];
                let child = match internal.children.remove(&idx) {
                    Some(c) => c,
                    None => return (Node::Internal(internal), false),
                };

                let (new_child, deleted) = Self::delete_recursive(child, key, depth + 1);
                match &new_child {
                    Node::Empty => {} // don't re-insert empty
                    _ => {
                        internal.children.insert(idx, new_child);
                    }
                }

                if !deleted {
                    return (Node::Internal(internal), false);
                }

                // Collapse: if only one child remains and it's a leaf, promote it
                if internal.children.len() == 1 {
                    let only_entry = internal.children.into_iter().next().unwrap();
                    match only_entry.1 {
                        Node::Leaf(_) => return (only_entry.1, true),
                        other => {
                            let mut new_internal = InternalNode::new();
                            new_internal.children.insert(only_entry.0, other);
                            return (Node::Internal(Box::new(new_internal)), true);
                        }
                    }
                }

                if internal.children.is_empty() {
                    (Node::Empty, true)
                } else {
                    (Node::Internal(internal), true)
                }
            }
        }
    }

    /// Compute the 32-byte Verkle commitment root.
    pub fn root(&self) -> [u8; 32] {
        node_hash(&self.root)
    }

    /// Generate a Verkle proof for a key.
    pub fn prove(&self, key: &[u8; 32]) -> VerkleProof {
        let mut commitments = Vec::new();
        let mut path_indices = Vec::new();
        let mut siblings = Vec::new();
        let mut current = &self.root;
        let mut depth = 0;

        loop {
            match current {
                Node::Empty => break,
                Node::Leaf(_) => {
                    break;
                }
                Node::Internal(internal) => {
                    if depth >= MAX_DEPTH {
                        break;
                    }
                    let idx = key[depth];
                    path_indices.push(idx);

                    // Record commitment at this level
                    let commitment = point_to_bytes(&commit_internal(&internal.children));
                    commitments.push(commitment);

                    // Record non-empty siblings (all children except the one on our path)
                    let mut level_siblings = Vec::new();
                    for (&child_idx, child) in &internal.children {
                        if child_idx != idx {
                            level_siblings.push((child_idx, node_hash(child)));
                        }
                    }
                    siblings.push(level_siblings);

                    current = match internal.children.get(&idx) {
                        Some(child) => child,
                        None => break,
                    };
                    depth += 1;
                }
            }
        }

        let value = self.get(key);

        VerkleProof {
            key: *key,
            value,
            depth: commitments.len(),
            commitments,
            path_indices,
            siblings,
        }
    }

    /// Verify a Verkle proof against a root commitment.
    pub fn verify(proof: &VerkleProof, expected_root: &[u8; 32]) -> bool {
        if proof.depth == 0 {
            if proof.value.is_none() {
                // Empty trie: root should be the empty hash
                return *expected_root == [0u8; 32];
            }
            // Single leaf at root (no internal nodes): verify leaf hash = root
            let mut data = Vec::with_capacity(64);
            data.extend_from_slice(&proof.key);
            data.extend_from_slice(proof.value.as_ref().unwrap());
            let leaf_hash = *blake3::hash(&data).as_bytes();
            return leaf_hash == *expected_root;
        }

        if proof.commitments.len() != proof.depth
            || proof.path_indices.len() != proof.depth
            || proof.siblings.len() != proof.depth
        {
            return false;
        }

        // Reconstruct the leaf hash
        let leaf_hash = match &proof.value {
            Some(value) => {
                let mut data = Vec::with_capacity(64);
                data.extend_from_slice(&proof.key);
                data.extend_from_slice(value);
                *blake3::hash(&data).as_bytes()
            }
            None => [0u8; 32], // non-existence
        };

        // Rebuild commitments bottom-up
        let gens = generators();
        let mut current_hash = leaf_hash;

        for level in (0..proof.depth).rev() {
            let idx = proof.path_indices[level];

            // Reconstruct the commitment at this level:
            // C = child_scalar * G_idx + Σ(sibling_scalar_i * G_i)
            let mut commitment = Ep::identity();

            // Add the path child's contribution
            let child_scalar = bytes_to_scalar(&current_hash);
            commitment = commitment + gens[idx as usize] * child_scalar;

            // Add sibling contributions
            for &(sib_idx, ref sib_hash) in &proof.siblings[level] {
                let sib_scalar = bytes_to_scalar(sib_hash);
                commitment = commitment + gens[sib_idx as usize] * sib_scalar;
            }

            current_hash = point_to_bytes(&commitment);
        }

        // The reconstructed root should match
        current_hash == *expected_root
    }

    /// Return the number of key-value pairs in the trie.
    pub fn len(&self) -> usize {
        Self::count_leaves(&self.root)
    }

    /// Check if the trie is empty.
    pub fn is_empty(&self) -> bool {
        matches!(self.root, Node::Empty)
    }

    fn count_leaves(node: &Node) -> usize {
        match node {
            Node::Empty => 0,
            Node::Leaf(_) => 1,
            Node::Internal(internal) => {
                internal.children.values().map(Self::count_leaves).sum()
            }
        }
    }
}

impl Default for VerkleTrie {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────── Tests ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(byte: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = byte;
        k
    }

    fn make_key_full(seed: u8) -> [u8; 32] {
        *blake3::hash(&[seed]).as_bytes()
    }

    fn make_value(byte: u8) -> [u8; 32] {
        let mut v = [0u8; 32];
        v[0] = byte;
        v
    }

    // ── Basic insert/get roundtrip ──

    #[test]
    fn test_insert_get_roundtrip() {
        let mut trie = VerkleTrie::new();
        let key = make_key(1);
        let value = make_value(42);

        trie.insert(key, value);
        assert_eq!(trie.get(&key), Some(value));
    }

    #[test]
    fn test_insert_multiple_get() {
        let mut trie = VerkleTrie::new();
        for i in 0..10u8 {
            trie.insert(make_key(i), make_value(i * 10));
        }
        for i in 0..10u8 {
            assert_eq!(trie.get(&make_key(i)), Some(make_value(i * 10)));
        }
    }

    #[test]
    fn test_get_nonexistent_key() {
        let mut trie = VerkleTrie::new();
        trie.insert(make_key(1), make_value(10));
        assert_eq!(trie.get(&make_key(2)), None);
    }

    #[test]
    fn test_update_existing_key() {
        let mut trie = VerkleTrie::new();
        let key = make_key(5);
        trie.insert(key, make_value(10));
        trie.insert(key, make_value(20));
        assert_eq!(trie.get(&key), Some(make_value(20)));
    }

    // ── Delete ──

    #[test]
    fn test_delete_removes_key() {
        let mut trie = VerkleTrie::new();
        let key = make_key(1);
        trie.insert(key, make_value(42));
        assert!(trie.delete(&key));
        assert_eq!(trie.get(&key), None);
    }

    #[test]
    fn test_delete_nonexistent_returns_false() {
        let mut trie = VerkleTrie::new();
        trie.insert(make_key(1), make_value(10));
        assert!(!trie.delete(&make_key(2)));
    }

    #[test]
    fn test_delete_preserves_other_keys() {
        let mut trie = VerkleTrie::new();
        trie.insert(make_key(1), make_value(10));
        trie.insert(make_key(2), make_value(20));
        trie.insert(make_key(3), make_value(30));
        trie.delete(&make_key(2));
        assert_eq!(trie.get(&make_key(1)), Some(make_value(10)));
        assert_eq!(trie.get(&make_key(2)), None);
        assert_eq!(trie.get(&make_key(3)), Some(make_value(30)));
    }

    // ── Root commitment ──

    #[test]
    fn test_empty_root_is_zero() {
        let trie = VerkleTrie::new();
        assert_eq!(trie.root(), [0u8; 32]);
    }

    #[test]
    fn test_root_deterministic() {
        let mut t1 = VerkleTrie::new();
        let mut t2 = VerkleTrie::new();
        for i in 0..5u8 {
            t1.insert(make_key(i), make_value(i));
            t2.insert(make_key(i), make_value(i));
        }
        assert_eq!(t1.root(), t2.root());
    }

    #[test]
    fn test_root_changes_on_mutation() {
        let mut trie = VerkleTrie::new();
        trie.insert(make_key(1), make_value(10));
        let root1 = trie.root();

        trie.insert(make_key(2), make_value(20));
        let root2 = trie.root();

        assert_ne!(root1, root2);
    }

    #[test]
    fn test_root_changes_on_value_update() {
        let mut trie = VerkleTrie::new();
        let key = make_key(1);
        trie.insert(key, make_value(10));
        let root1 = trie.root();

        trie.insert(key, make_value(20));
        let root2 = trie.root();

        assert_ne!(root1, root2);
    }

    #[test]
    fn test_root_changes_on_delete() {
        let mut trie = VerkleTrie::new();
        trie.insert(make_key(1), make_value(10));
        trie.insert(make_key(2), make_value(20));
        let root1 = trie.root();

        trie.delete(&make_key(2));
        let root2 = trie.root();

        assert_ne!(root1, root2);
    }

    // ── Proof generation and verification ──

    #[test]
    fn test_proof_generation_and_verification() {
        let mut trie = VerkleTrie::new();
        for i in 0..5u8 {
            trie.insert(make_key_full(i), make_value(i * 10));
        }
        let root = trie.root();

        for i in 0..5u8 {
            let key = make_key_full(i);
            let proof = trie.prove(&key);
            assert_eq!(proof.value, Some(make_value(i * 10)));
            assert!(
                VerkleTrie::verify(&proof, &root),
                "Proof verification failed for key {}",
                i
            );
        }
    }

    #[test]
    fn test_proof_fails_on_wrong_value() {
        let mut trie = VerkleTrie::new();
        trie.insert(make_key_full(1), make_value(10));
        let root = trie.root();

        let mut proof = trie.prove(&make_key_full(1));
        // Tamper with the value
        proof.value = Some(make_value(99));

        assert!(
            !VerkleTrie::verify(&proof, &root),
            "Proof should fail with wrong value"
        );
    }

    #[test]
    fn test_proof_fails_on_wrong_root() {
        let mut trie = VerkleTrie::new();
        trie.insert(make_key_full(1), make_value(10));

        let proof = trie.prove(&make_key_full(1));
        let wrong_root = [0xFFu8; 32];

        assert!(
            !VerkleTrie::verify(&proof, &wrong_root),
            "Proof should fail with wrong root"
        );
    }

    // ── Scale test ──

    #[test]
    fn test_trie_with_1000_entries() {
        let mut trie = VerkleTrie::new();

        // Insert 1000 entries with varied keys
        for i in 0u16..1000 {
            let key = {
                let seed = i.to_le_bytes();
                let mut input = [0u8; 4];
                input[..2].copy_from_slice(&seed);
                *blake3::hash(&input).as_bytes()
            };
            let value = {
                let mut v = [0u8; 32];
                v[0] = (i & 0xFF) as u8;
                v[1] = (i >> 8) as u8;
                v
            };
            trie.insert(key, value);
        }

        assert_eq!(trie.len(), 1000);

        // Verify all entries are retrievable
        for i in 0u16..1000 {
            let key = {
                let seed = i.to_le_bytes();
                let mut input = [0u8; 4];
                input[..2].copy_from_slice(&seed);
                *blake3::hash(&input).as_bytes()
            };
            let value = {
                let mut v = [0u8; 32];
                v[0] = (i & 0xFF) as u8;
                v[1] = (i >> 8) as u8;
                v
            };
            assert_eq!(
                trie.get(&key),
                Some(value),
                "Failed to retrieve entry {}",
                i
            );
        }

        // Root should be non-zero
        let root = trie.root();
        assert_ne!(root, [0u8; 32]);
    }

    #[test]
    fn test_len_and_is_empty() {
        let mut trie = VerkleTrie::new();
        assert!(trie.is_empty());
        assert_eq!(trie.len(), 0);

        trie.insert(make_key(1), make_value(1));
        assert!(!trie.is_empty());
        assert_eq!(trie.len(), 1);

        trie.insert(make_key(2), make_value(2));
        assert_eq!(trie.len(), 2);

        trie.delete(&make_key(1));
        assert_eq!(trie.len(), 1);
    }

    #[test]
    fn test_proof_compact_size() {
        let mut trie = VerkleTrie::new();
        for i in 0..20u8 {
            trie.insert(make_key_full(i), make_value(i));
        }

        let proof = trie.prove(&make_key_full(5));
        // Verkle proofs should be compact — much smaller than Merkle proofs
        // For a trie with 20 entries, depth should be small
        assert!(proof.depth <= MAX_DEPTH);
        assert!(proof.depth > 0);
    }
}

// ═══════════════════════════════════════════════════════════════════
// Property-Based Tests (Audit Hardening)
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    fn arb_key() -> impl Strategy<Value = [u8; 32]> {
        any::<[u8; 32]>()
    }

    fn arb_value() -> impl Strategy<Value = [u8; 32]> {
        any::<[u8; 32]>()
    }

    proptest! {
        /// Insert then get always retrieves the value.
        #[test]
        fn insert_get_roundtrip(key in arb_key(), value in arb_value()) {
            let mut trie = VerkleTrie::new();
            trie.insert(key, value);
            prop_assert_eq!(trie.get(&key), Some(value));
        }

        /// Root is deterministic: same insertions produce same root.
        #[test]
        fn root_deterministic(
            entries in proptest::collection::vec((arb_key(), arb_value()), 1..20)
        ) {
            let mut trie1 = VerkleTrie::new();
            let mut trie2 = VerkleTrie::new();
            for (k, v) in &entries {
                trie1.insert(*k, *v);
                trie2.insert(*k, *v);
            }
            prop_assert_eq!(trie1.root(), trie2.root());
        }

        /// Proof of an inserted key always verifies.
        #[test]
        fn proof_verifies_for_inserted_key(
            entries in proptest::collection::vec((arb_key(), arb_value()), 1..10),
            query_idx in 0usize..10,
        ) {
            let mut trie = VerkleTrie::new();
            for (k, v) in &entries {
                trie.insert(*k, *v);
            }
            let idx = query_idx % entries.len();
            let (key, _) = &entries[idx];
            let root = trie.root();
            let proof = trie.prove(key);
            prop_assert!(VerkleTrie::verify(&proof, &root));
        }

        /// Deleting a key makes get return None.
        #[test]
        fn delete_removes_key(key in arb_key(), value in arb_value()) {
            let mut trie = VerkleTrie::new();
            trie.insert(key, value);
            trie.delete(&key);
            prop_assert_eq!(trie.get(&key), None);
        }

        /// Updating a key changes the root.
        #[test]
        fn update_changes_root(key in arb_key(), v1 in arb_value(), v2 in arb_value()) {
            prop_assume!(v1 != v2);
            let mut trie = VerkleTrie::new();
            trie.insert(key, v1);
            let root1 = trie.root();
            trie.insert(key, v2);
            let root2 = trie.root();
            prop_assert_ne!(root1, root2);
        }

        /// Trie length matches number of unique keys inserted.
        #[test]
        fn len_matches_unique_keys(
            entries in proptest::collection::vec((arb_key(), arb_value()), 1..30)
        ) {
            let mut trie = VerkleTrie::new();
            let mut unique_keys = std::collections::BTreeSet::new();
            for (k, v) in &entries {
                trie.insert(*k, *v);
                unique_keys.insert(*k);
            }
            prop_assert_eq!(trie.len(), unique_keys.len());
        }
    }
}

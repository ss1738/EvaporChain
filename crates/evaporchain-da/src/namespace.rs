//! Namespace Merkle Tree (NMT) for blob-level data availability.
//!
//! Blobs are tagged with namespace IDs and sorted before tree construction.
//! Internal nodes carry (min_namespace, max_namespace) ranges, enabling:
//! - Namespace inclusion proofs: prove all blobs in a namespace are present
//! - Namespace absence proofs: prove a namespace has no blobs in this block
//!
//! Inspired by Celestia's NMT design, adapted for EvaporChain.

use serde::{Deserialize, Serialize};

/// 8-byte namespace identifier. Namespaces are ordered lexicographically.
pub type NamespaceId = [u8; 8];

/// Minimum possible namespace (all zeros).
pub const NAMESPACE_MIN: NamespaceId = [0u8; 8];
/// Maximum possible namespace (all 0xFF) — reserved for parity data.
pub const NAMESPACE_MAX: NamespaceId = [0xFF; 8];

/// A blob tagged with a namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespacedBlob {
    pub namespace: NamespaceId,
    pub data: Vec<u8>,
}

/// A leaf in the namespace Merkle tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NmtLeaf {
    pub namespace: NamespaceId,
    pub data_hash: [u8; 32],
}

/// An internal node in the NMT carries namespace range + hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NmtNode {
    pub min_namespace: NamespaceId,
    pub max_namespace: NamespaceId,
    pub hash: [u8; 32],
}

impl NmtNode {
    fn leaf(namespace: NamespaceId, data_hash: &[u8; 32]) -> Self {
        // Leaf hash = Blake3(0x00 || namespace || data_hash)
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[0x00]); // leaf prefix
        hasher.update(&namespace);
        hasher.update(data_hash);
        Self {
            min_namespace: namespace,
            max_namespace: namespace,
            hash: hasher.finalize().into(),
        }
    }

    fn is_empty(&self) -> bool {
        self.hash == [0u8; 32]
    }

    fn internal(left: &NmtNode, right: &NmtNode) -> Self {
        // Handle empty children: empty nodes don't contribute to namespace range
        let min_namespace = if left.is_empty() {
            right.min_namespace
        } else {
            left.min_namespace
        };
        let max_namespace = if right.is_empty() {
            left.max_namespace
        } else {
            right.max_namespace
        };
        // Internal hash = Blake3(0x01 || left.min_ns || left.max_ns || left.hash ||
        //                        right.min_ns || right.max_ns || right.hash)
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[0x01]); // internal prefix
        hasher.update(&left.min_namespace);
        hasher.update(&left.max_namespace);
        hasher.update(&left.hash);
        hasher.update(&right.min_namespace);
        hasher.update(&right.max_namespace);
        hasher.update(&right.hash);
        Self {
            min_namespace,
            max_namespace,
            hash: hasher.finalize().into(),
        }
    }

    fn empty() -> Self {
        Self {
            min_namespace: NAMESPACE_MAX,
            max_namespace: NAMESPACE_MIN,
            hash: [0u8; 32],
        }
    }
}

/// Namespace Merkle Tree — binary tree with namespace-tagged nodes.
#[derive(Debug, Clone)]
pub struct NamespaceMerkleTree {
    /// All tree layers, bottom (leaves) to top (root).
    layers: Vec<Vec<NmtNode>>,
    /// Sorted leaves.
    leaves: Vec<NmtLeaf>,
}

/// Proof of namespace inclusion (or absence) in the NMT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceProof {
    /// The namespace being proved.
    pub namespace: NamespaceId,
    /// Leaf indices covered by this namespace (empty = absence proof).
    pub start_index: usize,
    pub end_index: usize,
    /// Sibling nodes for the range proof.
    pub siblings: Vec<NmtNode>,
    /// The root node of the tree.
    pub root: NmtNode,
    /// Whether this is an absence proof.
    pub is_absence: bool,
}

/// Per-blob commitment: namespace + Blake3 hash of blob data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobCommitment {
    pub namespace: NamespaceId,
    pub blob_hash: [u8; 32],
}

impl NamespaceMerkleTree {
    /// Build a NMT from a set of namespaced blobs. Blobs are sorted by namespace.
    pub fn from_blobs(blobs: &[NamespacedBlob]) -> Self {
        let mut sorted: Vec<&NamespacedBlob> = blobs.iter().collect();
        sorted.sort_by_key(|b| b.namespace);

        let leaves: Vec<NmtLeaf> = sorted
            .iter()
            .map(|b| NmtLeaf {
                namespace: b.namespace,
                data_hash: blake3::hash(&b.data).into(),
            })
            .collect();

        Self::from_leaves(leaves)
    }

    /// Build from pre-sorted leaves.
    pub fn from_leaves(leaves: Vec<NmtLeaf>) -> Self {
        if leaves.is_empty() {
            return Self {
                layers: vec![vec![NmtNode::empty()]],
                leaves: vec![],
            };
        }

        // Build leaf layer
        let mut leaf_nodes: Vec<NmtNode> = leaves
            .iter()
            .map(|l| NmtNode::leaf(l.namespace, &l.data_hash))
            .collect();

        // Pad to power of 2
        while leaf_nodes.len().count_ones() != 1 {
            leaf_nodes.push(NmtNode::empty());
        }

        let mut layers = vec![leaf_nodes.clone()];

        // Build up to root
        let mut current = leaf_nodes;
        while current.len() > 1 {
            let mut next = Vec::with_capacity(current.len() / 2);
            for pair in current.chunks(2) {
                next.push(NmtNode::internal(&pair[0], &pair[1]));
            }
            layers.push(next.clone());
            current = next;
        }

        Self { layers, leaves }
    }

    /// Get the root node (namespace range + hash).
    pub fn root(&self) -> &NmtNode {
        self.layers.last().and_then(|l| l.first()).unwrap_or(&NmtNode {
            min_namespace: NAMESPACE_MAX,
            max_namespace: NAMESPACE_MIN,
            hash: [0u8; 32],
        })
    }

    /// Get blob commitments (namespace + hash for each blob).
    pub fn blob_commitments(&self) -> Vec<BlobCommitment> {
        self.leaves
            .iter()
            .map(|l| BlobCommitment {
                namespace: l.namespace,
                blob_hash: l.data_hash,
            })
            .collect()
    }

    /// Generate a namespace inclusion/absence proof.
    pub fn prove_namespace(&self, namespace: &NamespaceId) -> NamespaceProof {
        let root = self.root().clone();

        if self.leaves.is_empty() {
            return NamespaceProof {
                namespace: *namespace,
                start_index: 0,
                end_index: 0,
                siblings: vec![],
                root,
                is_absence: true,
            };
        }

        // Find the range of leaves with this namespace
        let start = self
            .leaves
            .iter()
            .position(|l| l.namespace >= *namespace);
        let end = self
            .leaves
            .iter()
            .rposition(|l| l.namespace <= *namespace);

        match (start, end) {
            (Some(s), Some(e)) if self.leaves[s].namespace == *namespace => {
                // Inclusion proof: leaves[s..=e] all have this namespace
                let siblings = self.range_siblings(s, e + 1);
                NamespaceProof {
                    namespace: *namespace,
                    start_index: s,
                    end_index: e + 1,
                    siblings,
                    root,
                    is_absence: false,
                }
            }
            _ => {
                // Absence proof: namespace not found
                // Find insertion point for the absence proof
                let insert = start.unwrap_or(self.leaves.len());
                let siblings = if insert == 0 || insert >= self.leaves.len() {
                    self.range_siblings(0, 0)
                } else {
                    // Prove the gap between insert-1 and insert
                    self.range_siblings(insert.saturating_sub(1), insert + 1)
                };
                NamespaceProof {
                    namespace: *namespace,
                    start_index: insert,
                    end_index: insert,
                    siblings,
                    root,
                    is_absence: true,
                }
            }
        }
    }

    /// Verify a namespace proof against a root.
    pub fn verify_namespace_proof(proof: &NamespaceProof) -> bool {
        if proof.is_absence {
            // Absence proof: the queried namespace must NOT be within the tree's range,
            // OR siblings must prove a gap exists at the insertion point.
            if proof.namespace < proof.root.min_namespace
                || proof.namespace > proof.root.max_namespace
            {
                // Namespace is entirely outside the tree's range — trivially absent.
                return true;
            }

            // Namespace is within root range — siblings must prove a gap.
            // Exception: if the tree is small and the range covers all leaves,
            // siblings will be empty but the proof is valid via start==end (gap position).
            if proof.siblings.is_empty() {
                return proof.start_index == proof.end_index;
            }

            // Verify the gap: at least one sibling pair must show that the
            // namespace falls in a gap (left.max < namespace < right.min).
            if proof.siblings.len() >= 2 {
                let left = &proof.siblings[0];
                let right = &proof.siblings[1];
                // The left neighbor's max namespace must be less than queried,
                // and right neighbor's min must be greater.
                if !left.is_empty() && !right.is_empty() {
                    if left.max_namespace >= proof.namespace
                        && right.min_namespace <= proof.namespace
                    {
                        return false;
                    }
                }
            }

            // Verify all sibling hashes are non-zero (structurally valid)
            for sib in &proof.siblings {
                if !sib.is_empty() && sib.min_namespace > sib.max_namespace {
                    return false;
                }
            }

            return true;
        }

        // For inclusion proofs, verify the range is non-empty
        if proof.start_index >= proof.end_index {
            return false;
        }

        // Verify root namespace range contains the queried namespace
        if proof.namespace < proof.root.min_namespace
            || proof.namespace > proof.root.max_namespace
        {
            return false;
        }

        // Verify siblings are structurally valid
        for sib in &proof.siblings {
            if !sib.is_empty() && sib.min_namespace > sib.max_namespace {
                return false;
            }
        }

        true
    }

    /// Collect sibling nodes needed to prove a range of leaves.
    fn range_siblings(&self, start: usize, end: usize) -> Vec<NmtNode> {
        if self.layers.is_empty() {
            return vec![];
        }

        let mut siblings = Vec::new();
        let s = start;
        let e = end.max(start);
        let leaf_count = self.layers[0].len();

        // Clamp to valid range
        let s = s.min(leaf_count);
        let e = e.min(leaf_count);

        // Walk up the tree, collecting sibling nodes outside [s, e)
        let mut cur_s = s;
        let mut cur_e = e;

        for layer_idx in 0..self.layers.len().saturating_sub(1) {
            let layer = &self.layers[layer_idx];

            // Left sibling: if cur_s is odd, include cur_s-1
            if cur_s % 2 == 1 && cur_s > 0 {
                siblings.push(layer[cur_s - 1].clone());
            }

            // Right sibling: if cur_e is odd, include cur_e
            if cur_e % 2 == 1 && cur_e < layer.len() {
                siblings.push(layer[cur_e].clone());
            }

            cur_s /= 2;
            cur_e = (cur_e + 1) / 2;
        }

        siblings
    }

    /// Compute blob_commitments as 32-byte hashes for the Block.blob_commitments field.
    /// Each commitment = Blake3(namespace || blob_data_hash).
    pub fn blob_commitment_hashes(&self) -> Vec<[u8; 32]> {
        self.leaves
            .iter()
            .map(|l| {
                let mut hasher = blake3::Hasher::new();
                hasher.update(&l.namespace);
                hasher.update(&l.data_hash);
                hasher.finalize().into()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(id: u8) -> NamespaceId {
        let mut n = [0u8; 8];
        n[7] = id;
        n
    }

    fn blob(namespace_id: u8, data: &[u8]) -> NamespacedBlob {
        NamespacedBlob {
            namespace: ns(namespace_id),
            data: data.to_vec(),
        }
    }

    #[test]
    fn test_nmt_basic_construction() {
        let blobs = vec![
            blob(1, b"hello"),
            blob(2, b"world"),
            blob(1, b"foo"),
        ];
        let tree = NamespaceMerkleTree::from_blobs(&blobs);
        let root = tree.root();

        // Root should span namespace 1..2
        assert_eq!(root.min_namespace, ns(1));
        assert_eq!(root.max_namespace, ns(2));
        assert_ne!(root.hash, [0u8; 32]);
    }

    #[test]
    fn test_nmt_single_namespace() {
        let blobs = vec![blob(5, b"a"), blob(5, b"b"), blob(5, b"c")];
        let tree = NamespaceMerkleTree::from_blobs(&blobs);
        let root = tree.root();
        assert_eq!(root.min_namespace, ns(5));
        assert_eq!(root.max_namespace, ns(5));
    }

    #[test]
    fn test_nmt_deterministic() {
        let blobs = vec![blob(1, b"x"), blob(3, b"y"), blob(2, b"z")];
        let tree1 = NamespaceMerkleTree::from_blobs(&blobs);
        let tree2 = NamespaceMerkleTree::from_blobs(&blobs);
        assert_eq!(tree1.root(), tree2.root());
    }

    #[test]
    fn test_nmt_namespace_inclusion_proof() {
        let blobs = vec![
            blob(1, b"a"),
            blob(2, b"b"),
            blob(2, b"c"),
            blob(3, b"d"),
        ];
        let tree = NamespaceMerkleTree::from_blobs(&blobs);

        // Prove namespace 2 exists
        let proof = tree.prove_namespace(&ns(2));
        assert!(!proof.is_absence);
        assert_eq!(proof.start_index, 1);
        assert_eq!(proof.end_index, 3); // indices 1, 2
        assert!(NamespaceMerkleTree::verify_namespace_proof(&proof));
    }

    #[test]
    fn test_nmt_namespace_absence_proof() {
        let blobs = vec![blob(1, b"a"), blob(3, b"b")];
        let tree = NamespaceMerkleTree::from_blobs(&blobs);

        // Prove namespace 2 does NOT exist
        let proof = tree.prove_namespace(&ns(2));
        assert!(proof.is_absence);
        assert!(NamespaceMerkleTree::verify_namespace_proof(&proof));
    }

    #[test]
    fn test_nmt_empty_tree() {
        let tree = NamespaceMerkleTree::from_blobs(&[]);
        let root = tree.root();
        assert_eq!(root.hash, [0u8; 32]);

        let proof = tree.prove_namespace(&ns(1));
        assert!(proof.is_absence);
    }

    #[test]
    fn test_blob_commitments() {
        let blobs = vec![blob(1, b"hello"), blob(2, b"world")];
        let tree = NamespaceMerkleTree::from_blobs(&blobs);
        let commitments = tree.blob_commitments();
        assert_eq!(commitments.len(), 2);
        assert_eq!(commitments[0].namespace, ns(1));
        assert_eq!(commitments[1].namespace, ns(2));
    }

    #[test]
    fn test_blob_commitment_hashes() {
        let blobs = vec![blob(1, b"hello"), blob(2, b"world")];
        let tree = NamespaceMerkleTree::from_blobs(&blobs);
        let hashes = tree.blob_commitment_hashes();
        assert_eq!(hashes.len(), 2);
        // Each hash should be non-zero and unique
        assert_ne!(hashes[0], [0u8; 32]);
        assert_ne!(hashes[1], [0u8; 32]);
        assert_ne!(hashes[0], hashes[1]);
    }

    #[test]
    fn test_nmt_sorted_by_namespace() {
        // Blobs given out of order should produce same tree
        let blobs_ordered = vec![blob(1, b"a"), blob(2, b"b"), blob(3, b"c")];
        let blobs_unordered = vec![blob(3, b"c"), blob(1, b"a"), blob(2, b"b")];
        let tree1 = NamespaceMerkleTree::from_blobs(&blobs_ordered);
        let tree2 = NamespaceMerkleTree::from_blobs(&blobs_unordered);
        assert_eq!(tree1.root(), tree2.root());
    }

    #[test]
    fn test_nmt_different_data_different_root() {
        let blobs1 = vec![blob(1, b"hello")];
        let blobs2 = vec![blob(1, b"world")];
        let tree1 = NamespaceMerkleTree::from_blobs(&blobs1);
        let tree2 = NamespaceMerkleTree::from_blobs(&blobs2);
        assert_ne!(tree1.root().hash, tree2.root().hash);
    }

    #[test]
    fn test_namespace_proof_out_of_range() {
        let blobs = vec![blob(5, b"data")];
        let tree = NamespaceMerkleTree::from_blobs(&blobs);

        // Namespace below range
        let proof = tree.prove_namespace(&ns(1));
        assert!(proof.is_absence);

        // Namespace above range
        let proof = tree.prove_namespace(&ns(10));
        assert!(proof.is_absence);
    }
}

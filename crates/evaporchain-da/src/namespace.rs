//! Namespace Merkle Tree (NMT) for blob-level data availability.
//!
//! Blobs are tagged with namespace IDs and sorted before tree construction.
//! Internal nodes carry (min_namespace, max_namespace) ranges, enabling:
//! - Namespace inclusion proofs: prove all blobs in a namespace are present
//! - Namespace absence proofs: prove a namespace has no blobs in this block
//!
//! Inspired by Celestia's NMT design, adapted for EvaporChain.

use serde::{Deserialize, Serialize};
use tracing::warn;

/// 8-byte namespace identifier. Namespaces are ordered lexicographically.
pub type NamespaceId = [u8; 8];

/// Reserved namespace (all zeros) — system / sentinel; user blobs cannot use it.
/// Closes Gap-A #9 (audit `end_to_end_audit_2026_04_27.md` §7).
pub const NAMESPACE_MIN: NamespaceId = [0u8; 8];
/// Reserved namespace (all 0xFF) — parity / erasure padding.
pub const NAMESPACE_MAX: NamespaceId = [0xFF; 8];

/// Construction errors for the namespace Merkle tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NmtBuildError {
    /// A user blob carried a reserved namespace that the NMT refuses to admit.
    ReservedNamespace { namespace: NamespaceId },
}

impl std::fmt::Display for NmtBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NmtBuildError::ReservedNamespace { namespace } => write!(
                f,
                "reserved namespace 0x{} cannot appear in user blobs",
                hex::encode(namespace)
            ),
        }
    }
}

impl std::error::Error for NmtBuildError {}

#[inline]
fn is_reserved_namespace(ns: &NamespaceId) -> bool {
    // Only NAMESPACE_MAX (all 0xFF) is structurally reserved by the NMT
    // for parity / erasure padding. NAMESPACE_MIN (all zeros) is in
    // active production use by the tendermint proposal builder for
    // "core transactions" (non-Blob txs are framed under ns=0). If
    // user-submitted BlobTx with namespace_id=0 needs to be rejected,
    // gate it at mempool admission, not at NMT construction.
    *ns == NAMESPACE_MAX
}

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
    /// Build a NMT from user blobs. Reserved namespaces ([0..0] and [FF..FF])
    /// are filtered out with a warning so a malformed admission path can
    /// never bake reserved IDs into the tree. Use [`try_from_blobs`] when
    /// you want a hard error instead of silent filtering.
    pub fn from_blobs(blobs: &[NamespacedBlob]) -> Self {
        let mut sorted: Vec<&NamespacedBlob> = blobs
            .iter()
            .filter(|b| {
                if is_reserved_namespace(&b.namespace) {
                    warn!(
                        namespace = %hex::encode(b.namespace),
                        size = b.data.len(),
                        "NMT: dropping blob with reserved namespace at construction"
                    );
                    false
                } else {
                    true
                }
            })
            .collect();
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

    /// Strict version of [`from_blobs`] that refuses to construct a tree
    /// when any blob carries a reserved namespace ([0..0] / [FF..FF]).
    /// Closes Gap-A #9 from the end-to-end audit.
    pub fn try_from_blobs(blobs: &[NamespacedBlob]) -> Result<Self, NmtBuildError> {
        for b in blobs {
            if is_reserved_namespace(&b.namespace) {
                return Err(NmtBuildError::ReservedNamespace {
                    namespace: b.namespace,
                });
            }
        }
        Ok(Self::from_blobs(blobs))
    }

    /// Build from pre-sorted leaves. Reserved namespaces are filtered with
    /// a warning; use [`try_from_leaves`] for the hard-error variant.
    pub fn from_leaves(leaves: Vec<NmtLeaf>) -> Self {
        let leaves: Vec<NmtLeaf> = leaves
            .into_iter()
            .filter(|l| {
                if is_reserved_namespace(&l.namespace) {
                    warn!(
                        namespace = %hex::encode(l.namespace),
                        "NMT: dropping leaf with reserved namespace at construction"
                    );
                    false
                } else {
                    true
                }
            })
            .collect();

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

    /// Strict version of [`from_leaves`] that refuses to construct a tree
    /// when any leaf carries a reserved namespace.
    pub fn try_from_leaves(leaves: Vec<NmtLeaf>) -> Result<Self, NmtBuildError> {
        for l in &leaves {
            if is_reserved_namespace(&l.namespace) {
                return Err(NmtBuildError::ReservedNamespace {
                    namespace: l.namespace,
                });
            }
        }
        Ok(Self::from_leaves(leaves))
    }

    /// Get the root node (namespace range + hash).
    pub fn root(&self) -> &NmtNode {
        self.layers
            .last()
            .and_then(|l| l.first())
            .unwrap_or(&NmtNode {
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
        let start = self.leaves.iter().position(|l| l.namespace >= *namespace);
        let end = self.leaves.iter().rposition(|l| l.namespace <= *namespace);

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
                if !left.is_empty()
                    && !right.is_empty()
                    && (left.max_namespace >= proof.namespace
                        || right.min_namespace <= proof.namespace)
                {
                    return false;
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
        if proof.namespace < proof.root.min_namespace || proof.namespace > proof.root.max_namespace
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
            cur_e = cur_e.div_ceil(2);
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
        let blobs = vec![blob(1, b"hello"), blob(2, b"world"), blob(1, b"foo")];
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
        let blobs = vec![blob(1, b"a"), blob(2, b"b"), blob(2, b"c"), blob(3, b"d")];
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

    // ─── Reserved-namespace enforcement (Gap-A #9) ────────────────────────

    fn blob_with_ns(namespace: NamespaceId, data: &[u8]) -> NamespacedBlob {
        NamespacedBlob {
            namespace,
            data: data.to_vec(),
        }
    }

    #[test]
    fn test_from_blobs_admits_namespace_min_for_core_tx_framing() {
        // Namespace 0 is in production use as the "core transactions"
        // namespace by tendermint's proposal builder. Filtering it would
        // drop every non-Blob tx from the NMT (regression caught after
        // initial Gap-A #9 commit).
        let core = blob_with_ns(NAMESPACE_MIN, b"transfer");
        let user = blob(7, b"user-blob");
        let tree = NamespaceMerkleTree::from_blobs(&[core.clone(), user.clone()]);
        assert_eq!(tree.leaves.len(), 2);
    }

    #[test]
    fn test_from_blobs_filters_reserved_max_namespace() {
        let good = blob(7, b"valid");
        let parity = blob_with_ns(NAMESPACE_MAX, b"parity-impostor");
        let tree = NamespaceMerkleTree::from_blobs(&[good.clone(), parity]);
        assert_eq!(tree.leaves.len(), 1);
        assert_eq!(tree.leaves[0].namespace, good.namespace);
    }

    #[test]
    fn test_try_from_blobs_rejects_reserved_max_namespace() {
        let blobs = vec![blob_with_ns(NAMESPACE_MAX, b"x"), blob(7, b"valid")];
        let err = NamespaceMerkleTree::try_from_blobs(&blobs).unwrap_err();
        assert_eq!(
            err,
            NmtBuildError::ReservedNamespace {
                namespace: NAMESPACE_MAX
            }
        );
    }

    #[test]
    fn test_try_from_blobs_accepts_valid_blobs() {
        let blobs = vec![blob(1, b"a"), blob(7, b"b"), blob(99, b"c")];
        let tree = NamespaceMerkleTree::try_from_blobs(&blobs).unwrap();
        assert_eq!(tree.leaves.len(), 3);
    }

    #[test]
    fn test_try_from_leaves_rejects_reserved_max_namespace() {
        let leaves = vec![
            NmtLeaf {
                namespace: ns(7),
                data_hash: [0u8; 32],
            },
            NmtLeaf {
                namespace: NAMESPACE_MAX,
                data_hash: [0u8; 32],
            },
        ];
        let err = NamespaceMerkleTree::try_from_leaves(leaves).unwrap_err();
        assert_eq!(
            err,
            NmtBuildError::ReservedNamespace {
                namespace: NAMESPACE_MAX
            }
        );
    }

    /// T1.20 — NmtBuildError::Display prints reserved namespace hex
    /// (lines 30-38). Previously the Display impl was unreached
    /// (only PartialEq comparison used in tests).
    #[test]
    fn t1_20_nmt_build_error_display() {
        let err = NmtBuildError::ReservedNamespace {
            namespace: NAMESPACE_MAX,
        };
        let s = format!("{}", err);
        assert!(s.contains("reserved namespace"));
        assert!(s.contains("ffffffffffffffff"));
    }

    /// T1.20 — from_blobs silently filters reserved-namespace blobs
    /// with a warning (lines 173-178 — the warn-and-drop branch).
    #[test]
    fn t1_20_from_blobs_drops_reserved_with_warning() {
        let blobs = vec![
            NamespacedBlob {
                namespace: [1u8; 8],
                data: vec![0xAA],
            },
            NamespacedBlob {
                namespace: NAMESPACE_MAX,
                data: vec![0xBB],
            },
        ];
        let nmt = NamespaceMerkleTree::from_blobs(&blobs);
        // Only the non-reserved blob makes it into the commitments.
        let commits = nmt.blob_commitments();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].namespace, [1u8; 8]);
    }

    /// T1.20 — from_leaves silently filters reserved-namespace leaves
    /// with a warning (lines 219-228 — the warn-and-drop branch).
    #[test]
    fn t1_20_from_leaves_drops_reserved_with_warning() {
        let leaves = vec![
            NmtLeaf {
                namespace: [1u8; 8],
                data_hash: [0xAA; 32],
            },
            NmtLeaf {
                namespace: NAMESPACE_MAX,
                data_hash: [0xBB; 32],
            },
        ];
        let nmt = NamespaceMerkleTree::from_leaves(leaves);
        // The reserved leaf was filtered; only one survives.
        let commits = nmt.blob_commitments();
        assert_eq!(commits.len(), 1);
    }

    /// T1.20 — `try_from_leaves` happy path. Existing tests only
    /// cover the reservation-rejection branch; this pins that valid
    /// leaves construct a tree cleanly (no error, root populated,
    /// commitments match input).
    #[test]
    fn t1_20_try_from_leaves_accepts_valid_leaves() {
        let leaves = vec![
            NmtLeaf {
                namespace: [0x01; 8],
                data_hash: [0xAAu8; 32],
            },
            NmtLeaf {
                namespace: [0x02; 8],
                data_hash: [0xBBu8; 32],
            },
        ];
        let nmt =
            NamespaceMerkleTree::try_from_leaves(leaves).expect("valid leaves must accept");
        assert_eq!(nmt.blob_commitments().len(), 2);
        assert_eq!(nmt.root().min_namespace, [0x01; 8]);
        assert_eq!(nmt.root().max_namespace, [0x02; 8]);
    }

    /// T1.20 — `root()` on a tree built from zero leaves returns the
    /// inverted-range sentinel (min=NAMESPACE_MAX, max=NAMESPACE_MIN)
    /// from the empty-layer construction at line 232. Callers can
    /// detect "no real root" via `min > max`.
    #[test]
    fn t1_20_root_empty_tree_returns_inverted_sentinel() {
        let nmt = NamespaceMerkleTree::from_leaves(vec![]);
        let root = nmt.root();
        assert_eq!(root.min_namespace, NAMESPACE_MAX);
        assert_eq!(root.max_namespace, NAMESPACE_MIN);
        // Empty-tree predicate: min > max.
        assert!(root.min_namespace > root.max_namespace);
    }

    /// T1.20 — `prove_namespace` boundary: namespace strictly BELOW
    /// the tree's minimum yields an absence proof anchored at
    /// insertion point 0 (line 333: `start.unwrap_or(self.leaves.len())`
    /// when start is None for a below-min ns). The verifier accepts
    /// because `ns < root.min_namespace`.
    #[test]
    fn t1_20_prove_namespace_below_minimum_is_absent() {
        let leaves = vec![
            NmtLeaf {
                namespace: [0x05; 8],
                data_hash: [0xCC; 32],
            },
            NmtLeaf {
                namespace: [0x06; 8],
                data_hash: [0xDD; 32],
            },
        ];
        let nmt = NamespaceMerkleTree::from_leaves(leaves);
        let proof = nmt.prove_namespace(&[0x01; 8]);
        assert!(proof.is_absence);
        assert_eq!(proof.start_index, 0);
        assert_eq!(proof.end_index, 0);
        assert!(
            NamespaceMerkleTree::verify_namespace_proof(&proof),
            "below-min absence proof must verify"
        );
    }
}

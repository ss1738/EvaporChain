//! Merkle Mountain Range (MMR) accumulator for energy-stamped nullifiers.
//!
//! When an object evaporates, its nullifier encodes the object's entire lifecycle:
//! `nullifier = hash(object_id || value_hash || evaporation_epoch || energy_at_death || owner)`
//!
//! The MMR is an append-only structure of perfect binary Merkle trees (peaks).
//! It provides O(log n) inclusion proofs and a single root hash computed by
//! "bagging the peaks" — hashing all peak hashes together from right to left.

use crate::hash::blake3_hash;
use serde::{Deserialize, Serialize};

// ─────────────────────── EnergyStampedNullifier ──────────────────────────

/// A nullifier that cryptographically encodes an object's full lifecycle.
///
/// This is the innovation: the nullifier itself is proof of the object's
/// creation, content, death, energy state, and ownership — all in 32 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnergyStampedNullifier {
    pub object_id: [u8; 32],
    pub value_hash: [u8; 32],
    pub evaporation_epoch: u64,
    pub energy_at_death: u64,
    pub owner: [u8; 32],
}

impl EnergyStampedNullifier {
    /// Compute the 32-byte nullifier hash.
    /// This is what gets appended to the MMR.
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut data = Vec::with_capacity(32 + 32 + 8 + 8 + 32);
        data.extend_from_slice(&self.object_id);
        data.extend_from_slice(&self.value_hash);
        data.extend_from_slice(&self.evaporation_epoch.to_le_bytes());
        data.extend_from_slice(&self.energy_at_death.to_le_bytes());
        data.extend_from_slice(&self.owner);
        blake3_hash(&data)
    }
}

// ─────────────────────── MMR Position ────────────────────────────────────

/// Position of a leaf in the MMR, used to generate proofs later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MMRPosition {
    /// Zero-based index of the leaf among all leaves appended.
    pub leaf_index: u64,
    /// Total MMR size (number of nodes, including internal) at time of insertion.
    pub mmr_size: u64,
}

// ─────────────────────── MMR Proof ───────────────────────────────────────

/// Inclusion proof for a leaf in the MMR.
/// Contains the sibling hashes along the path from leaf to its peak,
/// plus the peak-bagging path from that peak to the root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MMRProof {
    /// The leaf index being proved.
    pub leaf_index: u64,
    /// MMR size at proof generation time.
    pub mmr_size: u64,
    /// Sibling hashes from leaf up to the peak.
    pub siblings: Vec<[u8; 32]>,
    /// Peak hashes for bagging (excluding the peak this leaf belongs to).
    pub peak_hashes: Vec<[u8; 32]>,
    /// Index of the peak this leaf belongs to (among all peaks).
    pub peak_index: usize,
}

// ─────────────────────── Merkle Mountain Range ───────────────────────────

/// Append-only Merkle Mountain Range accumulator.
///
/// Internally stores all nodes (leaves + internal) in a flat array.
/// Peaks are the roots of the perfect binary trees that compose the MMR.
pub struct MerkleMountainRange {
    /// All nodes in insertion order. Leaves are hashes of nullifiers;
    /// internal nodes are hash(left || right).
    nodes: Vec<[u8; 32]>,
    /// Number of leaves (nullifiers) appended.
    leaf_count: u64,
}

impl MerkleMountainRange {
    /// Create a new empty MMR.
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            leaf_count: 0,
        }
    }

    /// Append a nullifier (as a 32-byte hash) and return its position.
    pub fn append(&mut self, nullifier_hash: [u8; 32]) -> MMRPosition {
        let leaf_index = self.leaf_count;

        // Push the leaf
        self.nodes.push(nullifier_hash);
        let mut pos = self.nodes.len() - 1;
        let mut height = 0u32;

        // Merge with left siblings as long as we complete a pair at this height.
        // A leaf at leaf_index has a left sibling at the same height if
        // bit `height` of `leaf_index` is set (when counting from 0).
        let mut current = nullifier_hash;
        while (leaf_index >> height) & 1 == 1 {
            // The left sibling is the node just before the subtree we built.
            // Its position: pos - (2 << height) + 1, but simpler:
            // it's the node at pos - size_of_right_subtree - 1 ... - size_of_left_subtree
            let left_size = (1u64 << (height + 1)) - 1;
            let left_pos = pos as u64 - left_size;
            let left_hash = self.nodes[left_pos as usize];

            current = hash_pair(&left_hash, &current);
            self.nodes.push(current);
            pos = self.nodes.len() - 1;
            height += 1;
        }

        self.leaf_count += 1;

        MMRPosition {
            leaf_index,
            mmr_size: self.nodes.len() as u64,
        }
    }

    /// Compute the MMR root by bagging the peaks (hashing right-to-left).
    pub fn root(&self) -> [u8; 32] {
        let peaks = self.peak_hashes();
        bag_peaks(&peaks)
    }

    /// Total number of nullifiers (leaves) in the MMR.
    pub fn size(&self) -> usize {
        self.leaf_count as usize
    }

    /// Generate an inclusion proof for a leaf at the given index.
    pub fn prove(&self, leaf_index: u64) -> Option<MMRProof> {
        if leaf_index >= self.leaf_count {
            return None;
        }

        // Find the node position of this leaf in the flat array
        let node_pos = leaf_index_to_node_pos(leaf_index);
        if node_pos >= self.nodes.len() as u64 {
            return None;
        }

        // Collect siblings up to the peak
        let mut siblings = Vec::new();
        let mut pos = node_pos;
        let mut height = 0u32;

        loop {
            let sibling_pos;
            let parent_pos;

            // Determine if we are a left or right child
            let subtree_size = (1u64 << (height + 1)) - 1;

            // Check if right sibling exists at pos + subtree_size + 1
            let _right_sib = pos + subtree_size + 1;
            // Check if left sibling exists at pos - subtree_size - 1
            // To determine: is there a parent merging us?
            // We are a left child if the node at (pos + subtree_size + 1) is at height `height`
            // and the parent at (pos + subtree_size + 1 + ... ) merges them.
            //
            // Simpler approach: use leaf_index bits.
            // At height h, if bit h of leaf_index is 0, we are a left child.
            if (leaf_index >> height) & 1 == 0 {
                // Left child — sibling is to the right
                sibling_pos = pos + subtree_size;
                parent_pos = sibling_pos + 1;
            } else {
                // Right child — sibling is to the left
                sibling_pos = pos - subtree_size;
                parent_pos = pos + 1;
            }

            // Check if parent exists (i.e., the merge happened)
            if parent_pos >= self.nodes.len() as u64 {
                // We've reached the peak
                break;
            }

            if sibling_pos >= self.nodes.len() as u64 {
                break;
            }

            siblings.push(self.nodes[sibling_pos as usize]);
            pos = parent_pos;
            height += 1;
        }

        // Collect peak hashes and find which peak we belong to
        let all_peaks = self.peak_positions();
        let mut peak_hashes = Vec::new();
        let mut peak_index = 0;

        for (i, &peak_pos) in all_peaks.iter().enumerate() {
            if peak_pos == pos {
                peak_index = i;
            } else {
                peak_hashes.push(self.nodes[peak_pos as usize]);
            }
        }

        Some(MMRProof {
            leaf_index,
            mmr_size: self.nodes.len() as u64,
            siblings,
            peak_hashes,
            peak_index,
        })
    }

    /// Verify an inclusion proof for a nullifier hash against a root.
    pub fn verify(proof: &MMRProof, nullifier_hash: &[u8; 32], expected_root: &[u8; 32]) -> bool {
        // Rebuild from leaf to peak using siblings
        let mut current = *nullifier_hash;
        let mut height = 0u32;

        for sibling in &proof.siblings {
            if (proof.leaf_index >> height) & 1 == 0 {
                // We are left child
                current = hash_pair(&current, sibling);
            } else {
                // We are right child
                current = hash_pair(sibling, &current);
            }
            height += 1;
        }

        // Now `current` is our peak hash. Bag all peaks together.
        // Insert our peak at the correct index among the other peaks.
        let mut all_peaks = Vec::with_capacity(proof.peak_hashes.len() + 1);
        let mut other_idx = 0;
        for i in 0..=proof.peak_hashes.len() {
            if i == proof.peak_index {
                all_peaks.push(current);
            } else {
                if other_idx >= proof.peak_hashes.len() {
                    continue;
                }
                all_peaks.push(proof.peak_hashes[other_idx]);
                other_idx += 1;
            }
        }

        let computed_root = bag_peaks(&all_peaks);
        computed_root == *expected_root
    }

    /// Return the hash at each peak position.
    fn peak_hashes(&self) -> Vec<[u8; 32]> {
        self.peak_positions()
            .iter()
            .map(|&pos| self.nodes[pos as usize])
            .collect()
    }

    /// Compute the node positions of all peaks.
    ///
    /// Peaks correspond to the set bits in `leaf_count`.
    /// A peak at height h has (2^(h+1) - 1) nodes beneath it.
    fn peak_positions(&self) -> Vec<u64> {
        let mut peaks = Vec::new();
        let mut remaining = self.leaf_count;
        let mut node_offset = 0u64;

        // Walk from the highest set bit to the lowest
        let mut height = 63 - self.leaf_count.max(1).leading_zeros();

        loop {
            let subtree_leaves = 1u64 << height;
            if remaining >= subtree_leaves {
                // There's a peak of this height
                let subtree_nodes = (subtree_leaves << 1) - 1; // 2^(h+1) - 1
                let peak_pos = node_offset + subtree_nodes - 1;
                peaks.push(peak_pos);
                node_offset += subtree_nodes;
                remaining -= subtree_leaves;
            }
            if height == 0 {
                break;
            }
            height -= 1;
        }

        peaks
    }
}

impl Default for MerkleMountainRange {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────── Helper Functions ─────────────────────────────────

/// Hash two 32-byte values: H(left || right). NOT canonically ordered —
/// MMR uses positional ordering (left child always first).
fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut combined = [0u8; 64];
    combined[..32].copy_from_slice(left);
    combined[32..].copy_from_slice(right);
    blake3_hash(&combined)
}

/// Bag peaks right-to-left: fold from the rightmost peak.
/// root = H(peak[0] || H(peak[1] || H(... || peak[n-1])))
fn bag_peaks(peaks: &[[u8; 32]]) -> [u8; 32] {
    if peaks.is_empty() {
        return [0u8; 32];
    }
    if peaks.len() == 1 {
        return peaks[0];
    }

    let mut root = peaks[peaks.len() - 1];
    for i in (0..peaks.len() - 1).rev() {
        root = hash_pair(&peaks[i], &root);
    }
    root
}

/// Convert a leaf index (0-based among leaves) to its node position
/// in the flat MMR array.
///
/// Leaf i is at position: i + number_of_internal_nodes_before_leaf_i.
/// The number of internal nodes before leaf i equals the number of
/// trailing 1-bits consumed by merges of leaves 0..i-1.
fn leaf_index_to_node_pos(leaf_index: u64) -> u64 {
    // Each leaf at index i occupies position i * 2 - popcount(i) in the
    // "perfect" mapping. But it's simpler to compute iteratively:
    // For leaf 0: pos=0, leaf 1: pos=1, after merge pos=2 is internal,
    // leaf 2: pos=3, leaf 3: pos=4, merge→5, merge→6, leaf 4: pos=7 ...
    //
    // The node position of leaf i = 2*i - popcount(i)
    2 * leaf_index - leaf_index.count_ones() as u64
}

// ─────────────────── Legacy trait (kept for API compat) ──────────────────

/// Cryptographic accumulator trait.
pub trait Accumulator: Send + Sync {
    fn add(&mut self, element: &[u8]) -> [u8; 32];
    fn remove(&mut self, element: &[u8]) -> [u8; 32];
    fn prove_membership(&self, element: &[u8]) -> Option<MembershipProof>;
    fn verify_membership(&self, proof: &MembershipProof, element: &[u8]) -> bool;
    fn prove_non_membership(&self, element: &[u8]) -> Option<NonMembershipProof>;
    fn verify_non_membership(&self, proof: &NonMembershipProof, element: &[u8]) -> bool;
    fn digest(&self) -> [u8; 32];
}

/// Membership proof (legacy, kept for trait compat).
#[derive(Debug, Clone, PartialEq)]
pub struct MembershipProof {
    pub path: Vec<Option<[u8; 32]>>,
    pub element_hash: [u8; 32],
}

/// Non-membership proof (legacy, kept for trait compat).
#[derive(Debug, Clone, PartialEq)]
pub struct NonMembershipProof {
    pub left_neighbor: Option<[u8; 32]>,
    pub right_neighbor: Option<[u8; 32]>,
    pub digest: [u8; 32],
}

/// In-memory accumulator backed by the MMR.
/// Wraps the legacy Accumulator trait around the real MMR.
pub struct InMemoryAccumulator {
    mmr: MerkleMountainRange,
    /// Track elements for membership queries (MMR is append-only).
    elements: std::collections::HashMap<[u8; 32], u64>, // hash -> leaf_index
}

impl InMemoryAccumulator {
    pub fn new() -> Self {
        Self {
            mmr: MerkleMountainRange::new(),
            elements: std::collections::HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
}

impl Default for InMemoryAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl Accumulator for InMemoryAccumulator {
    fn add(&mut self, element: &[u8]) -> [u8; 32] {
        let hash = blake3_hash(element);
        if !self.elements.contains_key(&hash) {
            let pos = self.mmr.append(hash);
            self.elements.insert(hash, pos.leaf_index);
        }
        self.mmr.root()
    }

    fn remove(&mut self, element: &[u8]) -> [u8; 32] {
        let hash = blake3_hash(element);
        // MMR is append-only — we can't truly remove. We just remove from
        // the element tracking map so membership queries fail.
        self.elements.remove(&hash);
        self.mmr.root()
    }

    fn prove_membership(&self, element: &[u8]) -> Option<MembershipProof> {
        let hash = blake3_hash(element);
        let &leaf_index = self.elements.get(&hash)?;
        let mmr_proof = self.mmr.prove(leaf_index)?;

        // Convert MMR proof to legacy MembershipProof format
        let mut path: Vec<Option<[u8; 32]>> = mmr_proof
            .siblings
            .iter()
            .map(|s| Some(*s))
            .collect();
        // Append peak hashes for complete verification
        for ph in &mmr_proof.peak_hashes {
            path.push(Some(*ph));
        }

        Some(MembershipProof {
            path,
            element_hash: hash,
        })
    }

    fn verify_membership(&self, proof: &MembershipProof, element: &[u8]) -> bool {
        let hash = blake3_hash(element);
        if hash != proof.element_hash {
            return false;
        }
        self.elements.contains_key(&hash)
    }

    fn prove_non_membership(&self, element: &[u8]) -> Option<NonMembershipProof> {
        let hash = blake3_hash(element);
        if self.elements.contains_key(&hash) {
            return None;
        }
        Some(NonMembershipProof {
            left_neighbor: None,
            right_neighbor: None,
            digest: self.mmr.root(),
        })
    }

    fn verify_non_membership(&self, proof: &NonMembershipProof, element: &[u8]) -> bool {
        let hash = blake3_hash(element);
        !self.elements.contains_key(&hash) && proof.digest == self.mmr.root()
    }

    fn digest(&self) -> [u8; 32] {
        self.mmr.root()
    }
}

// ─────────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Legacy trait tests (backward compat) ──

    #[test]
    fn test_add_and_membership() {
        let mut acc = InMemoryAccumulator::new();
        acc.add(b"object_1");
        acc.add(b"object_2");

        assert_eq!(acc.len(), 2);
        assert!(acc.prove_membership(b"object_1").is_some());
        assert!(acc.prove_membership(b"object_2").is_some());
        assert!(acc.prove_membership(b"object_3").is_none());
    }

    #[test]
    fn test_verify_membership_proof() {
        let mut acc = InMemoryAccumulator::new();
        acc.add(b"ghost_A");
        acc.add(b"ghost_B");
        acc.add(b"ghost_C");

        let proof = acc.prove_membership(b"ghost_B").unwrap();
        assert!(acc.verify_membership(&proof, b"ghost_B"));
        assert!(!acc.verify_membership(&proof, b"ghost_A"));
    }

    #[test]
    fn test_remove_invalidates_proof() {
        let mut acc = InMemoryAccumulator::new();
        acc.add(b"ephemeral");

        let proof = acc.prove_membership(b"ephemeral").unwrap();
        assert!(acc.verify_membership(&proof, b"ephemeral"));

        acc.remove(b"ephemeral");
        assert!(acc.prove_membership(b"ephemeral").is_none());
        assert_eq!(acc.len(), 0);
    }

    #[test]
    fn test_non_membership_proof() {
        let mut acc = InMemoryAccumulator::new();
        acc.add(b"alpha");
        acc.add(b"gamma");

        let proof = acc.prove_non_membership(b"beta").unwrap();
        assert!(acc.verify_non_membership(&proof, b"beta"));
        assert!(acc.prove_non_membership(b"alpha").is_none());
    }

    #[test]
    fn test_digest_changes() {
        let mut acc = InMemoryAccumulator::new();
        let d0 = acc.digest();

        acc.add(b"first");
        let d1 = acc.digest();
        assert_ne!(d0, d1);

        acc.add(b"second");
        let d2 = acc.digest();
        assert_ne!(d1, d2);
    }

    #[test]
    fn test_digest_deterministic() {
        let mut acc1 = InMemoryAccumulator::new();
        let mut acc2 = InMemoryAccumulator::new();

        // Same elements in same order → same digest (MMR is ordered)
        acc1.add(b"X");
        acc1.add(b"Y");

        acc2.add(b"X");
        acc2.add(b"Y");

        assert_eq!(acc1.digest(), acc2.digest());
    }

    #[test]
    fn test_empty_accumulator() {
        let acc = InMemoryAccumulator::new();
        assert!(acc.is_empty());
        assert_eq!(acc.digest(), [0u8; 32]);
    }

    #[test]
    fn test_single_element() {
        let mut acc = InMemoryAccumulator::new();
        acc.add(b"solo");

        let proof = acc.prove_membership(b"solo").unwrap();
        assert!(acc.verify_membership(&proof, b"solo"));
    }

    #[test]
    fn test_accumulator_trait_object() {
        let mut acc = InMemoryAccumulator::new();
        let trait_obj: &mut dyn Accumulator = &mut acc;

        trait_obj.add(b"element");
        assert!(trait_obj.prove_membership(b"element").is_some());
        assert_ne!(trait_obj.digest(), [0u8; 32]);
    }

    // ── MMR-specific tests ──

    #[test]
    fn test_mmr_append_single_verify_proof() {
        let mut mmr = MerkleMountainRange::new();
        let hash = blake3_hash(b"nullifier_0");
        let pos = mmr.append(hash);

        assert_eq!(pos.leaf_index, 0);
        assert_eq!(mmr.size(), 1);

        let root = mmr.root();
        let proof = mmr.prove(0).unwrap();
        assert!(MerkleMountainRange::verify(&proof, &hash, &root));
    }

    #[test]
    fn test_mmr_append_1000_verify_random_proofs() {
        let mut mmr = MerkleMountainRange::new();
        let mut hashes = Vec::new();

        for i in 0u16..1000 {
            let h = blake3_hash(&i.to_le_bytes());
            mmr.append(h);
            hashes.push(h);
        }

        assert_eq!(mmr.size(), 1000);
        let root = mmr.root();

        // Verify proofs for a selection of indices
        for &idx in &[0u64, 1, 2, 7, 42, 100, 255, 500, 777, 999] {
            let proof = mmr.prove(idx).unwrap();
            assert!(
                MerkleMountainRange::verify(&proof, &hashes[idx as usize], &root),
                "Proof failed for leaf index {}",
                idx
            );
        }
    }

    #[test]
    fn test_mmr_root_changes_on_every_append() {
        let mut mmr = MerkleMountainRange::new();
        let mut prev_root = mmr.root();

        for i in 0u8..20 {
            mmr.append(blake3_hash(&[i]));
            let new_root = mmr.root();
            assert_ne!(prev_root, new_root, "Root did not change after append {}", i);
            prev_root = new_root;
        }
    }

    #[test]
    fn test_mmr_root_deterministic() {
        let mut mmr1 = MerkleMountainRange::new();
        let mut mmr2 = MerkleMountainRange::new();

        for i in 0u8..10 {
            let h = blake3_hash(&[i]);
            mmr1.append(h);
            mmr2.append(h);
        }

        assert_eq!(mmr1.root(), mmr2.root());
    }

    #[test]
    fn test_mmr_proof_fails_against_wrong_root() {
        let mut mmr = MerkleMountainRange::new();
        let hash = blake3_hash(b"test");
        mmr.append(hash);

        let proof = mmr.prove(0).unwrap();
        let wrong_root = [0xFFu8; 32];
        assert!(!MerkleMountainRange::verify(&proof, &hash, &wrong_root));
    }

    #[test]
    fn test_mmr_proof_fails_for_wrong_nullifier() {
        let mut mmr = MerkleMountainRange::new();
        let real_hash = blake3_hash(b"real");
        let fake_hash = blake3_hash(b"fake");
        mmr.append(real_hash);

        let root = mmr.root();
        let proof = mmr.prove(0).unwrap();

        // Proof should verify with real hash
        assert!(MerkleMountainRange::verify(&proof, &real_hash, &root));
        // Proof should fail with fake hash
        assert!(!MerkleMountainRange::verify(&proof, &fake_hash, &root));
    }

    #[test]
    fn test_energy_stamped_nullifier_serialization_roundtrip() {
        let nullifier = EnergyStampedNullifier {
            object_id: [1u8; 32],
            value_hash: [2u8; 32],
            evaporation_epoch: 42,
            energy_at_death: 0,
            owner: [3u8; 32],
        };

        let bytes = nullifier.to_bytes();
        assert_ne!(bytes, [0u8; 32]);

        // Same inputs produce same hash
        let nullifier2 = EnergyStampedNullifier {
            object_id: [1u8; 32],
            value_hash: [2u8; 32],
            evaporation_epoch: 42,
            energy_at_death: 0,
            owner: [3u8; 32],
        };
        assert_eq!(nullifier.to_bytes(), nullifier2.to_bytes());

        // Different inputs produce different hash
        let nullifier3 = EnergyStampedNullifier {
            object_id: [1u8; 32],
            value_hash: [2u8; 32],
            evaporation_epoch: 43, // different epoch
            energy_at_death: 0,
            owner: [3u8; 32],
        };
        assert_ne!(nullifier.to_bytes(), nullifier3.to_bytes());
    }

    #[test]
    fn test_energy_stamped_nullifier_in_mmr() {
        let mut mmr = MerkleMountainRange::new();

        let nullifier = EnergyStampedNullifier {
            object_id: [0xAA; 32],
            value_hash: blake3_hash(b"some data"),
            evaporation_epoch: 100,
            energy_at_death: 0,
            owner: [0xBB; 32],
        };

        let hash = nullifier.to_bytes();
        let pos = mmr.append(hash);
        assert_eq!(pos.leaf_index, 0);

        let root = mmr.root();
        let proof = mmr.prove(0).unwrap();
        assert!(MerkleMountainRange::verify(&proof, &hash, &root));
    }

    #[test]
    fn test_mmr_empty_has_valid_root() {
        let mmr = MerkleMountainRange::new();
        assert_eq!(mmr.root(), [0u8; 32]);
        assert_eq!(mmr.size(), 0);
    }

    #[test]
    fn test_mmr_size_tracks_correctly() {
        let mut mmr = MerkleMountainRange::new();
        for i in 0u8..50 {
            mmr.append(blake3_hash(&[i]));
            assert_eq!(mmr.size(), (i + 1) as usize);
        }
    }

    #[test]
    fn test_mmr_prove_out_of_bounds() {
        let mut mmr = MerkleMountainRange::new();
        mmr.append(blake3_hash(b"only_one"));
        assert!(mmr.prove(0).is_some());
        assert!(mmr.prove(1).is_none());
        assert!(mmr.prove(999).is_none());
    }

    #[test]
    fn test_mmr_multiple_peaks() {
        // 5 leaves = peaks at heights 2 and 0 (binary 101)
        let mut mmr = MerkleMountainRange::new();
        for i in 0u8..5 {
            mmr.append(blake3_hash(&[i]));
        }
        let root = mmr.root();
        assert_ne!(root, [0u8; 32]);

        // All 5 leaves should be provable
        for i in 0u64..5 {
            let h = blake3_hash(&[i as u8]);
            let proof = mmr.prove(i).unwrap();
            assert!(
                MerkleMountainRange::verify(&proof, &h, &root),
                "Failed for leaf {}",
                i
            );
        }
    }

    #[test]
    fn test_mmr_power_of_two_leaves() {
        // 8 leaves = single peak (perfect binary tree)
        let mut mmr = MerkleMountainRange::new();
        let mut hashes = Vec::new();
        for i in 0u8..8 {
            let h = blake3_hash(&[i]);
            mmr.append(h);
            hashes.push(h);
        }
        let root = mmr.root();

        for i in 0u64..8 {
            let proof = mmr.prove(i).unwrap();
            assert!(MerkleMountainRange::verify(
                &proof,
                &hashes[i as usize],
                &root
            ));
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Property-Based Tests (Audit Hardening)
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::hash::blake3_hash;
    use proptest::prelude::*;

    proptest! {
        /// Every appended leaf is provable against the current root.
        #[test]
        fn all_leaves_provable(count in 1u64..50) {
            let mut mmr = MerkleMountainRange::new();
            let mut hashes = Vec::new();
            for i in 0..count {
                let h = blake3_hash(&i.to_le_bytes());
                mmr.append(h);
                hashes.push(h);
            }
            let root = mmr.root();
            for i in 0..count {
                let proof = mmr.prove(i).ok_or_else(|| TestCaseError::fail("prove returned None"))?;
                prop_assert!(
                    MerkleMountainRange::verify(&proof, &hashes[i as usize], &root),
                    "proof failed for leaf {} of {}",
                    i,
                    count
                );
            }
        }

        /// Appending a new leaf always changes the root.
        #[test]
        fn append_changes_root(count in 1u64..30) {
            let mut mmr = MerkleMountainRange::new();
            let mut prev_root = mmr.root();
            for i in 0..count {
                let h = blake3_hash(&i.to_le_bytes());
                mmr.append(h);
                let new_root = mmr.root();
                // After first append, root should differ from empty
                // After each subsequent append, root should change
                if i > 0 || prev_root != [0u8; 32] {
                    prop_assert_ne!(new_root, prev_root, "root unchanged after append {}", i);
                }
                prev_root = new_root;
            }
        }

        /// Root is deterministic: same sequence of appends gives same root.
        #[test]
        fn root_deterministic(count in 1u64..30) {
            let mut mmr1 = MerkleMountainRange::new();
            let mut mmr2 = MerkleMountainRange::new();
            for i in 0..count {
                let h = blake3_hash(&i.to_le_bytes());
                mmr1.append(h);
                mmr2.append(h);
            }
            prop_assert_eq!(mmr1.root(), mmr2.root());
        }

        /// A proof for the wrong leaf data fails verification.
        #[test]
        fn wrong_data_fails_verification(count in 2u64..20, leaf_idx in 0u64..20) {
            let leaf_idx = leaf_idx % count;
            let mut mmr = MerkleMountainRange::new();
            for i in 0..count {
                mmr.append(blake3_hash(&i.to_le_bytes()));
            }
            let root = mmr.root();
            let proof = mmr.prove(leaf_idx).ok_or_else(|| TestCaseError::fail("prove returned None"))?;
            let wrong_data = blake3_hash(b"wrong");
            prop_assert!(!MerkleMountainRange::verify(&proof, &wrong_data, &root));
        }

        /// EnergyStampedNullifier serialization is deterministic.
        #[test]
        fn nullifier_deterministic(
            epoch in any::<u64>(),
            energy in any::<u64>(),
            id_byte in any::<u8>(),
        ) {
            let mut obj_id = [0u8; 32];
            obj_id[0] = id_byte;
            let nullifier = EnergyStampedNullifier {
                object_id: obj_id,
                value_hash: blake3_hash(&[id_byte]),
                evaporation_epoch: epoch,
                energy_at_death: energy,
                owner: [0u8; 32],
            };
            let bytes1 = nullifier.to_bytes();
            let bytes2 = nullifier.to_bytes();
            prop_assert_eq!(bytes1, bytes2);
            prop_assert_eq!(bytes1.len(), 32);
        }
    }
}

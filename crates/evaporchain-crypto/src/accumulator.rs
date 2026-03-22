use crate::hash::blake3_hash;
use std::collections::HashSet;

// ─────────────────────────── Accumulator Trait ───────────────────────────

/// Cryptographic accumulator for compact set membership proofs.
///
/// Used by the EvaporChain state engine to prove that evaporated objects
/// (ghosts) existed without storing the full object data on-chain.
///
/// Current implementation: `InMemoryAccumulator` using HashSet + BLAKE3
/// Merkle proofs as a functional placeholder.
///
/// **Next phase**: Replace with an RSA accumulator (using hidden-order groups)
/// or class-group accumulator for constant-size proofs without a trusted setup.
/// RSA accumulators support both membership and non-membership proofs with
/// O(1) proof size, making them ideal for ghost record verification.
pub trait Accumulator: Send + Sync {
    /// Add an element to the accumulator. Returns the new accumulator digest.
    fn add(&mut self, element: &[u8]) -> [u8; 32];

    /// Remove an element from the accumulator. Returns the new accumulator digest.
    fn remove(&mut self, element: &[u8]) -> [u8; 32];

    /// Generate a membership proof for an element.
    fn prove_membership(&self, element: &[u8]) -> Option<MembershipProof>;

    /// Verify a membership proof against the current accumulator state.
    fn verify_membership(&self, proof: &MembershipProof, element: &[u8]) -> bool;

    /// Generate a non-membership proof for an element.
    fn prove_non_membership(&self, element: &[u8]) -> Option<NonMembershipProof>;

    /// Verify a non-membership proof.
    fn verify_non_membership(&self, proof: &NonMembershipProof, element: &[u8]) -> bool;

    /// Current accumulator digest (commitment to the entire set).
    fn digest(&self) -> [u8; 32];
}

/// Proof that an element is in the accumulated set.
#[derive(Debug, Clone, PartialEq)]
pub struct MembershipProof {
    /// Merkle path from element to root. `None` means the node was an odd
    /// element promoted without a sibling (no hashing needed at that level).
    /// In RSA accumulator: this would be the witness w such that w^e = A mod N.
    pub path: Vec<Option<[u8; 32]>>,
    pub element_hash: [u8; 32],
}

/// Proof that an element is NOT in the accumulated set.
#[derive(Debug, Clone, PartialEq)]
pub struct NonMembershipProof {
    /// In RSA accumulator: Bezout coefficients (a, b) such that a*e + b*product = 1.
    /// Placeholder uses adjacent-element proof in sorted set.
    pub left_neighbor: Option<[u8; 32]>,
    pub right_neighbor: Option<[u8; 32]>,
    pub digest: [u8; 32],
}

// ─────────────────── InMemoryAccumulator (placeholder) ───────────────────

/// Hash-set-backed accumulator with BLAKE3 Merkle proofs.
///
/// This is a functional placeholder that provides the correct interface
/// for integration with the state engine and ghost records. It is NOT
/// a cryptographic accumulator — the "proofs" are only valid when
/// verified against this instance's internal state.
///
/// Will be replaced with RSA accumulator or class-group accumulator.
pub struct InMemoryAccumulator {
    elements: HashSet<[u8; 32]>,
    /// Sorted element hashes for deterministic digest and Merkle tree.
    sorted_cache: Vec<[u8; 32]>,
    cached_digest: [u8; 32],
    dirty: bool,
}

impl InMemoryAccumulator {
    pub fn new() -> Self {
        Self {
            elements: HashSet::new(),
            sorted_cache: Vec::new(),
            cached_digest: [0u8; 32],
            dirty: false,
        }
    }

    /// Number of elements in the accumulator.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Recompute the Merkle root from sorted elements.
    fn recompute(&mut self) {
        if !self.dirty {
            return;
        }
        self.sorted_cache = self.elements.iter().copied().collect();
        self.sorted_cache.sort();
        self.cached_digest = merkle_root(&self.sorted_cache);
        self.dirty = false;
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
        self.elements.insert(hash);
        self.dirty = true;
        self.recompute();
        self.cached_digest
    }

    fn remove(&mut self, element: &[u8]) -> [u8; 32] {
        let hash = blake3_hash(element);
        self.elements.remove(&hash);
        self.dirty = true;
        self.recompute();
        self.cached_digest
    }

    fn prove_membership(&self, element: &[u8]) -> Option<MembershipProof> {
        let hash = blake3_hash(element);
        if !self.elements.contains(&hash) {
            return None;
        }

        // Build Merkle path (simplified: just include sibling hashes)
        let path = merkle_path(&self.sorted_cache, &hash);
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
        // Verify by recomputing root from proof path
        let computed_root = compute_root_from_proof(&hash, &proof.path);
        computed_root == self.cached_digest
    }

    fn prove_non_membership(&self, element: &[u8]) -> Option<NonMembershipProof> {
        let hash = blake3_hash(element);
        if self.elements.contains(&hash) {
            return None; // Element IS a member, can't prove non-membership
        }

        // Find neighbors in sorted set
        let (left, right) = find_neighbors(&self.sorted_cache, &hash);

        Some(NonMembershipProof {
            left_neighbor: left,
            right_neighbor: right,
            digest: self.cached_digest,
        })
    }

    fn verify_non_membership(&self, proof: &NonMembershipProof, element: &[u8]) -> bool {
        let hash = blake3_hash(element);
        // Element must not be a member
        if self.elements.contains(&hash) {
            return false;
        }
        // Digest must match
        proof.digest == self.cached_digest
    }

    fn digest(&self) -> [u8; 32] {
        self.cached_digest
    }
}

// ─────────────────── Merkle tree helpers ─────────────────────────────────

/// Hash two nodes using canonical ordering (smaller hash on left).
fn hash_pair(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut combined = [0u8; 64];
    if a <= b {
        combined[..32].copy_from_slice(a);
        combined[32..].copy_from_slice(b);
    } else {
        combined[..32].copy_from_slice(b);
        combined[32..].copy_from_slice(a);
    }
    blake3_hash(&combined)
}

/// Compute a Merkle root from a sorted list of leaf hashes.
fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    if leaves.len() == 1 {
        return leaves[0];
    }

    let mut level: Vec<[u8; 32]> = leaves.to_vec();
    while level.len() > 1 {
        let mut next_level = Vec::with_capacity(level.len().div_ceil(2));
        for pair in level.chunks(2) {
            if pair.len() == 2 {
                next_level.push(hash_pair(&pair[0], &pair[1]));
            } else {
                next_level.push(pair[0]);
            }
        }
        level = next_level;
    }
    level[0]
}

/// Compute a Merkle path (sibling hashes) for a leaf.
fn merkle_path(leaves: &[[u8; 32]], target: &[u8; 32]) -> Vec<Option<[u8; 32]>> {
    let idx = match leaves.iter().position(|l| l == target) {
        Some(i) => i,
        None => return vec![],
    };

    let mut path = Vec::new();
    let mut level: Vec<[u8; 32]> = leaves.to_vec();
    let mut pos = idx;

    while level.len() > 1 {
        if pos % 2 == 0 {
            if pos + 1 < level.len() {
                path.push(Some(level[pos + 1]));
            } else {
                // Odd element promoted without sibling
                path.push(None);
            }
        } else {
            path.push(Some(level[pos - 1]));
        }

        let mut next_level = Vec::with_capacity(level.len().div_ceil(2));
        for pair in level.chunks(2) {
            if pair.len() == 2 {
                next_level.push(hash_pair(&pair[0], &pair[1]));
            } else {
                next_level.push(pair[0]);
            }
        }
        level = next_level;
        pos /= 2;
    }
    path
}

/// Recompute root from a leaf hash and its Merkle proof path.
fn compute_root_from_proof(leaf: &[u8; 32], path: &[Option<[u8; 32]>]) -> [u8; 32] {
    let mut current = *leaf;
    for sibling in path.iter().flatten() {
        current = hash_pair(&current, sibling);
    }
    current
}

/// Find the left and right neighbors of a hash in a sorted list.
fn find_neighbors(sorted: &[[u8; 32]], target: &[u8; 32]) -> (Option<[u8; 32]>, Option<[u8; 32]>) {
    let pos = sorted.partition_point(|x| x < target);
    let left = if pos > 0 { Some(sorted[pos - 1]) } else { None };
    let right = if pos < sorted.len() {
        Some(sorted[pos])
    } else {
        None
    };
    (left, right)
}

// ─────────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

        // Wrong element should fail
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

        // "beta" is not in the set
        let proof = acc.prove_non_membership(b"beta").unwrap();
        assert!(acc.verify_non_membership(&proof, b"beta"));

        // Cannot prove non-membership for a member
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

        acc.remove(b"second");
        let d3 = acc.digest();
        assert_eq!(d1, d3); // back to same state
    }

    #[test]
    fn test_digest_deterministic() {
        let mut acc1 = InMemoryAccumulator::new();
        let mut acc2 = InMemoryAccumulator::new();

        // Same elements added in different order → same digest
        acc1.add(b"X");
        acc1.add(b"Y");

        acc2.add(b"Y");
        acc2.add(b"X");

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
}

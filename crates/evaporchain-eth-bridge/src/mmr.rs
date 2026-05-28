//! Bridge-layer Merkle Mountain Range (keccak256).
//!
//! This is the *bridge* MMR — a parallel structure to whatever
//! EvaporChain's native ghost-record MMR uses (which is BLAKE3-based).
//! The Solidity verifier needs keccak so the on-chain hash cost stays
//! low; the producer (validators / relayer) computes both in lockstep
//! and signs the keccak root inside the block header tuple.
//!
//! Structure:
//!   * Leaves are 32-byte hashes (typically `keccak256(objectId || ...)`).
//!   * Built into perfect binary subtrees (peaks) left-to-right.
//!   * Bagged root = right-to-left concatenation: `keccak(P_n ‖ keccak(P_{n-1} ‖ ...))`.
//!
//! Inclusion-proof format matches `MmrInclusion.sol::verify`.

use sha3::{Digest, Keccak256};

fn k256(input: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(input);
    let out = h.finalize();
    let mut a = [0u8; 32];
    a.copy_from_slice(&out);
    a
}

fn k256_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(left);
    buf[32..].copy_from_slice(right);
    k256(&buf)
}

/// An inclusion proof for one leaf.
#[derive(Debug, Clone)]
pub struct InclusionProof {
    /// Index of the leaf (0..tree_size).
    pub leaf_index: u64,
    /// Total leaf count.
    pub tree_size: u64,
    /// Concatenated 32-byte sibling hashes, leaf-up.
    pub path: Vec<u8>,
    /// Peaks left of the leaf's peak (in MMR order).
    pub peaks_left: Vec<u8>,
    /// Peaks right of the leaf's peak.
    pub peaks_right: Vec<u8>,
}

/// Build a bridge-MMR over the supplied leaves and return
/// `(root, inclusion_proof_for_target_leaf)`.
///
/// Strategy: split the `n` leaves into a sequence of perfect binary
/// subtrees of decreasing power-of-two sizes — the standard MMR peak
/// decomposition. For each peak that's NOT the one containing
/// `target_index`, accumulate it into `peaks_left` or `peaks_right`.
/// For the peak containing the target, run a Merkle inclusion-proof
/// walk to populate `path`.
pub fn build_and_prove(leaves: &[[u8; 32]], target_index: u64) -> ([u8; 32], InclusionProof) {
    assert!(!leaves.is_empty());
    assert!((target_index as usize) < leaves.len());

    let n = leaves.len();

    // Decompose n into descending powers of two: e.g. 13 = 8 + 4 + 1.
    let mut peak_sizes: Vec<usize> = Vec::new();
    {
        let mut remaining = n;
        let mut p = 1usize << (usize::BITS - 1 - remaining.leading_zeros());
        while remaining > 0 {
            if p <= remaining {
                peak_sizes.push(p);
                remaining -= p;
            }
            p >>= 1;
        }
    }

    // Compute each peak. While iterating, watch for the peak containing
    // the target leaf and capture its in-peak index + leaf list slice.
    let mut peaks: Vec<[u8; 32]> = Vec::with_capacity(peak_sizes.len());
    let mut leaf_cursor: usize = 0;
    let mut target_peak_idx: usize = 0;
    let mut target_in_peak_idx: usize = 0;
    let mut target_peak_leaves: Vec<[u8; 32]> = Vec::new();

    for (pi, &sz) in peak_sizes.iter().enumerate() {
        let slice: Vec<[u8; 32]> = leaves[leaf_cursor..leaf_cursor + sz].to_vec();
        let peak = compute_peak(&slice);
        if (target_index as usize) >= leaf_cursor && (target_index as usize) < leaf_cursor + sz {
            target_peak_idx = pi;
            target_in_peak_idx = (target_index as usize) - leaf_cursor;
            target_peak_leaves = slice.clone();
        }
        peaks.push(peak);
        leaf_cursor += sz;
    }

    // Build the leaf-up sibling path inside the target peak.
    let path = merkle_path(&target_peak_leaves, target_in_peak_idx);

    // Other peaks split into left/right around target.
    let mut peaks_left: Vec<u8> = Vec::new();
    let mut peaks_right: Vec<u8> = Vec::new();
    for (pi, p) in peaks.iter().enumerate() {
        if pi < target_peak_idx {
            peaks_left.extend_from_slice(p);
        } else if pi > target_peak_idx {
            peaks_right.extend_from_slice(p);
        }
    }

    // Bag right-to-left: peak[n-1], peak[n-2], …, peak[0].
    let mut acc: Option<[u8; 32]> = None;
    for p in peaks.iter().rev() {
        acc = Some(match acc {
            None => *p,
            Some(a) => k256_pair(p, &a),
        });
    }
    let root = acc.expect("non-empty MMR");

    let proof = InclusionProof {
        leaf_index: target_in_peak_idx as u64,
        tree_size: target_peak_leaves.len() as u64,
        path,
        peaks_left,
        peaks_right,
    };
    (root, proof)
}

/// Compute the Merkle root of a perfect binary tree (leaf count is a
/// power of two).
fn compute_peak(leaves: &[[u8; 32]]) -> [u8; 32] {
    assert!(!leaves.is_empty() && leaves.len().is_power_of_two());
    let mut layer: Vec<[u8; 32]> = leaves.to_vec();
    while layer.len() > 1 {
        let mut next = Vec::with_capacity(layer.len() / 2);
        for pair in layer.chunks_exact(2) {
            next.push(k256_pair(&pair[0], &pair[1]));
        }
        layer = next;
    }
    layer[0]
}

/// Build the leaf-up Merkle inclusion path inside a perfect-binary-tree
/// peak: at each level, the sibling of the current node.
fn merkle_path(leaves: &[[u8; 32]], leaf_index: usize) -> Vec<u8> {
    if leaves.len() == 1 {
        return Vec::new();
    }
    let mut path: Vec<u8> = Vec::new();
    let mut layer: Vec<[u8; 32]> = leaves.to_vec();
    let mut idx = leaf_index;
    while layer.len() > 1 {
        let sibling = if idx & 1 == 0 {
            layer[idx + 1]
        } else {
            layer[idx - 1]
        };
        path.extend_from_slice(&sibling);
        let mut next = Vec::with_capacity(layer.len() / 2);
        for pair in layer.chunks_exact(2) {
            next.push(k256_pair(&pair[0], &pair[1]));
        }
        layer = next;
        idx /= 2;
    }
    path
}

/// keccak256 the canonical leaf encoding used by `EvaporationDispatcher.dispatch`:
///   `keccak256(objectId || evaporatedAtHeight || finalEnergy)`
pub fn ghost_leaf_hash(
    object_id: [u8; 32],
    evaporated_at_height: u64,
    final_energy: u128,
) -> [u8; 32] {
    let mut buf = Vec::with_capacity(32 + 8 + 16);
    buf.extend_from_slice(&object_id);
    buf.extend_from_slice(&evaporated_at_height.to_be_bytes());
    buf.extend_from_slice(&final_energy.to_be_bytes());
    k256(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(seed: u8) -> [u8; 32] {
        k256(&[seed; 16])
    }

    #[test]
    fn single_leaf_root_equals_leaf() {
        let leaves = vec![leaf(1)];
        let (root, proof) = build_and_prove(&leaves, 0);
        assert_eq!(root, leaves[0]);
        assert_eq!(proof.path.len(), 0);
        assert_eq!(proof.peaks_left.len(), 0);
        assert_eq!(proof.peaks_right.len(), 0);
        assert_eq!(proof.tree_size, 1);
        assert_eq!(proof.leaf_index, 0);
    }

    #[test]
    fn four_leaves_perfect_tree() {
        let leaves: Vec<[u8; 32]> = (0..4u8).map(leaf).collect();
        let (root, proof) = build_and_prove(&leaves, 2);
        // path should be 2 hashes (height 2 tree).
        assert_eq!(proof.path.len(), 64);
        assert_eq!(proof.tree_size, 4);
        assert_eq!(proof.leaf_index, 2);

        // Reconstruct the root manually.
        let n01 = k256_pair(&leaves[0], &leaves[1]);
        let n23 = k256_pair(&leaves[2], &leaves[3]);
        let r = k256_pair(&n01, &n23);
        assert_eq!(root, r);
    }

    #[test]
    fn five_leaves_two_peaks() {
        let leaves: Vec<[u8; 32]> = (0..5u8).map(leaf).collect();
        // 5 = 4 + 1 → peaks of sizes 4 and 1.
        let (root, proof) = build_and_prove(&leaves, 4);
        // Target leaf is in the size-1 peak (no path needed inside).
        assert_eq!(proof.path.len(), 0);
        assert_eq!(proof.tree_size, 1);
        assert_eq!(proof.peaks_left.len(), 32); // size-4 peak left of us
        assert_eq!(proof.peaks_right.len(), 0);

        // Manually reconstruct the bag.
        let p_big = {
            let n01 = k256_pair(&leaves[0], &leaves[1]);
            let n23 = k256_pair(&leaves[2], &leaves[3]);
            k256_pair(&n01, &n23)
        };
        let p_small = leaves[4];
        let bagged = k256_pair(&p_big, &p_small);
        assert_eq!(root, bagged);
    }

    #[test]
    fn ghost_leaf_hash_deterministic() {
        let a = ghost_leaf_hash([1u8; 32], 100, 0);
        let b = ghost_leaf_hash([1u8; 32], 100, 0);
        assert_eq!(a, b);
        let c = ghost_leaf_hash([1u8; 32], 101, 0);
        assert_ne!(a, c);
    }
}

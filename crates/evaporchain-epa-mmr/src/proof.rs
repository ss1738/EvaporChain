//! `InclusionProof` — energy-aware MMR inclusion proof.
//!
//! A proof carries:
//! 1. The leaf being proven (with energy + minted_at).
//! 2. The leaf's index.
//! 3. The sibling-hash path within the leaf's peak-tree.
//! 4. The other peak hashes (left + right of the target peak)
//!    needed to reconstruct the bagged root.
//!
//! Verification:
//! 1. Recompute the leaf hash.
//! 2. Walk the sibling path, hashing left/right per the index's
//!    bits, to get the target-peak hash.
//! 3. Bag the target peak with the other peak hashes to get the
//!    candidate root.
//! 4. Compare candidate root to expected.
//! 5. **Energy floor check** — if the leaf's energy is below the
//!    verifier's floor, the proof is structurally invalid even
//!    if the hash chain checks out. This is the EPA part: a
//!    proof of an evaporated leaf is rejected.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::leaf::EnergyLeaf;
use crate::mmr::{bag_peaks, perfect_tree_root, EpaMmr};

pub const INNER_TAG: &[u8] = b"evaporchain:epa-mmr:inner:v1\0";
pub const PEAK_BAG_TAG: &[u8] = b"evaporchain:epa-mmr:peak-bag:v1\0";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProofError {
    #[error("leaf index {idx} out of range (mmr has {len} leaves)")]
    LeafOutOfRange { idx: usize, len: usize },
    #[error("inclusion-proof root mismatch")]
    RootMismatch,
    #[error("leaf energy {energy} is below floor {floor} — proof structurally invalid")]
    EnergyBelowFloor { energy: u64, floor: u64 },
    #[error("malformed proof — sibling path length doesn't match the leaf's peak-tree height")]
    MalformedPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InclusionProof {
    pub leaf: EnergyLeaf,
    /// Index of `leaf` within the MMR's leaf-append sequence.
    pub leaf_index: usize,
    /// Sibling hashes from leaf-up to the peak, in
    /// leaf-towards-root order.
    pub sibling_path: Vec<[u8; 32]>,
    /// Peak hashes left-to-right of the target peak in the bagged
    /// root, EXCLUDING the target peak itself (which the
    /// verifier reconstructs from the sibling path).
    pub left_peaks: Vec<[u8; 32]>,
    /// Peak hashes right of the target peak in the bagged root.
    pub right_peaks: Vec<[u8; 32]>,
    /// MMR leaf count at the time of proof generation. Lets the
    /// verifier check the proof was produced against the right
    /// MMR size.
    pub mmr_leaf_count: usize,
}

/// Domain-tagged inner-node hash.
pub fn inner_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(INNER_TAG);
    h.update(left);
    h.update(right);
    *h.finalize().as_bytes()
}

/// Domain-tagged peak-bagging hash.
pub fn peak_bag_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(PEAK_BAG_TAG);
    h.update(left);
    h.update(right);
    *h.finalize().as_bytes()
}

impl InclusionProof {
    /// Build an inclusion proof for `leaf_index` against `mmr`.
    pub fn build(mmr: &EpaMmr, leaf_index: usize) -> Result<Self, ProofError> {
        let n = mmr.leaf_count();
        if leaf_index >= n {
            return Err(ProofError::LeafOutOfRange {
                idx: leaf_index,
                len: n,
            });
        }
        let leaves = mmr.leaves();
        let leaf = leaves[leaf_index].clone();

        // Find which peak-tree contains leaf_index, and compute
        // (offset_in_tree, tree_size, left_peaks, right_peaks).
        let (peak_idx, peak_size, offset_in_peak, peak_left_lo, peak_left_hi) =
            locate_peak(n, leaf_index);
        let _ = peak_idx; // not directly needed beyond locate_peak's return

        let peak_tree_slice = &leaves[peak_left_lo..peak_left_hi];

        // Compute the sibling path within the peak tree.
        let sibling_path = sibling_path_in_perfect_tree(peak_tree_slice, offset_in_peak);

        // Compute the other peaks, splitting at the target peak.
        let all_peaks = mmr.peaks();
        let mut peak_idx_in_all = 0;
        let mut walked_size = 0;
        for (i, p_size) in peak_sizes(n).iter().enumerate() {
            if walked_size == peak_left_lo && *p_size == peak_size {
                peak_idx_in_all = i;
                break;
            }
            walked_size += p_size;
        }
        let left_peaks: Vec<[u8; 32]> = all_peaks[..peak_idx_in_all].to_vec();
        let right_peaks: Vec<[u8; 32]> = all_peaks[peak_idx_in_all + 1..].to_vec();

        Ok(Self {
            leaf,
            leaf_index,
            sibling_path,
            left_peaks,
            right_peaks,
            mmr_leaf_count: n,
        })
    }
}

/// Verify an inclusion proof against a candidate root and an
/// energy floor.
pub fn verify_inclusion(
    proof: &InclusionProof,
    expected_root: &[u8; 32],
    energy_floor: u64,
) -> Result<(), ProofError> {
    // Energy floor check: structural-invalidity for evaporated
    // leaves. Even if hash chain is valid.
    if proof.leaf.energy < energy_floor {
        return Err(ProofError::EnergyBelowFloor {
            energy: proof.leaf.energy,
            floor: energy_floor,
        });
    }

    // Walk the sibling path to reconstruct the target peak.
    let mut h = proof.leaf.hash();
    let (_, peak_size, offset_in_peak, _, _) = locate_peak(proof.mmr_leaf_count, proof.leaf_index);
    let expected_path_len = peak_size.trailing_zeros() as usize;
    if proof.sibling_path.len() != expected_path_len {
        return Err(ProofError::MalformedPath);
    }

    let mut idx = offset_in_peak;
    for sibling in &proof.sibling_path {
        h = if idx & 1 == 0 {
            inner_hash(&h, sibling)
        } else {
            inner_hash(sibling, &h)
        };
        idx >>= 1;
    }

    // Now `h` is the target peak. Bag with the other peaks to
    // reconstruct the candidate root.
    let mut all_peaks: Vec<[u8; 32]> =
        Vec::with_capacity(proof.left_peaks.len() + 1 + proof.right_peaks.len());
    all_peaks.extend_from_slice(&proof.left_peaks);
    all_peaks.push(h);
    all_peaks.extend_from_slice(&proof.right_peaks);

    let candidate_root = bag_peaks(&all_peaks);
    if &candidate_root != expected_root {
        return Err(ProofError::RootMismatch);
    }
    Ok(())
}

/// Locate the peak-tree containing `leaf_index` in an MMR of size
/// `n`. Returns `(peak_position, peak_size, offset_in_peak,
/// peak_lo, peak_hi)` where `peak_lo..peak_hi` is the peak's
/// half-open leaf range in the original sequence.
fn locate_peak(n: usize, leaf_index: usize) -> (usize, usize, usize, usize, usize) {
    debug_assert!(leaf_index < n);
    let mut start = 0;
    let mut peak_pos = 0;
    for height in (0..usize::BITS as usize).rev() {
        let size = match 1usize.checked_shl(height as u32) {
            Some(s) => s,
            None => continue,
        };
        if (n >> height) & 1 == 1 {
            let end = start + size;
            if leaf_index < end {
                return (peak_pos, size, leaf_index - start, start, end);
            }
            start = end;
            peak_pos += 1;
        }
    }
    unreachable!("leaf_index < n guarantees we find a peak");
}

/// Decompose `n` into the sequence of peak sizes (largest first).
fn peak_sizes(n: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for height in (0..usize::BITS as usize).rev() {
        let size = match 1usize.checked_shl(height as u32) {
            Some(s) => s,
            None => continue,
        };
        if (n >> height) & 1 == 1 {
            out.push(size);
        }
    }
    out
}

/// Sibling path for a leaf at `offset` in a perfect tree over
/// `slice.len()` leaves. The path is leaf-up.
fn sibling_path_in_perfect_tree(slice: &[EnergyLeaf], offset: usize) -> Vec<[u8; 32]> {
    debug_assert!(slice.len().is_power_of_two());
    if slice.len() == 1 {
        return vec![];
    }
    let half = slice.len() / 2;
    if offset < half {
        // Sibling is the right half's root, followed by the
        // path within the left half.
        let mut p = sibling_path_in_perfect_tree(&slice[..half], offset);
        p.push(perfect_tree_root(&slice[half..]));
        p
    } else {
        let mut p = sibling_path_in_perfect_tree(&slice[half..], offset - half);
        p.push(perfect_tree_root(&slice[..half]));
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mmr::EpaMmr;

    fn leaf(v: u8, energy: u64) -> EnergyLeaf {
        EnergyLeaf::new([v; 32], energy, 0)
    }

    fn build_mmr(n: usize, energy: u64) -> EpaMmr {
        let mut m = EpaMmr::new();
        for i in 1..=n {
            m.append(leaf(i as u8, energy));
        }
        m
    }

    // ── inclusion-proof round-trips ───────────────────────────────

    #[test]
    fn single_leaf_proof_round_trips() {
        let m = build_mmr(1, 1000);
        let root = m.root().unwrap();
        let proof = InclusionProof::build(&m, 0).unwrap();
        verify_inclusion(&proof, &root, 0).unwrap();
    }

    #[test]
    fn power_of_two_proof_round_trips() {
        // 8-leaf MMR: one peak of size 8 (height 3 → 3-step path).
        let m = build_mmr(8, 1000);
        let root = m.root().unwrap();
        for i in 0..8 {
            let proof = InclusionProof::build(&m, i).unwrap();
            assert_eq!(proof.sibling_path.len(), 3);
            verify_inclusion(&proof, &root, 0).unwrap();
        }
    }

    #[test]
    fn non_power_of_two_proof_round_trips() {
        // 11 = 0b1011 → peaks of sizes 8, 2, 1.
        let m = build_mmr(11, 1000);
        let root = m.root().unwrap();
        for i in 0..11 {
            let proof = InclusionProof::build(&m, i).unwrap();
            verify_inclusion(&proof, &root, 0).unwrap();
        }
    }

    // ── failure modes ─────────────────────────────────────────────

    #[test]
    fn out_of_range_leaf_rejected() {
        let m = build_mmr(5, 1000);
        let err = InclusionProof::build(&m, 100).unwrap_err();
        assert!(matches!(
            err,
            ProofError::LeafOutOfRange { idx: 100, len: 5 }
        ));
    }

    #[test]
    fn proof_against_wrong_root_rejected() {
        let m = build_mmr(5, 1000);
        let proof = InclusionProof::build(&m, 2).unwrap();
        let wrong_root = [0xFF; 32];
        let err = verify_inclusion(&proof, &wrong_root, 0).unwrap_err();
        assert_eq!(err, ProofError::RootMismatch);
    }

    #[test]
    fn tampered_leaf_rejected() {
        let m = build_mmr(5, 1000);
        let root = m.root().unwrap();
        let mut proof = InclusionProof::build(&m, 2).unwrap();
        // Tamper: pretend the leaf had a different value.
        proof.leaf.value_hash = [0xDE; 32];
        let err = verify_inclusion(&proof, &root, 0).unwrap_err();
        assert_eq!(err, ProofError::RootMismatch);
    }

    #[test]
    fn tampered_energy_rejected() {
        // The energy is hashed into the leaf — a prover who pumps
        // the energy value gets a leaf-hash mismatch (because the
        // committed leaf hash is the ORIGINAL energy).
        let m = build_mmr(5, 1000);
        let root = m.root().unwrap();
        let mut proof = InclusionProof::build(&m, 2).unwrap();
        proof.leaf.energy = 999_999_999; // pump it
        let err = verify_inclusion(&proof, &root, 0).unwrap_err();
        // Hash chain breaks → root mismatch (NOT below floor).
        assert_eq!(err, ProofError::RootMismatch);
    }

    // ── EPA energy-floor — the load-bearing decay-native check ────

    #[test]
    fn proof_of_evaporated_leaf_rejected_by_floor() {
        // A leaf with energy=10 is in the MMR. The verifier's
        // floor is 100. The proof's hash chain is valid against
        // the MMR root, but the energy-floor check rejects it.
        let mut m = EpaMmr::new();
        m.append(leaf(1, 10)); // evaporated
        m.append(leaf(2, 1000)); // healthy
        let root = m.root().unwrap();
        let proof = InclusionProof::build(&m, 0).unwrap();
        let err = verify_inclusion(&proof, &root, 100).unwrap_err();
        assert_eq!(
            err,
            ProofError::EnergyBelowFloor {
                energy: 10,
                floor: 100
            }
        );
    }

    #[test]
    fn proof_of_healthy_leaf_passes_floor() {
        // The same MMR; this time prove the healthy leaf at
        // index 1.
        let mut m = EpaMmr::new();
        m.append(leaf(1, 10));
        m.append(leaf(2, 1000));
        let root = m.root().unwrap();
        let proof = InclusionProof::build(&m, 1).unwrap();
        verify_inclusion(&proof, &root, 100).unwrap();
    }

    #[test]
    fn floor_check_runs_before_hash_check() {
        // Even with a totally bogus root, the floor check fires
        // first — the verifier returns EnergyBelowFloor, not
        // RootMismatch, because the leaf is evaporated.
        let mut m = EpaMmr::new();
        m.append(leaf(1, 10));
        let _ = m.root();
        let proof = InclusionProof::build(&m, 0).unwrap();
        let err = verify_inclusion(&proof, &[0xFF; 32], 100).unwrap_err();
        assert!(matches!(err, ProofError::EnergyBelowFloor { .. }));
    }

    // ── update_energy invalidates old proofs ──────────────────────

    #[test]
    fn updating_energy_invalidates_old_proof() {
        let mut m = build_mmr(5, 1000);
        let root_before = m.root().unwrap();
        let proof = InclusionProof::build(&m, 2).unwrap();
        // Old root is fine.
        verify_inclusion(&proof, &root_before, 0).unwrap();

        // Decay tick reduces energy → MMR root changes.
        m.update_energy(2, 500).unwrap();
        let root_after = m.root().unwrap();
        assert_ne!(root_before, root_after);

        // Old proof against new root: rejected (old leaf-hash
        // doesn't match the updated leaf).
        let err = verify_inclusion(&proof, &root_after, 0).unwrap_err();
        assert_eq!(err, ProofError::RootMismatch);

        // New proof against new root: passes.
        let fresh = InclusionProof::build(&m, 2).unwrap();
        verify_inclusion(&fresh, &root_after, 0).unwrap();
    }

    // ── domain-tag separation ─────────────────────────────────────

    #[test]
    fn inner_and_peak_bag_tags_actually_differ() {
        // Domain separation: hashing the same two children under
        // INNER_TAG vs PEAK_BAG_TAG must produce different
        // outputs. Otherwise an attacker could craft a peak that
        // also passes as an inner node, swapping levels of the
        // tree.
        let a = [0x11; 32];
        let b = [0x22; 32];
        assert_ne!(inner_hash(&a, &b), peak_bag_hash(&a, &b));
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "EvaporChain MMR inclusion proofs are
        // structurally invalid for leaves whose energy has
        // decayed below threshold. The chain doesn't have to
        // delete leaves to make them unprovable; the verifier
        // refuses them. Provability is a function of energy,
        // not just topology."
        let mut m = EpaMmr::new();
        m.append(leaf(1, 1000));
        m.append(leaf(2, 1000));
        m.append(leaf(3, 1000));
        let root = m.root().unwrap();

        // All leaves provable initially.
        for i in 0..3 {
            let p = InclusionProof::build(&m, i).unwrap();
            verify_inclusion(&p, &root, 100).unwrap();
        }

        // The chain's decay tick fires; leaf 1 falls below the
        // floor.
        m.update_energy(1, 50).unwrap();
        let root2 = m.root().unwrap();

        // Leaf 0 and 2 still provable above floor=100. Leaf 1
        // unprovable.
        verify_inclusion(&InclusionProof::build(&m, 0).unwrap(), &root2, 100).unwrap();
        verify_inclusion(&InclusionProof::build(&m, 2).unwrap(), &root2, 100).unwrap();
        let err =
            verify_inclusion(&InclusionProof::build(&m, 1).unwrap(), &root2, 100).unwrap_err();
        assert!(matches!(err, ProofError::EnergyBelowFloor { .. }));
    }

    proptest::proptest! {
        #[test]
        fn property_all_leaves_in_mmr_provable_at_zero_floor(
            n in 1usize..32usize,
            target_idx in 0usize..32usize,
        ) {
            let m = build_mmr(n, 1000);
            let target = target_idx % n;
            let root = m.root().unwrap();
            let proof = InclusionProof::build(&m, target).unwrap();
            proptest::prop_assert!(verify_inclusion(&proof, &root, 0).is_ok());
        }

        #[test]
        fn property_floor_above_leaf_energy_always_rejects(
            n in 1usize..32usize,
            target_idx in 0usize..32usize,
            energy in 1u64..1000u64,
            floor_above in 1u64..1000u64,
        ) {
            let m = build_mmr(n, energy);
            let target = target_idx % n;
            let root = m.root().unwrap();
            let proof = InclusionProof::build(&m, target).unwrap();
            let floor = energy.saturating_add(floor_above);
            proptest::prop_assert!(verify_inclusion(&proof, &root, floor).is_err());
        }
    }
}

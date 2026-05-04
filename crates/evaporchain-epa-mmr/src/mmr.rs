//! `EpaMmr` — peaks-array Merkle Mountain Range with energy-tagged
//! leaves.
//!
//! ## Representation
//!
//! Internally, an MMR with `N` leaves is a forest of perfect
//! binary trees of strictly decreasing heights, one per set bit
//! of `N` in binary. We track the per-leaf data and *all node
//! hashes* in a single `Vec` (the canonical "node array"
//! representation; node positions are determined by the MMR
//! preorder traversal Todd describes).
//!
//! For V1 simplicity we keep the full leaf list (so we can
//! generate proofs without external state) and recompute peaks
//! lazily. This gives O(log N) append + O(log N) proof.
//! Production deployments would persist the node array directly.
//!
//! ## Invariants
//!
//! - `leaves.len()` is the leaf count.
//! - The MMR root is the bagging of the current peaks.
//! - Two MMRs with the same leaf sequence (in the same order)
//!   produce the byte-identical root — required for validator
//!   agreement.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::leaf::{leaf_hash, EnergyLeaf};
use crate::proof::{inner_hash, peak_bag_hash};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MmrError {
    #[error("leaf index {idx} is out of range (have {len} leaves)")]
    LeafOutOfRange { idx: usize, len: usize },
    #[error("MMR is empty — no root")]
    Empty,
}

/// Energy-Padded Authenticated MMR.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EpaMmr {
    /// All leaves in append order. Production storage would not
    /// keep this; for V1 we keep it for simple proof generation.
    leaves: Vec<EnergyLeaf>,
}

impl EpaMmr {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    /// Append a leaf. Returns the leaf's index.
    pub fn append(&mut self, leaf: EnergyLeaf) -> usize {
        let idx = self.leaves.len();
        self.leaves.push(leaf);
        idx
    }

    /// Read a leaf by index.
    pub fn get(&self, idx: usize) -> Option<&EnergyLeaf> {
        self.leaves.get(idx)
    }

    /// Mutate a leaf's energy in-place — used by the chain's decay
    /// tick. The chain calls this with the post-decay energy; the
    /// MMR root changes accordingly because the leaf hash binds
    /// energy.
    pub fn update_energy(&mut self, idx: usize, new_energy: u64) -> Result<(), MmrError> {
        let len = self.leaves.len();
        let leaf = self
            .leaves
            .get_mut(idx)
            .ok_or(MmrError::LeafOutOfRange { idx, len })?;
        leaf.energy = new_energy;
        Ok(())
    }

    /// Compute the current MMR root (bagging of peaks).
    ///
    /// Algorithm:
    /// 1. For each set bit of `leaf_count`, compute the peak
    ///    spanning that prefix of leaves.
    /// 2. Bag the peaks left-to-right with `peak_bag_hash`.
    pub fn root(&self) -> Result<[u8; 32], MmrError> {
        if self.leaves.is_empty() {
            return Err(MmrError::Empty);
        }
        let peaks = self.peaks();
        Ok(bag_peaks(&peaks))
    }

    /// Return all current peak hashes, left-to-right (largest tree
    /// first).
    pub fn peaks(&self) -> Vec<[u8; 32]> {
        // Decompose leaf_count into a sum of powers of two
        // (most-significant bit first); each power-of-two tree
        // produces one peak.
        let n = self.leaves.len();
        if n == 0 {
            return vec![];
        }
        let mut peaks = Vec::new();
        let mut start = 0;
        // Find the highest set bit and walk down.
        for height in (0..=usize::BITS as usize - 1).rev() {
            let size = 1usize.checked_shl(height as u32).unwrap_or(0);
            if size == 0 {
                continue;
            }
            if (n >> height) & 1 == 1 {
                let end = start + size;
                let peak = perfect_tree_root(&self.leaves[start..end]);
                peaks.push(peak);
                start = end;
            }
        }
        peaks
    }

    /// Internal accessor for proof construction.
    pub(crate) fn leaves(&self) -> &[EnergyLeaf] {
        &self.leaves
    }
}

/// Compute the root of a perfect binary tree over `slice.len()`
/// leaves. `slice.len()` MUST be a power of two.
pub(crate) fn perfect_tree_root(slice: &[EnergyLeaf]) -> [u8; 32] {
    debug_assert!(slice.len().is_power_of_two() && !slice.is_empty());
    if slice.len() == 1 {
        return leaf_hash(
            &slice[0].value_hash,
            slice[0].energy,
            slice[0].minted_at_epoch,
        );
    }
    let half = slice.len() / 2;
    let left = perfect_tree_root(&slice[..half]);
    let right = perfect_tree_root(&slice[half..]);
    inner_hash(&left, &right)
}

/// Bag the peaks left-to-right under the peak-bag domain tag.
///
/// Convention: the bagged hash is computed right-to-left so that
/// appending leaves to the MMR (which appends NEW peaks on the
/// right side, before any merges) keeps existing left-side peaks
/// stable in the inner stages of the bag computation.
pub(crate) fn bag_peaks(peaks: &[[u8; 32]]) -> [u8; 32] {
    debug_assert!(!peaks.is_empty());
    if peaks.len() == 1 {
        return peaks[0];
    }
    // Right-to-left fold: bag = peak_bag(left, bag_rest)
    let mut bag = peaks[peaks.len() - 1];
    for p in peaks.iter().rev().skip(1) {
        bag = peak_bag_hash(p, &bag);
    }
    bag
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(v: u8, energy: u64) -> EnergyLeaf {
        EnergyLeaf::new([v; 32], energy, 0)
    }

    #[test]
    fn empty_mmr_root_is_error() {
        let m = EpaMmr::new();
        assert!(matches!(m.root().unwrap_err(), MmrError::Empty));
    }

    #[test]
    fn single_leaf_root_is_its_leaf_hash() {
        let mut m = EpaMmr::new();
        let l = leaf(1, 1000);
        m.append(l.clone());
        assert_eq!(m.root().unwrap(), l.hash());
    }

    #[test]
    fn root_changes_with_energy_update() {
        let mut m = EpaMmr::new();
        m.append(leaf(1, 1000));
        let root_before = m.root().unwrap();
        m.update_energy(0, 500).unwrap();
        let root_after = m.root().unwrap();
        assert_ne!(root_before, root_after);
    }

    #[test]
    fn append_increases_leaf_count() {
        let mut m = EpaMmr::new();
        m.append(leaf(1, 1000));
        m.append(leaf(2, 1000));
        m.append(leaf(3, 1000));
        assert_eq!(m.leaf_count(), 3);
    }

    #[test]
    fn root_is_deterministic_across_runs() {
        let mut m1 = EpaMmr::new();
        let mut m2 = EpaMmr::new();
        for i in 1..=10u8 {
            m1.append(leaf(i, 1000));
            m2.append(leaf(i, 1000));
        }
        assert_eq!(m1.root().unwrap(), m2.root().unwrap());
    }

    #[test]
    fn peak_count_matches_set_bit_count() {
        let mut m = EpaMmr::new();
        for i in 1..=11u8 {
            m.append(leaf(i, 1000));
        }
        // 11 = 0b1011 — three set bits → three peaks.
        assert_eq!(m.peaks().len(), 11usize.count_ones() as usize);
    }

    #[test]
    fn power_of_two_size_has_one_peak() {
        let mut m = EpaMmr::new();
        for i in 1..=8u8 {
            m.append(leaf(i, 1000));
        }
        // 8 = 0b1000 — one set bit → one peak.
        assert_eq!(m.peaks().len(), 1);
    }

    #[test]
    fn out_of_range_update_rejected() {
        let mut m = EpaMmr::new();
        m.append(leaf(1, 1000));
        let err = m.update_energy(99, 0).unwrap_err();
        assert!(matches!(err, MmrError::LeafOutOfRange { idx: 99, len: 1 }));
    }

    #[test]
    fn root_changes_with_appended_leaf() {
        let mut m = EpaMmr::new();
        m.append(leaf(1, 1000));
        let r1 = m.root().unwrap();
        m.append(leaf(2, 1000));
        let r2 = m.root().unwrap();
        assert_ne!(r1, r2);
    }

    #[test]
    fn round_trip_serde() {
        let mut m = EpaMmr::new();
        for i in 1..=5u8 {
            m.append(leaf(i, 1000));
        }
        let s = serde_json::to_string(&m).unwrap();
        let back: EpaMmr = serde_json::from_str(&s).unwrap();
        assert_eq!(m.root().unwrap(), back.root().unwrap());
    }
}

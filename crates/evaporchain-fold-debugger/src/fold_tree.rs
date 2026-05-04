//! `FoldTree` — tree of folded snapshots + per-block input deltas.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::snapshot::Snapshot;

/// One forward fold step: input bytes + the next state-root
/// committed-to after applying them. Treated opaquely; the chain's
/// proving layer pre-validated these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldStep {
    pub from_height: u64,
    pub to_height: u64,
    /// 32-byte commitment over the inputs (transactions, etc.)
    /// applied between heights.
    pub input_commitment: [u8; 32],
    /// State root after applying this fold step.
    pub resulting_state_root: [u8; 32],
    /// Updated fold accumulator after this step.
    pub resulting_accumulator: [u8; 32],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FoldTreeError {
    #[error("snapshot at height {0} already exists — debugger refuses to overwrite (would break fold-tree determinism)")]
    DuplicateSnapshot(u64),
    #[error("fold step from {from_h} to {to_h} is non-monotone")]
    NonMonotoneStep { from_h: u64, to_h: u64 },
    #[error("no snapshot found at or below height {0}")]
    NoSnapshotForHeight(u64),
    #[error("fold-step chain is broken — gap between {gap_lo} and {gap_hi}")]
    BrokenChain { gap_lo: u64, gap_hi: u64 },
}

/// The fold tree. `BTreeMap<height, snapshot>` for sorted lookup;
/// `Vec<FoldStep>` carrying the linear chain of forward folds.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FoldTree {
    snapshots: BTreeMap<u64, Snapshot>,
    /// Forward fold-step chain; sorted by `from_height`.
    steps: Vec<FoldStep>,
    /// Highest snapshot height we know about.
    pub head_height: u64,
}

impl FoldTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a snapshot. Refuses to overwrite — same-height
    /// snapshots must be identical or the tree is corrupted.
    pub fn insert_snapshot(&mut self, s: Snapshot) -> Result<(), FoldTreeError> {
        if let Some(existing) = self.snapshots.get(&s.height) {
            if existing == &s {
                return Ok(());
            }
            return Err(FoldTreeError::DuplicateSnapshot(s.height));
        }
        if s.height > self.head_height {
            self.head_height = s.height;
        }
        self.snapshots.insert(s.height, s);
        Ok(())
    }

    /// Append a forward fold step. Must be monotone from the
    /// last-known step's `to_height`.
    pub fn append_step(&mut self, step: FoldStep) -> Result<(), FoldTreeError> {
        if step.to_height <= step.from_height {
            return Err(FoldTreeError::NonMonotoneStep {
                from_h: step.from_height,
                to_h: step.to_height,
            });
        }
        if let Some(last) = self.steps.last() {
            if step.from_height < last.to_height {
                return Err(FoldTreeError::NonMonotoneStep {
                    from_h: step.from_height,
                    to_h: step.to_height,
                });
            }
        }
        if step.to_height > self.head_height {
            self.head_height = step.to_height;
        }
        self.steps.push(step);
        Ok(())
    }

    /// Find the largest snapshot at or below `target`.
    pub fn nearest_snapshot_le(&self, target: u64) -> Result<&Snapshot, FoldTreeError> {
        self.snapshots
            .range(..=target)
            .next_back()
            .map(|(_, s)| s)
            .ok_or(FoldTreeError::NoSnapshotForHeight(target))
    }

    /// Walk the fold-step chain starting at `from_height`,
    /// stopping when `to_height` is reached. Returns the slice
    /// of relevant steps. Errors if the chain has a gap.
    pub fn steps_in_range(&self, from_h: u64, to_h: u64) -> Result<&[FoldStep], FoldTreeError> {
        if from_h >= to_h {
            return Ok(&[]);
        }
        // Find the index of the first step whose from_height >= from_h.
        let start = self
            .steps
            .iter()
            .position(|s| s.from_height >= from_h)
            .ok_or(FoldTreeError::BrokenChain {
                gap_lo: from_h,
                gap_hi: to_h,
            })?;
        // Find the index AFTER the last step whose to_height <= to_h.
        let mut end = start;
        let mut prev_to = from_h;
        for s in &self.steps[start..] {
            if s.from_height != prev_to {
                return Err(FoldTreeError::BrokenChain {
                    gap_lo: prev_to,
                    gap_hi: s.from_height,
                });
            }
            if s.to_height > to_h {
                break;
            }
            end += 1;
            prev_to = s.to_height;
            if prev_to == to_h {
                break;
            }
        }
        if prev_to != to_h {
            return Err(FoldTreeError::BrokenChain {
                gap_lo: prev_to,
                gap_hi: to_h,
            });
        }
        Ok(&self.steps[start..end])
    }

    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    pub fn step_count(&self) -> usize {
        self.steps.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(h: u64, root: u8, acc: u8) -> Snapshot {
        Snapshot::new(h, [root; 32], [acc; 32])
    }

    fn step(from: u64, to: u64, root_byte: u8) -> FoldStep {
        FoldStep {
            from_height: from,
            to_height: to,
            input_commitment: [(from + to) as u8; 32],
            resulting_state_root: [root_byte; 32],
            resulting_accumulator: [root_byte; 32],
        }
    }

    #[test]
    fn insert_snapshot_succeeds() {
        let mut t = FoldTree::new();
        t.insert_snapshot(snap(0, 0x01, 0x02)).unwrap();
        assert_eq!(t.snapshot_count(), 1);
        assert_eq!(t.head_height, 0);
    }

    #[test]
    fn duplicate_height_with_same_content_is_idempotent() {
        let mut t = FoldTree::new();
        let s = snap(0, 0x01, 0x02);
        t.insert_snapshot(s.clone()).unwrap();
        t.insert_snapshot(s).unwrap();
        assert_eq!(t.snapshot_count(), 1);
    }

    #[test]
    fn duplicate_height_with_different_content_rejected() {
        let mut t = FoldTree::new();
        t.insert_snapshot(snap(0, 0x01, 0x02)).unwrap();
        let err = t.insert_snapshot(snap(0, 0x03, 0x04)).unwrap_err();
        assert_eq!(err, FoldTreeError::DuplicateSnapshot(0));
    }

    #[test]
    fn nearest_snapshot_le_walks_correctly() {
        let mut t = FoldTree::new();
        t.insert_snapshot(snap(0, 0, 0)).unwrap();
        t.insert_snapshot(snap(8, 1, 1)).unwrap();
        t.insert_snapshot(snap(16, 2, 2)).unwrap();
        assert_eq!(t.nearest_snapshot_le(7).unwrap().height, 0);
        assert_eq!(t.nearest_snapshot_le(8).unwrap().height, 8);
        assert_eq!(t.nearest_snapshot_le(15).unwrap().height, 8);
        assert_eq!(t.nearest_snapshot_le(20).unwrap().height, 16);
    }

    #[test]
    fn nearest_snapshot_le_below_first_errors() {
        let mut t = FoldTree::new();
        t.insert_snapshot(snap(10, 0, 0)).unwrap();
        let err = t.nearest_snapshot_le(5).unwrap_err();
        assert_eq!(err, FoldTreeError::NoSnapshotForHeight(5));
    }

    #[test]
    fn append_steps_in_chain_succeeds() {
        let mut t = FoldTree::new();
        t.append_step(step(0, 4, 0xA)).unwrap();
        t.append_step(step(4, 8, 0xB)).unwrap();
        t.append_step(step(8, 16, 0xC)).unwrap();
        assert_eq!(t.step_count(), 3);
        assert_eq!(t.head_height, 16);
    }

    #[test]
    fn append_non_monotone_step_rejected() {
        let mut t = FoldTree::new();
        t.append_step(step(0, 4, 0xA)).unwrap();
        let err = t.append_step(step(2, 6, 0xB)).unwrap_err();
        assert!(matches!(err, FoldTreeError::NonMonotoneStep { .. }));
    }

    #[test]
    fn append_zero_length_step_rejected() {
        let mut t = FoldTree::new();
        let err = t.append_step(step(5, 5, 0xA)).unwrap_err();
        assert!(matches!(err, FoldTreeError::NonMonotoneStep { .. }));
    }

    #[test]
    fn steps_in_range_returns_relevant_slice() {
        let mut t = FoldTree::new();
        t.append_step(step(0, 4, 0xA)).unwrap();
        t.append_step(step(4, 8, 0xB)).unwrap();
        t.append_step(step(8, 12, 0xC)).unwrap();
        let s = t.steps_in_range(4, 12).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].from_height, 4);
        assert_eq!(s[1].to_height, 12);
    }

    #[test]
    fn steps_in_range_zero_when_from_eq_to() {
        let t = FoldTree::new();
        let s = t.steps_in_range(5, 5).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn steps_in_range_with_gap_errors() {
        let mut t = FoldTree::new();
        t.append_step(step(0, 4, 0xA)).unwrap();
        // Skip 4..8.
        t.append_step(step(8, 12, 0xC)).unwrap();
        let err = t.steps_in_range(0, 12).unwrap_err();
        assert!(matches!(err, FoldTreeError::BrokenChain { .. }));
    }
}

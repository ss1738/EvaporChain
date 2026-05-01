//! `PhasedNullifierTree` — bounded sliding-window nullifier store.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use thiserror::Error;

pub type Nullifier = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhasedNullifierTree {
    /// Number of live phases retained. Total state is bounded by
    /// `window_depth × max_per_phase`.
    pub window_depth: usize,
    /// Index of the current (write-target) phase. Monotone.
    pub current_phase: u64,
    /// Live phases, oldest at front. `window.back()` is `current_phase`'s set.
    pub window: VecDeque<BTreeSet<Nullifier>>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PntError {
    #[error("window_depth must be >= 1")]
    ZeroDepth,
    #[error("nullifier {n:?} already in window — double-spend rejected")]
    DoubleSpend { n: Nullifier },
}

impl PhasedNullifierTree {
    pub fn new(window_depth: usize) -> Result<Self, PntError> {
        if window_depth == 0 {
            return Err(PntError::ZeroDepth);
        }
        let mut window = VecDeque::with_capacity(window_depth);
        window.push_back(BTreeSet::new());
        Ok(Self {
            window_depth,
            current_phase: 0,
            window,
        })
    }

    /// True iff `n` is recorded in any live phase.
    pub fn is_spent_in_window(&self, n: &Nullifier) -> bool {
        self.window.iter().any(|set| set.contains(n))
    }

    /// Insert a fresh nullifier into the *current* phase. Rejects
    /// double-spends within the live window.
    pub fn insert_nullifier(&mut self, n: Nullifier) -> Result<(), PntError> {
        if self.is_spent_in_window(&n) {
            return Err(PntError::DoubleSpend { n });
        }
        self.window.back_mut().unwrap().insert(n);
        Ok(())
    }

    /// Open a fresh phase. If we're at `window_depth`, drop the
    /// oldest. `current_phase` is incremented.
    pub fn advance_phase(&mut self) {
        self.current_phase = self.current_phase.saturating_add(1);
        if self.window.len() >= self.window_depth {
            self.window.pop_front();
        }
        self.window.push_back(BTreeSet::new());
    }

    /// Total live nullifier count across all retained phases.
    pub fn live_count(&self) -> usize {
        self.window.iter().map(|s| s.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(b: u8) -> Nullifier {
        [b; 32]
    }

    #[test]
    fn zero_depth_rejected() {
        assert_eq!(
            PhasedNullifierTree::new(0).unwrap_err(),
            PntError::ZeroDepth
        );
    }

    #[test]
    fn fresh_tree_no_spends() {
        let t = PhasedNullifierTree::new(3).unwrap();
        assert!(!t.is_spent_in_window(&n(1)));
        assert_eq!(t.live_count(), 0);
    }

    #[test]
    fn insert_records_and_blocks_double_spend() {
        let mut t = PhasedNullifierTree::new(3).unwrap();
        t.insert_nullifier(n(1)).unwrap();
        assert!(t.is_spent_in_window(&n(1)));
        let err = t.insert_nullifier(n(1)).unwrap_err();
        assert!(matches!(err, PntError::DoubleSpend { .. }));
    }

    #[test]
    fn advance_phase_keeps_old_nullifier_live() {
        let mut t = PhasedNullifierTree::new(3).unwrap();
        t.insert_nullifier(n(1)).unwrap();
        t.advance_phase();
        // Still in window — depth 3 means we keep the last 3 phases.
        assert!(t.is_spent_in_window(&n(1)));
    }

    #[test]
    fn nullifier_drops_when_window_rotates_past_it() {
        let mut t = PhasedNullifierTree::new(2).unwrap();
        t.insert_nullifier(n(1)).unwrap(); // phase 0
        t.advance_phase(); // window = [phase 0, phase 1]
        assert!(t.is_spent_in_window(&n(1)));
        t.advance_phase(); // window = [phase 1, phase 2]; phase 0 dropped
        assert!(!t.is_spent_in_window(&n(1)));
        // It can be re-inserted now (the chain "forgot" it).
        t.insert_nullifier(n(1)).unwrap();
    }

    #[test]
    fn live_count_bounded_by_window() {
        let mut t = PhasedNullifierTree::new(2).unwrap();
        for i in 0..5u8 {
            t.insert_nullifier(n(i)).unwrap();
        }
        // All 5 in current phase; window = [phase 0 (5 entries)].
        assert_eq!(t.live_count(), 5);
        t.advance_phase();
        for i in 5..8u8 {
            t.insert_nullifier(n(i)).unwrap();
        }
        // window = [phase 0 (5), phase 1 (3)] → 8 live.
        assert_eq!(t.live_count(), 8);
        t.advance_phase();
        // window = [phase 1 (3), phase 2 (0)] → 3 live; phase 0's 5
        // nullifiers were forgotten.
        assert_eq!(t.live_count(), 3);
    }
}

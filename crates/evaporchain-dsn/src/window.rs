//! `DsnWindow` — sliding window of accumulators.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use thiserror::Error;

use crate::accumulator::Accumulator;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DsnWindow {
    pub window_depth: usize,
    pub current_window: u64,
    pub window: VecDeque<Accumulator>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DsnError {
    #[error("window_depth must be >= 1")]
    ZeroDepth,
}

impl DsnWindow {
    pub fn new(window_depth: usize) -> Result<Self, DsnError> {
        if window_depth == 0 {
            return Err(DsnError::ZeroDepth);
        }
        let mut window = VecDeque::with_capacity(window_depth);
        window.push_back(Accumulator::empty());
        Ok(Self {
            window_depth,
            current_window: 0,
            window,
        })
    }

    /// Fold a nullifier into the current window's accumulator.
    pub fn fold_nullifier(&mut self, nullifier: &[u8; 32]) {
        let cur = self.window.back().copied().unwrap_or(Accumulator::empty());
        let new = cur.fold(nullifier);
        *self.window.back_mut().unwrap() = new;
    }

    /// Open a fresh window. Drops the oldest if at depth.
    pub fn advance_window(&mut self) {
        self.current_window = self.current_window.saturating_add(1);
        if self.window.len() >= self.window_depth {
            self.window.pop_front();
        }
        self.window.push_back(Accumulator::empty());
    }

    /// Total count of nullifiers across all live windows.
    pub fn total_count(&self) -> u64 {
        self.window.iter().map(|a| a.count).sum()
    }

    /// Aggregate accumulator over the live window — domain-separated
    /// hash of all per-window roots in order. Bounded size = 32 bytes
    /// regardless of how many nullifiers have been folded.
    pub fn aggregate_root(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(b"evaporchain-dsn-window-root");
        h.update(&(self.window.len() as u64).to_le_bytes());
        for acc in &self.window {
            h.update(&acc.value);
            h.update(&acc.count.to_le_bytes());
        }
        *h.finalize().as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(b: u8) -> [u8; 32] {
        [b; 32]
    }

    #[test]
    fn zero_depth_rejected() {
        assert_eq!(DsnWindow::new(0).unwrap_err(), DsnError::ZeroDepth);
    }

    #[test]
    fn fresh_window_zero_count() {
        let w = DsnWindow::new(3).unwrap();
        assert_eq!(w.total_count(), 0);
    }

    #[test]
    fn fold_increments_count_and_changes_root() {
        let mut w = DsnWindow::new(3).unwrap();
        let r0 = w.aggregate_root();
        w.fold_nullifier(&n(1));
        assert_eq!(w.total_count(), 1);
        assert_ne!(w.aggregate_root(), r0);
    }

    #[test]
    fn advance_drops_oldest_at_capacity() {
        let mut w = DsnWindow::new(2).unwrap();
        w.fold_nullifier(&n(1));
        w.advance_window();
        w.fold_nullifier(&n(2));
        assert_eq!(w.total_count(), 2);
        w.advance_window();
        // Third window opens; oldest dropped → counts {0+1+0}? No:
        // window_depth=2 means window VecDeque length stays at 2:
        // [phase1(1 entry), phase2(0 entries)]. Phase 0 dropped.
        assert_eq!(w.total_count(), 1);
    }

    #[test]
    fn aggregate_root_size_is_constant() {
        let mut w = DsnWindow::new(3).unwrap();
        for i in 0..1000u32 {
            let mut nullifier = [0u8; 32];
            nullifier[..4].copy_from_slice(&i.to_le_bytes());
            w.fold_nullifier(&nullifier);
        }
        // No matter how many nullifiers, the root is always 32 bytes.
        let _: [u8; 32] = w.aggregate_root();
    }
}

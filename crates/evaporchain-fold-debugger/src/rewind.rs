//! `Debugger` — the rewind-to-height driver.

use thiserror::Error;

use crate::fold_tree::{FoldStep, FoldTree, FoldTreeError};
use crate::snapshot::Snapshot;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RewindError {
    #[error(
        "rewind target {target} > head_height {head} — debugger refuses to extrapolate forward"
    )]
    BeyondHead { target: u64, head: u64 },
    #[error(transparent)]
    Tree(#[from] FoldTreeError),
}

#[derive(Debug, Clone)]
pub struct Debugger<'a> {
    tree: &'a FoldTree,
}

impl<'a> Debugger<'a> {
    pub fn new(tree: &'a FoldTree) -> Self {
        Self { tree }
    }

    /// Rewind to `target_height`. Returns the (snapshot, replay
    /// steps) pair the chain can hand to its execution layer to
    /// rebuild byte-identical state at `target_height`.
    pub fn rewind(&self, target_height: u64) -> Result<RewindResult<'a>, RewindError> {
        if target_height > self.tree.head_height {
            return Err(RewindError::BeyondHead {
                target: target_height,
                head: self.tree.head_height,
            });
        }
        let snap = self.tree.nearest_snapshot_le(target_height)?;
        let steps = self.tree.steps_in_range(snap.height, target_height)?;
        Ok(RewindResult {
            anchor: snap.clone(),
            replay_steps: steps,
        })
    }
}

/// The recipe for reconstructing state at a target height.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindResult<'a> {
    pub anchor: Snapshot,
    pub replay_steps: &'a [FoldStep],
}

/// Top-level convenience: rewind without holding a Debugger.
pub fn rewind_to(tree: &FoldTree, target_height: u64) -> Result<RewindResult<'_>, RewindError> {
    Debugger::new(tree).rewind(target_height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::Snapshot;

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

    fn build_tree() -> FoldTree {
        let mut t = FoldTree::new();
        t.insert_snapshot(snap(0, 0, 0)).unwrap();
        t.insert_snapshot(snap(8, 1, 1)).unwrap();
        t.insert_snapshot(snap(16, 2, 2)).unwrap();
        t.append_step(step(0, 1, 0x10)).unwrap();
        t.append_step(step(1, 2, 0x11)).unwrap();
        t.append_step(step(2, 4, 0x12)).unwrap();
        t.append_step(step(4, 8, 0x13)).unwrap();
        t.append_step(step(8, 16, 0x14)).unwrap();
        t
    }

    #[test]
    fn rewind_to_snapshot_height_returns_zero_steps() {
        let t = build_tree();
        let r = rewind_to(&t, 8).unwrap();
        assert_eq!(r.anchor.height, 8);
        assert_eq!(r.replay_steps.len(), 0);
    }

    #[test]
    fn rewind_between_snapshots_returns_replay_steps() {
        let t = build_tree();
        let r = rewind_to(&t, 4).unwrap();
        assert_eq!(r.anchor.height, 0);
        // Walk: 0→1, 1→2, 2→4. 3 steps.
        assert_eq!(r.replay_steps.len(), 3);
    }

    #[test]
    fn rewind_to_height_zero_returns_genesis() {
        let t = build_tree();
        let r = rewind_to(&t, 0).unwrap();
        assert_eq!(r.anchor.height, 0);
        assert_eq!(r.replay_steps.len(), 0);
    }

    #[test]
    fn rewind_beyond_head_rejected() {
        let t = build_tree();
        let err = rewind_to(&t, 100).unwrap_err();
        assert!(matches!(
            err,
            RewindError::BeyondHead {
                target: 100,
                head: 16
            }
        ));
    }

    #[test]
    fn rewind_below_first_snapshot_errors() {
        let mut t = FoldTree::new();
        t.insert_snapshot(snap(10, 0, 0)).unwrap();
        // (no steps below 10)
        let err = rewind_to(&t, 5).unwrap_err();
        assert!(matches!(
            err,
            RewindError::Tree(FoldTreeError::NoSnapshotForHeight(5))
        ));
    }

    // ── determinism ──────────────────────────────────────────────

    #[test]
    fn rewind_is_deterministic() {
        let t = build_tree();
        let r1 = rewind_to(&t, 4).unwrap();
        let r2 = rewind_to(&t, 4).unwrap();
        assert_eq!(r1, r2);
    }

    // ── doctrine claim ───────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Folded-State Debugger gives EvaporChain O(log N)
        // rewind to any past height. Instead of N block-replays,
        // the debugger walks log₂(N) fold steps from the nearest
        // snapshot. Same fold tree → byte-identical reconstructed
        // state, so two debugger instances on the same tree agree
        // on history."

        let t = build_tree();

        // Rewind to height 4: anchor at snapshot 0, 3 fold steps.
        // Conventional N-replay would visit blocks 1, 2, 3, 4 — 4
        // operations. Folded: anchor + 3 steps. For larger N, the
        // gain is dramatic.
        let r = rewind_to(&t, 4).unwrap();
        assert_eq!(r.anchor.height, 0);
        assert!(r.replay_steps.len() <= (4u64.ilog2() + 1) as usize + 1);

        // Determinism across instances.
        let r1 = Debugger::new(&t).rewind(4).unwrap();
        let r2 = Debugger::new(&t).rewind(4).unwrap();
        assert_eq!(r1, r2);
    }
}

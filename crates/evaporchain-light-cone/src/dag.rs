//! `LightCone` — the partial-order DAG of blocks.
//!
//! Insert-only (blocks are immutable once produced). `causal_past(b)`
//! and `causal_future(b)` are reachability sets in the parent / child
//! DAGs respectively, computed by BFS.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;

use crate::block::{Block, BlockId};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LightCone {
    blocks: BTreeMap<BlockId, Block>,
    /// Inverse adjacency for `causal_future` — `children[id]` is the
    /// set of blocks that name `id` as a parent.
    children: BTreeMap<BlockId, BTreeSet<BlockId>>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LightConeError {
    #[error("block {0:?} already inserted")]
    AlreadyInserted(BlockId),
    #[error("block {block:?} references missing parent {parent:?}")]
    MissingParent { block: BlockId, parent: BlockId },
}

impl LightCone {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn get(&self, id: &BlockId) -> Option<&Block> {
        self.blocks.get(id)
    }

    pub fn contains(&self, id: &BlockId) -> bool {
        self.blocks.contains_key(id)
    }

    /// Iterate every block id known to the DAG, in `BTreeMap` (sorted)
    /// order. Used by consumers that need to scan all blocks (e.g. the
    /// `is_maximal_antichain` brute-force candidate sweep).
    pub fn ids(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.blocks.keys().copied()
    }

    /// Phase 1.1 of `LIGHT_CONE_FULL_DAG_PLAN.md` — DAG **leaves**:
    /// blocks with no children. These are the candidate tips a
    /// DAG-aware fork-choice scores to pick the chain head. Returned
    /// in `BTreeMap` (sorted) order for cross-validator determinism.
    pub fn leaves(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.blocks.keys().filter(|id| {
            self.children
                .get(*id)
                .map(|ch| ch.is_empty())
                .unwrap_or(true)
        }).copied()
    }

    /// Insert a block. All parents must already be present (this is a
    /// causal-consistency requirement; the consensus layer enforces it
    /// at network ingest).
    pub fn insert(&mut self, block: Block) -> Result<(), LightConeError> {
        if self.blocks.contains_key(&block.id) {
            return Err(LightConeError::AlreadyInserted(block.id));
        }
        for parent in &block.parents {
            if !self.blocks.contains_key(parent) {
                return Err(LightConeError::MissingParent {
                    block: block.id,
                    parent: *parent,
                });
            }
        }
        // Update inverse adjacency.
        for parent in &block.parents {
            self.children.entry(*parent).or_default().insert(block.id);
        }
        self.blocks.insert(block.id, block);
        Ok(())
    }

    /// Phase 5 of `LIGHT_CONE_FULL_DAG_PLAN.md` — cascade-prune an
    /// orphaned branch. Starting from `tip`, walks the DAG backwards
    /// removing every ancestor that is exclusively in `tip`'s
    /// causal past (not reachable via any other live branch's
    /// ancestry). Stops at the first ancestor with multiple
    /// children — that's a branch point shared with a live tip.
    ///
    /// Returns the set of pruned BlockIds (for the caller to drop
    /// matching `state_branches[id]` entries in lockstep).
    /// Idempotent: returns empty if `tip` is not in the DAG OR
    /// is not actually a leaf (has children).
    ///
    /// Phase 3.4 LRU eviction in `tendermint.rs::prune_state_branches`
    /// drops the `LightConeBranchMetadata` entry; this method is
    /// the DAG-side companion that actually trims the underlying
    /// blocks. The two are paired in Phase 5 of the plan.
    pub fn prune_orphan_branch(&mut self, tip: BlockId) -> std::collections::BTreeSet<BlockId> {
        let mut pruned = std::collections::BTreeSet::new();

        // Tip must be a current leaf — pruning an internal block
        // would break ancestry of live descendants. Phase 5 contract.
        if !self.blocks.contains_key(&tip) {
            return pruned;
        }
        if self.children.get(&tip).map(|c| !c.is_empty()).unwrap_or(false) {
            return pruned;
        }

        // Walk backwards, removing each block iff after the cleanup
        // it has no remaining descendants. Stop at branch points
        // that are still reachable from another tip.
        let mut frontier: Vec<BlockId> = vec![tip];
        while let Some(id) = frontier.pop() {
            // Skip if already gone or has surviving descendants.
            let block = match self.blocks.get(&id) {
                Some(b) => b,
                None => continue,
            };
            let has_living_children = self
                .children
                .get(&id)
                .map(|ch| ch.iter().any(|c| !pruned.contains(c)))
                .unwrap_or(false);
            if has_living_children {
                continue;
            }
            // Eligible — capture the parents before removal so we
            // can keep walking backwards.
            let parents: Vec<BlockId> = block.parents.clone();
            pruned.insert(id);
            self.blocks.remove(&id);
            self.children.remove(&id);
            // Strip the pruned id from any remaining parent's
            // children-set so the next iteration can see it as a
            // potential leaf.
            for p in &parents {
                if let Some(ch) = self.children.get_mut(p) {
                    ch.remove(&id);
                }
                frontier.push(*p);
            }
        }

        pruned
    }

    /// Re-audit (2026-05-02) Light-cone prune bound: drop any block
    /// whose `observed_epoch` is strictly less than `keep_after_epoch`,
    /// along with the inverse-adjacency edges pointing to / from it.
    /// This is the only sanctioned way to bound the DAG's memory; the
    /// `insert` path is otherwise append-only. Returns the number of
    /// blocks pruned. Caller is responsible for choosing a watermark
    /// that respects fork-choice / causal-cone consumers — typically
    /// `latest_finalized.observed_epoch - retention_window`.
    pub fn prune_before_epoch(&mut self, keep_after_epoch: u64) -> usize {
        let to_prune: Vec<BlockId> = self
            .blocks
            .iter()
            .filter(|(_, b)| b.observed_epoch < keep_after_epoch)
            .map(|(id, _)| *id)
            .collect();
        let count = to_prune.len();
        for id in &to_prune {
            // Remove the block itself.
            self.blocks.remove(id);
            // Remove its outbound edges (this id appearing as someone's parent).
            self.children.remove(id);
        }
        // Strip pruned ids from any remaining inverse-adjacency sets.
        for child_set in self.children.values_mut() {
            for id in &to_prune {
                child_set.remove(id);
            }
        }
        count
    }
}

/// All blocks (transitively) reachable via parent edges from `start`,
/// excluding `start` itself. Returns empty set if `start` is absent.
pub fn causal_past(lc: &LightCone, start: BlockId) -> BTreeSet<BlockId> {
    let mut visited: BTreeSet<BlockId> = BTreeSet::new();
    let mut queue: VecDeque<BlockId> = VecDeque::new();
    if let Some(b) = lc.get(&start) {
        for p in &b.parents {
            queue.push_back(*p);
        }
    } else {
        return visited;
    }
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(b) = lc.get(&id) {
            for p in &b.parents {
                queue.push_back(*p);
            }
        }
    }
    visited
}

/// All blocks (transitively) reachable via child edges from `start`,
/// excluding `start` itself. Returns empty set if `start` is absent.
pub fn causal_future(lc: &LightCone, start: BlockId) -> BTreeSet<BlockId> {
    let mut visited: BTreeSet<BlockId> = BTreeSet::new();
    let mut queue: VecDeque<BlockId> = VecDeque::new();
    if !lc.contains(&start) {
        return visited;
    }
    if let Some(initial_children) = lc.children.get(&start) {
        for c in initial_children {
            queue.push_back(*c);
        }
    }
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(next_children) = lc.children.get(&id) {
            for c in next_children {
                queue.push_back(*c);
            }
        }
    }
    visited
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> BlockId {
        [b; 32]
    }

    #[test]
    fn empty_dag() {
        let lc = LightCone::new();
        assert!(lc.is_empty());
        assert_eq!(causal_past(&lc, id(0)).len(), 0);
        assert_eq!(causal_future(&lc, id(0)).len(), 0);
    }

    #[test]
    fn insert_genesis_then_child() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        assert_eq!(lc.len(), 2);
        assert_eq!(causal_past(&lc, id(1)), [id(0)].into_iter().collect());
        assert_eq!(causal_future(&lc, id(0)), [id(1)].into_iter().collect());
    }

    #[test]
    fn double_insert_rejected() {
        let mut lc = LightCone::new();
        let g = Block::new(id(0), vec![], 1000, 0);
        lc.insert(g.clone()).unwrap();
        let err = lc.insert(g).unwrap_err();
        assert!(matches!(err, LightConeError::AlreadyInserted(_)));
    }

    #[test]
    fn missing_parent_rejected() {
        let mut lc = LightCone::new();
        let err = lc
            .insert(Block::new(id(1), vec![id(0)], 900, 1))
            .unwrap_err();
        assert!(matches!(err, LightConeError::MissingParent { .. }));
    }

    #[test]
    fn diamond_shape_dag() {
        // A → B, A → C, B → D, C → D (diamond)
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 800, 2))
            .unwrap();
        // D's causal past = {A, B, C}.
        assert_eq!(
            causal_past(&lc, id(3)),
            [id(0), id(1), id(2)].into_iter().collect()
        );
        // A's causal future = {B, C, D}.
        assert_eq!(
            causal_future(&lc, id(0)),
            [id(1), id(2), id(3)].into_iter().collect()
        );
        // B's causal past = {A}; B's causal future = {D}; C is concurrent
        // and so does NOT appear in B's past or future.
        assert_eq!(causal_past(&lc, id(1)), [id(0)].into_iter().collect());
        assert_eq!(causal_future(&lc, id(1)), [id(3)].into_iter().collect());
    }

    /// Phase 5 of `LIGHT_CONE_FULL_DAG_PLAN.md` — pruning a tip
    /// that doesn't exist returns empty + leaves the DAG unchanged.
    #[test]
    fn prune_orphan_branch_unknown_tip_noop() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        let pruned = lc.prune_orphan_branch(id(99));
        assert!(pruned.is_empty());
        assert_eq!(lc.len(), 1);
    }

    /// Phase 5 — pruning a non-leaf is rejected (would orphan
    /// downstream descendants). Locks the safety contract.
    #[test]
    fn prune_orphan_branch_rejects_non_leaf() {
        // A → B; A is not a leaf.
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        let pruned = lc.prune_orphan_branch(id(0));
        assert!(pruned.is_empty(), "non-leaf prune must be a no-op");
        assert_eq!(lc.len(), 2, "DAG unchanged");
    }

    /// Phase 5 — pruning a single-leaf chain cascades all the way
    /// to genesis. A → B → C, prune C → {A, B, C} all gone.
    #[test]
    fn prune_orphan_branch_full_cascade_on_linear_chain() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(1)], 800, 2)).unwrap();
        let pruned = lc.prune_orphan_branch(id(2));
        assert_eq!(pruned.len(), 3);
        assert!(pruned.contains(&id(0)));
        assert!(pruned.contains(&id(1)));
        assert!(pruned.contains(&id(2)));
        assert!(lc.is_empty());
    }

    /// Phase 5 — at a branch point the cascade STOPS. Y-shape:
    /// A → B, A → C. Pruning C cascades C only — A is shared with
    /// B's lineage and must NOT be pruned.
    #[test]
    fn prune_orphan_branch_stops_at_branch_point() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();
        let pruned = lc.prune_orphan_branch(id(2));
        assert_eq!(pruned.len(), 1);
        assert!(pruned.contains(&id(2)));
        assert!(lc.contains(&id(0)), "branch point A preserved");
        assert!(lc.contains(&id(1)), "sibling B preserved");
        assert!(!lc.contains(&id(2)), "pruned tip C removed");
    }

    /// Phase 5 — diamond DAG. Pruning the merge node D (a leaf)
    /// cascades back through B, C, A — all of them are exclusively
    /// in D's causal past once D is gone. (After D is removed, B
    /// and C have no children → they become orphans → A loses both
    /// its children → A becomes orphan.)
    #[test]
    fn prune_orphan_branch_diamond_cascades_fully() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 800, 2))
            .unwrap();
        let pruned = lc.prune_orphan_branch(id(3));
        assert_eq!(pruned.len(), 4, "all 4 blocks pruned: {:?}", pruned);
        assert!(lc.is_empty());
    }
}

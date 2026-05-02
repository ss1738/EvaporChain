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
}

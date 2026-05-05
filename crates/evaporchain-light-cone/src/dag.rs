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

/// Phase B.0 of `MCC_FULL_MULTI_PARENT_PLAN.md` — Lowest Common
/// Ancestor of two blocks in the DAG. The LCA is the deepest (most
/// recent) ancestor present in both `causal_past(a) ∪ {a}` and
/// `causal_past(b) ∪ {b}`. Returns `None` if either block is absent
/// from the DAG OR if no common ancestor exists.
///
/// **Why "deepest" ancestor:** for state replay, we want the closest
/// branch point — replaying from the deeper LCA is cheaper than
/// replaying from a shallower common ancestor like genesis. "Deepest"
/// is determined by `observed_epoch` (highest wins); ties are broken
/// by smallest `BlockId` for validator-determinism.
///
/// `find_lca(lc, a, a)` returns `Some(a)` (every block is its own LCA).
///
/// Pairs with `block_path_from_to` for the full replay-walk: find the
/// LCA, walk forward from LCA to the target.
pub fn find_lca(lc: &LightCone, a: BlockId, b: BlockId) -> Option<BlockId> {
    if !lc.contains(&a) || !lc.contains(&b) {
        return None;
    }
    let mut ancestors_a = causal_past(lc, a);
    ancestors_a.insert(a);
    let mut ancestors_b = causal_past(lc, b);
    ancestors_b.insert(b);
    let common: BTreeSet<BlockId> = ancestors_a
        .intersection(&ancestors_b)
        .copied()
        .collect();
    if common.is_empty() {
        return None;
    }
    let mut best: Option<(BlockId, u64)> = None;
    for id in &common {
        if let Some(blk) = lc.get(id) {
            best = match best {
                None => Some((*id, blk.observed_epoch)),
                Some((_, prev_epoch)) if blk.observed_epoch > prev_epoch => {
                    Some((*id, blk.observed_epoch))
                }
                Some((prev_id, prev_epoch))
                    if blk.observed_epoch == prev_epoch && *id < prev_id =>
                {
                    Some((*id, prev_epoch))
                }
                Some(prev) => Some(prev),
            };
        }
    }
    best.map(|(id, _)| id)
}

/// Phase B.0 of `MCC_FULL_MULTI_PARENT_PLAN.md` — first-parent path
/// from `from` (exclusive) to `to` (inclusive), in chronological
/// order. Returns `None` if `from` is not an ancestor of `to` along
/// the first-parent chain, or if either block is absent from the DAG.
///
/// `from == to` returns `Some(vec![])` (empty path; nothing to apply).
///
/// **Multi-parent semantics:** walks `parents[0]` only — matches the
/// existing `first_parent_trajectory` convention in
/// `evaporchain-consensus::fork_choice`. For Phase B.2's
/// `replay_to_head`, this is the correct semantics: roll forward
/// along the canonical first-parent chain. Multi-parent reachability
/// is the responsibility of antichain finalization (Phase 4.2,
/// already shipped), not state replay.
pub fn block_path_from_to(
    lc: &LightCone,
    from: BlockId,
    to: BlockId,
) -> Option<Vec<BlockId>> {
    if !lc.contains(&from) || !lc.contains(&to) {
        return None;
    }
    if from == to {
        return Some(vec![]);
    }
    let mut path: Vec<BlockId> = Vec::new();
    let mut visited: BTreeSet<BlockId> = BTreeSet::new();
    let mut cur = to;
    while cur != from {
        if !visited.insert(cur) {
            return None; // cycle guard (DAGs shouldn't cycle, but be safe)
        }
        path.push(cur);
        match lc.get(&cur) {
            Some(b) if !b.parents.is_empty() => {
                cur = b.parents[0];
            }
            _ => return None, // reached genesis or unknown block without finding `from`
        }
    }
    path.reverse();
    Some(path)
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

    // ── MCC Phase B.0 — find_lca + block_path_from_to ────────────────

    /// MCC Phase B.0 — every block is its own LCA.
    #[test]
    fn find_lca_self_is_self() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        assert_eq!(find_lca(&lc, id(0), id(0)), Some(id(0)));
    }

    /// MCC Phase B.0 — missing block → None.
    #[test]
    fn find_lca_missing_block_returns_none() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        assert_eq!(find_lca(&lc, id(0), id(99)), None);
        assert_eq!(find_lca(&lc, id(99), id(0)), None);
    }

    /// MCC Phase B.0 — linear chain LCA is the lower of the two.
    /// Locks the contract: when one is an ancestor of the other,
    /// the LCA is that ancestor.
    #[test]
    fn find_lca_linear_chain_is_lower_of_two() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();        // genesis
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();    // depth 1
        lc.insert(Block::new(id(2), vec![id(1)], 800, 2)).unwrap();    // depth 2
        // LCA of depth-2 and depth-1: depth-1 (the deeper of the two).
        assert_eq!(find_lca(&lc, id(2), id(1)), Some(id(1)));
        assert_eq!(find_lca(&lc, id(1), id(2)), Some(id(1)));
        // LCA of depth-2 and genesis: genesis.
        assert_eq!(find_lca(&lc, id(2), id(0)), Some(id(0)));
    }

    /// MCC Phase B.0 — diamond DAG. Two children of genesis merge
    /// into a descendant. LCA of the two children is genesis;
    /// LCA of either child with the merge node is that child.
    #[test]
    fn find_lca_diamond_dag() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();        // genesis
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();    // child A
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();    // child B (sibling of A)
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 800, 2)).unwrap(); // merge

        // LCA of A and B: genesis (the only common ancestor).
        assert_eq!(find_lca(&lc, id(1), id(2)), Some(id(0)));
        // LCA of merge and A: A (A is on the path to merge).
        assert_eq!(find_lca(&lc, id(3), id(1)), Some(id(1)));
        // LCA of merge and B: B.
        assert_eq!(find_lca(&lc, id(3), id(2)), Some(id(2)));
    }

    /// MCC Phase B.0 — when multiple common ancestors exist with
    /// different epochs, the deepest (highest observed_epoch) wins.
    /// Locks the contract for replay-cost minimisation: replaying
    /// from a deeper LCA is cheaper than from a shallower one.
    #[test]
    fn find_lca_picks_deepest_common_ancestor() {
        let mut lc = LightCone::new();
        // Linear chain: 0 → 1 → 2; then 3 forks off 2 and 4 also off 2.
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(1)], 800, 2)).unwrap();
        lc.insert(Block::new(id(3), vec![id(2)], 700, 3)).unwrap();
        lc.insert(Block::new(id(4), vec![id(2)], 700, 3)).unwrap();
        // LCA of 3 and 4 should be 2 (epoch 2) — NOT 0 or 1.
        assert_eq!(find_lca(&lc, id(3), id(4)), Some(id(2)));
    }

    /// MCC Phase B.0 — block_path_from_to: empty path when from == to.
    #[test]
    fn block_path_from_to_self_is_empty() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        assert_eq!(block_path_from_to(&lc, id(0), id(0)), Some(vec![]));
    }

    /// MCC Phase B.0 — block_path_from_to: linear ancestor → descendant.
    /// Returns the chronological path with `from` excluded, `to` included.
    #[test]
    fn block_path_from_to_linear_chain() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(1)], 800, 2)).unwrap();
        lc.insert(Block::new(id(3), vec![id(2)], 700, 3)).unwrap();

        // Path from genesis (excluded) to depth-3 (included): [1, 2, 3].
        assert_eq!(
            block_path_from_to(&lc, id(0), id(3)),
            Some(vec![id(1), id(2), id(3)])
        );
        // Path from depth-1 (excluded) to depth-3 (included): [2, 3].
        assert_eq!(
            block_path_from_to(&lc, id(1), id(3)),
            Some(vec![id(2), id(3)])
        );
    }

    /// MCC Phase B.0 — when `from` is NOT an ancestor of `to` along
    /// the first-parent chain, returns None. Locks the contract: the
    /// caller must verify ancestry (or use `find_lca` first).
    #[test]
    fn block_path_from_to_unrelated_returns_none() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap(); // sibling of 1

        // 1 is NOT an ancestor of 2 (siblings of genesis).
        assert_eq!(block_path_from_to(&lc, id(1), id(2)), None);
        assert_eq!(block_path_from_to(&lc, id(2), id(1)), None);
    }

    /// MCC Phase B.0 — missing block → None.
    #[test]
    fn block_path_from_to_missing_block_returns_none() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        assert_eq!(block_path_from_to(&lc, id(0), id(99)), None);
        assert_eq!(block_path_from_to(&lc, id(99), id(0)), None);
    }

    /// MCC Phase B.0 — composition test: find_lca + block_path_from_to
    /// give a complete replay walk for the diamond case.
    /// Locks the intended Phase B.2 usage pattern.
    #[test]
    fn find_lca_then_path_gives_replay_walk() {
        // Diamond: 0 → 1, 0 → 2, both → 3.
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 800, 2)).unwrap();

        // To replay from head=1 to head=2: LCA=0, then walk 0 → 2.
        let lca = find_lca(&lc, id(1), id(2)).unwrap();
        assert_eq!(lca, id(0));
        let path = block_path_from_to(&lc, lca, id(2)).unwrap();
        assert_eq!(path, vec![id(2)], "from genesis to id(2): apply id(2)");

        // The "rollback" half — from id(1) back to LCA — is the
        // executor's responsibility (Phase B.2). This substrate
        // gives the path; the executor walks parents of id(1) until
        // it hits LCA. block_path_from_to(lca, id(1)) reconstructs
        // the forward direction:
        let forward = block_path_from_to(&lc, lca, id(1)).unwrap();
        assert_eq!(forward, vec![id(1)]);
    }
}

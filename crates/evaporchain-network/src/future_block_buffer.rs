//! `FutureBlockBuffer` — holds blocks that arrived before their parents.
//!
//! Under V1.5 leaderless block production (`docs/proposals/
//! leaderless-block-production-v15.md` §2.3) a block references a parent
//! SET and gossips out of causal order: block `b` can reach a peer
//! before some of `b.parents` do. This buffer holds such "future"
//! blocks until every parent has been seen, then releases them —
//! cascading, so a chain of waiting blocks frees in a single sweep. A
//! TTL bounds memory against parents that never arrive (spam /
//! equivocation), and a hard capacity caps the buffer size.
//!
//! Pure data structure — keyed on 32-byte block ids, no network/DAG
//! dependency — so it is unit-testable in isolation and reusable for any
//! out-of-order block delivery, not only leaderless mode. The consensus
//! layer drives it: `offer` on receipt, `mark_seen` on DAG insert,
//! `evict_expired` once per round.

use std::collections::{HashMap, HashSet};

/// 32-byte block identifier (matches the chain's block id).
pub type BlockId = [u8; 32];

/// Outcome of offering a block to the buffer.
#[derive(Debug, PartialEq, Eq)]
pub enum Offer {
    /// All parents already seen — deliver immediately, nothing buffered.
    Ready,
    /// Some parents missing — held, waiting on `missing` of them.
    Buffered { missing: usize },
    /// Buffer at capacity — dropped (caller may re-request later).
    DroppedAtCapacity,
    /// Already buffered, or the block itself is already seen — no-op.
    Duplicate,
}

#[derive(Debug, Clone)]
struct Pending {
    missing: HashSet<BlockId>,
    inserted_round: u64,
}

/// Buffer of blocks awaiting their parents.
#[derive(Debug)]
pub struct FutureBlockBuffer {
    pending: HashMap<BlockId, Pending>,
    /// Reverse index: missing-parent id -> block ids waiting on it.
    waiting_on: HashMap<BlockId, HashSet<BlockId>>,
    /// Blocks whose parents haven't all arrived within `ttl_rounds` are
    /// evicted by [`Self::evict_expired`].
    ttl_rounds: u64,
    /// Hard cap on buffered blocks (DoS / memory bound).
    cap: usize,
}

impl FutureBlockBuffer {
    pub fn new(ttl_rounds: u64, cap: usize) -> Self {
        Self {
            pending: HashMap::new(),
            waiting_on: HashMap::new(),
            ttl_rounds,
            cap,
        }
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn is_buffered(&self, id: &BlockId) -> bool {
        self.pending.contains_key(id)
    }

    /// Offer block `id` with `parents`, arriving at `now_round`.
    /// `is_seen(p)` reports whether parent `p` is already known
    /// (in the DAG). If no parent is missing → [`Offer::Ready`] (caller
    /// delivers now). Otherwise the block is buffered and indexed
    /// against its missing parents.
    pub fn offer(
        &mut self,
        id: BlockId,
        parents: &[BlockId],
        now_round: u64,
        is_seen: impl Fn(&BlockId) -> bool,
    ) -> Offer {
        if self.pending.contains_key(&id) || is_seen(&id) {
            return Offer::Duplicate;
        }
        let missing: HashSet<BlockId> = parents.iter().copied().filter(|p| !is_seen(p)).collect();
        if missing.is_empty() {
            return Offer::Ready;
        }
        if self.pending.len() >= self.cap {
            return Offer::DroppedAtCapacity;
        }
        let missing_count = missing.len();
        for p in &missing {
            self.waiting_on.entry(*p).or_default().insert(id);
        }
        self.pending.insert(
            id,
            Pending {
                missing,
                inserted_round: now_round,
            },
        );
        Offer::Buffered {
            missing: missing_count,
        }
    }

    /// Signal that block `seen_id` is now available (e.g. inserted into
    /// the DAG). Releases every buffered block whose last missing parent
    /// was `seen_id`, cascading: a released block becoming available can
    /// in turn satisfy others. Returns released ids in dependency order
    /// (a parent always precedes a child that waited on it).
    pub fn mark_seen(&mut self, seen_id: BlockId) -> Vec<BlockId> {
        let mut released = Vec::new();
        let mut frontier = vec![seen_id];
        while let Some(avail) = frontier.pop() {
            let waiters = match self.waiting_on.remove(&avail) {
                Some(w) => w,
                None => continue,
            };
            for w in waiters {
                if let Some(p) = self.pending.get_mut(&w) {
                    p.missing.remove(&avail);
                    if p.missing.is_empty() {
                        self.pending.remove(&w);
                        released.push(w);
                        // `w` is now available — its own waiters may free.
                        frontier.push(w);
                    }
                }
            }
        }
        released
    }

    /// Evict buffered blocks older than `ttl_rounds` at `now_round`
    /// (their parents never arrived). Returns evicted ids and cleans the
    /// reverse index so a late parent can't resurrect an evicted block.
    pub fn evict_expired(&mut self, now_round: u64) -> Vec<BlockId> {
        let expired: Vec<BlockId> = self
            .pending
            .iter()
            .filter(|(_, p)| now_round.saturating_sub(p.inserted_round) > self.ttl_rounds)
            .map(|(id, _)| *id)
            .collect();
        for id in &expired {
            if let Some(p) = self.pending.remove(id) {
                for parent in &p.missing {
                    if let Some(set) = self.waiting_on.get_mut(parent) {
                        set.remove(id);
                        if set.is_empty() {
                            self.waiting_on.remove(parent);
                        }
                    }
                }
            }
        }
        expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bid(n: u8) -> BlockId {
        [n; 32]
    }
    // Nothing is "seen" by default (empty DAG).
    fn none_seen(_: &BlockId) -> bool {
        false
    }

    #[test]
    fn all_parents_seen_is_ready_not_buffered() {
        let mut b = FutureBlockBuffer::new(10, 100);
        let o = b.offer(bid(2), &[bid(1)], 0, |p| *p == bid(1)); // parent seen
        assert_eq!(o, Offer::Ready);
        assert!(b.is_empty());
    }

    #[test]
    fn missing_parent_is_buffered() {
        let mut b = FutureBlockBuffer::new(10, 100);
        assert_eq!(b.offer(bid(2), &[bid(1)], 0, none_seen), Offer::Buffered { missing: 1 });
        assert_eq!(b.len(), 1);
        assert!(b.is_buffered(&bid(2)));
    }

    #[test]
    fn mark_seen_releases_waiting_block() {
        let mut b = FutureBlockBuffer::new(10, 100);
        b.offer(bid(2), &[bid(1)], 0, none_seen);
        let released = b.mark_seen(bid(1));
        assert_eq!(released, vec![bid(2)]);
        assert!(b.is_empty());
    }

    #[test]
    fn multi_parent_releases_only_when_all_arrive() {
        let mut b = FutureBlockBuffer::new(10, 100);
        b.offer(bid(3), &[bid(1), bid(2)], 0, none_seen);
        assert!(b.mark_seen(bid(1)).is_empty()); // still waiting on 2
        assert!(b.is_buffered(&bid(3)));
        assert_eq!(b.mark_seen(bid(2)), vec![bid(3)]); // now free
        assert!(b.is_empty());
    }

    #[test]
    fn release_cascades_down_a_chain() {
        // c waits on b; b waits on a. a arrives → b then c free.
        let mut b = FutureBlockBuffer::new(10, 100);
        b.offer(bid(2), &[bid(1)], 0, none_seen); // b(2) waits on a(1)
        b.offer(bid(3), &[bid(2)], 0, none_seen); // c(3) waits on b(2)
        let released = b.mark_seen(bid(1));
        assert!(b.is_empty());
        assert!(released.contains(&bid(2)) && released.contains(&bid(3)));
        // parent before child.
        let pos = |x| released.iter().position(|r| *r == x).unwrap();
        assert!(pos(bid(2)) < pos(bid(3)));
    }

    #[test]
    fn ttl_evicts_stale_blocks() {
        let mut b = FutureBlockBuffer::new(5, 100);
        b.offer(bid(2), &[bid(1)], 0, none_seen);
        assert!(b.evict_expired(5).is_empty()); // within TTL (0..=5)
        assert_eq!(b.evict_expired(6), vec![bid(2)]); // age 6 > 5 → evicted
        assert!(b.is_empty());
        // a late parent must NOT resurrect the evicted block.
        assert!(b.mark_seen(bid(1)).is_empty());
    }

    #[test]
    fn capacity_drops_excess() {
        let mut b = FutureBlockBuffer::new(10, 2);
        assert_eq!(b.offer(bid(10), &[bid(1)], 0, none_seen), Offer::Buffered { missing: 1 });
        assert_eq!(b.offer(bid(11), &[bid(2)], 0, none_seen), Offer::Buffered { missing: 1 });
        assert_eq!(b.offer(bid(12), &[bid(3)], 0, none_seen), Offer::DroppedAtCapacity);
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn duplicate_and_already_seen_are_noops() {
        let mut b = FutureBlockBuffer::new(10, 100);
        b.offer(bid(2), &[bid(1)], 0, none_seen);
        // same block offered again → Duplicate.
        assert_eq!(b.offer(bid(2), &[bid(1)], 0, none_seen), Offer::Duplicate);
        // a block whose own id is already seen → Duplicate.
        assert_eq!(b.offer(bid(9), &[bid(1)], 0, |p| *p == bid(9)), Offer::Duplicate);
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn mark_seen_unknown_id_releases_nothing() {
        let mut b = FutureBlockBuffer::new(10, 100);
        assert!(b.mark_seen(bid(42)).is_empty());
    }
}

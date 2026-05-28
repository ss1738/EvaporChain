//! Cross-shard message protocol.
//!
//! Messages between shards carry energy metadata so the router can
//! deprioritize messages to dying objects — they'll evaporate soon anyway.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};
use tracing::warn;

use super::shard_assignment::ShardId;

/// Cap on outstanding (sent-but-not-yet-acknowledged) cross-shard
/// messages in `CrossShardRouter`. The router tracks every `send()`
/// in `pending_receipts` and only removes entries on `acknowledge()`.
/// If the destination shard never returns receipts, the map grows
/// unbounded — a slow-burn DoS vector when the V1.5 sharding path
/// is wired to consensus.
///
/// 65,536 was chosen as a small enough hard ceiling (~13 MB worst
/// case at ~200 B/message) to bound memory, while large enough to
/// absorb realistic per-block cross-shard burst rates (a 256-shard
/// chain at 50 cross-shard tx per shard per block stays well under).
pub const MAX_PENDING_RECEIPTS: usize = 1 << 16;

/// Per-destination-shard queue cap. `drain_for_shard` removes the
/// queue wholesale, so a shard whose validator never drains will
/// accumulate every message bound for it. Same FIFO-eviction rule
/// applies.
pub const MAX_PER_SHARD_QUEUE: usize = 1 << 14;

/// Domain-separation tag for `CrossShardReceipt::receipt_hash` leaf
/// hashes. SH-CROSS-2: prevents cross-domain collisions where an
/// attacker could craft a hash preimage that parses as a valid
/// receipt under another EvaporChain hash domain (Verkle leaves,
/// MMR leaves, compaction-proof hashes, etc.).
pub const RECEIPT_LEAF_DST: &[u8] = b"EVAPORCHAIN_V1_CROSS_SHARD_RECEIPT_LEAF\0";

/// Domain-separation tag for `receipts_root` internal Merkle nodes.
/// SH-CROSS-2: distinct from `RECEIPT_LEAF_DST` so an attacker cannot
/// craft a leaf preimage that hashes to the same value as an internal
/// node — the H2/Verkle internal-vs-leaf attack class translated to
/// the cross-shard receipt tree.
pub const RECEIPT_INTERNAL_DST: &[u8] = b"EVAPORCHAIN_V1_CROSS_SHARD_RECEIPT_INTERNAL\0";

/// A message from one shard to another.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CrossShardMessage {
    pub id: u64,
    pub from_shard: ShardId,
    pub to_shard: ShardId,
    pub target_object: [u8; 20],
    pub payload: MessagePayload,
    pub target_energy: u64,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessagePayload {
    Transfer { from: [u8; 20], amount: u64 },
    Reference { source_object: [u8; 20] },
    Query { key: String },
    Eviction { reason: String },
}

/// Proof that a cross-shard message was processed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CrossShardReceipt {
    pub message_id: u64,
    pub from_shard: ShardId,
    pub to_shard: ShardId,
    pub success: bool,
    pub result_hash: [u8; 32],
    pub processed_at: u64,
}

impl CrossShardReceipt {
    pub fn receipt_hash(&self) -> [u8; 32] {
        let mut data = Vec::new();
        data.extend_from_slice(RECEIPT_LEAF_DST);
        data.extend_from_slice(&self.message_id.to_le_bytes());
        data.extend_from_slice(&self.from_shard.0.to_le_bytes());
        data.extend_from_slice(&self.to_shard.0.to_le_bytes());
        data.push(self.success as u8);
        data.extend_from_slice(&self.result_hash);
        *blake3::hash(&data).as_bytes()
    }
}

/// Routes cross-shard messages with energy-aware prioritization.
#[derive(Debug, Default, Clone)]
pub struct CrossShardRouter {
    queues: BTreeMap<ShardId, VecDeque<CrossShardMessage>>,
    pending_receipts: BTreeMap<u64, CrossShardMessage>,
    next_id: u64,
}

impl CrossShardRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn send(&mut self, mut msg: CrossShardMessage) -> u64 {
        msg.id = self.next_id;
        // Use saturating arithmetic to avoid debug-mode panic at the
        // u64 boundary; on real chains the next-id space is far from
        // exhausting u64 but it's still cleaner than `+= 1`.
        self.next_id = self.next_id.saturating_add(1);
        let id = msg.id;
        let to = msg.to_shard;

        // SH-CROSS-1: FIFO-evict the oldest outstanding pending receipt
        // when the cap is hit. `pending_receipts` is a `BTreeMap<u64, _>`
        // ordered by ascending send-time id, so `pop_first` is exactly
        // the oldest. Operator visibility via `tracing::warn!` so a
        // pathological flood is observable (under-acknowledged
        // destination, dropped acks, or unbounded burst).
        if self.pending_receipts.len() >= MAX_PENDING_RECEIPTS {
            if let Some((evicted_id, _)) = self.pending_receipts.pop_first() {
                warn!(
                    target: "evaporchain_sharding::cross_shard",
                    evicted_id,
                    cap = MAX_PENDING_RECEIPTS,
                    "pending_receipts cap hit; evicted oldest outstanding entry"
                );
            }
        }

        // Per-destination queue cap. `drain_for_shard` deletes the
        // entry wholesale; an undrained shard could otherwise hold an
        // unbounded VecDeque. FIFO-evict from the front (oldest first)
        // so the freshest cross-shard intent survives.
        let queue = self.queues.entry(to).or_default();
        if queue.len() >= MAX_PER_SHARD_QUEUE {
            if let Some(evicted) = queue.pop_front() {
                warn!(
                    target: "evaporchain_sharding::cross_shard",
                    to_shard = to.0,
                    evicted_id = evicted.id,
                    cap = MAX_PER_SHARD_QUEUE,
                    "per-shard queue cap hit; evicted oldest queued message"
                );
            }
        }

        self.pending_receipts.insert(id, msg.clone());
        queue.push_back(msg);
        id
    }

    /// Drain messages for a shard, sorted by target energy descending
    /// (high-energy targets first — low-energy ones may evaporate).
    pub fn drain_for_shard(&mut self, shard: ShardId) -> Vec<CrossShardMessage> {
        let mut msgs: Vec<CrossShardMessage> = self
            .queues
            .remove(&shard)
            .unwrap_or_default()
            .into_iter()
            .collect();
        msgs.sort_by_key(|m| std::cmp::Reverse(m.target_energy));
        msgs
    }

    pub fn acknowledge(&mut self, receipt: CrossShardReceipt) {
        self.pending_receipts.remove(&receipt.message_id);
    }

    pub fn pending_count(&self) -> usize {
        self.pending_receipts.len()
    }

    pub fn queue_depth(&self, shard: ShardId) -> usize {
        self.queues.get(&shard).map_or(0, |q| q.len())
    }

    /// Compute Merkle root of all pending receipts for block header inclusion.
    ///
    /// SH-CROSS-2: leaf hashes carry `RECEIPT_LEAF_DST`; internal-node
    /// combines carry `RECEIPT_INTERNAL_DST`. Distinct DSTs make it
    /// infeasible for an attacker to craft a leaf preimage that
    /// collides with an internal-node hash (the Verkle internal-vs-leaf
    /// attack class translated to this Merkle tree). For single-receipt
    /// input, no internal combine runs and the root is exactly the
    /// leaf hash — `test_receipts_root_single` remains green.
    pub fn receipts_root(receipts: &[CrossShardReceipt]) -> [u8; 32] {
        if receipts.is_empty() {
            return [0u8; 32];
        }
        let mut seen_ids = HashSet::new();
        let deduped: Vec<&CrossShardReceipt> = receipts
            .iter()
            .filter(|r| seen_ids.insert(r.message_id))
            .collect();
        let hashes: Vec<[u8; 32]> = deduped.iter().map(|r| r.receipt_hash()).collect();
        let mut current = hashes;
        while current.len() > 1 {
            let mut next = Vec::new();
            for chunk in current.chunks(2) {
                let mut combined = Vec::new();
                combined.extend_from_slice(RECEIPT_INTERNAL_DST);
                combined.extend_from_slice(&chunk[0]);
                if chunk.len() > 1 {
                    combined.extend_from_slice(&chunk[1]);
                } else {
                    combined.extend_from_slice(&chunk[0]);
                }
                next.push(*blake3::hash(&combined).as_bytes());
            }
            current = next;
        }
        current[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(from: u16, to: u16, energy: u64) -> CrossShardMessage {
        CrossShardMessage {
            id: 0,
            from_shard: ShardId(from),
            to_shard: ShardId(to),
            target_object: [0u8; 20],
            payload: MessagePayload::Transfer {
                from: [1u8; 20],
                amount: 100,
            },
            target_energy: energy,
            timestamp: 1000,
        }
    }

    #[test]
    fn test_send_and_drain() {
        let mut router = CrossShardRouter::new();
        router.send(make_msg(0, 1, 500));
        router.send(make_msg(0, 1, 1000));
        let msgs = router.drain_for_shard(ShardId(1));
        assert_eq!(msgs.len(), 2);
        // High energy first
        assert!(msgs[0].target_energy >= msgs[1].target_energy);
    }

    #[test]
    fn test_drain_empty_shard() {
        let mut router = CrossShardRouter::new();
        assert!(router.drain_for_shard(ShardId(5)).is_empty());
    }

    #[test]
    fn test_pending_count() {
        let mut router = CrossShardRouter::new();
        router.send(make_msg(0, 1, 500));
        router.send(make_msg(0, 2, 500));
        assert_eq!(router.pending_count(), 2);
    }

    #[test]
    fn test_acknowledge_removes_pending() {
        let mut router = CrossShardRouter::new();
        let id = router.send(make_msg(0, 1, 500));
        let receipt = CrossShardReceipt {
            message_id: id,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            success: true,
            result_hash: [0u8; 32],
            processed_at: 1001,
        };
        router.acknowledge(receipt);
        assert_eq!(router.pending_count(), 0);
    }

    #[test]
    fn test_receipt_hash_deterministic() {
        let r = CrossShardReceipt {
            message_id: 42,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            success: true,
            result_hash: [0xAB; 32],
            processed_at: 999,
        };
        assert_eq!(r.receipt_hash(), r.receipt_hash());
    }

    #[test]
    fn test_receipts_root_empty() {
        assert_eq!(CrossShardRouter::receipts_root(&[]), [0u8; 32]);
    }

    #[test]
    fn test_receipts_root_single() {
        let r = CrossShardReceipt {
            message_id: 1,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            success: true,
            result_hash: [0u8; 32],
            processed_at: 100,
        };
        let root = CrossShardRouter::receipts_root(std::slice::from_ref(&r));
        let h = r.receipt_hash();
        assert_eq!(root, h);
    }

    #[test]
    fn test_queue_depth() {
        let mut router = CrossShardRouter::new();
        router.send(make_msg(0, 3, 100));
        router.send(make_msg(1, 3, 200));
        assert_eq!(router.queue_depth(ShardId(3)), 2);
        assert_eq!(router.queue_depth(ShardId(0)), 0);
    }

    /// SH-CROSS-2 regression: `receipt_hash` prepends
    /// `RECEIPT_LEAF_DST`. The hash must differ from a domainless
    /// BLAKE3 over the same 43-byte field concatenation, proving the
    /// DST is mixed in.
    #[test]
    fn sh_cross_2_receipt_hash_includes_leaf_dst() {
        let r = CrossShardReceipt {
            message_id: 7,
            from_shard: ShardId(1),
            to_shard: ShardId(2),
            success: true,
            result_hash: [0x55; 32],
            processed_at: 100,
        };
        let with_dst = r.receipt_hash();

        // What the hash WOULD have been pre-DST.
        let mut bare = Vec::new();
        bare.extend_from_slice(&r.message_id.to_le_bytes());
        bare.extend_from_slice(&r.from_shard.0.to_le_bytes());
        bare.extend_from_slice(&r.to_shard.0.to_le_bytes());
        bare.push(r.success as u8);
        bare.extend_from_slice(&r.result_hash);
        let without_dst = *blake3::hash(&bare).as_bytes();

        assert_ne!(
            with_dst, without_dst,
            "leaf DST must be mixed into receipt_hash"
        );
        // Pin the exact DST byte string.
        assert_eq!(
            RECEIPT_LEAF_DST,
            b"EVAPORCHAIN_V1_CROSS_SHARD_RECEIPT_LEAF\0"
        );
    }

    /// SH-CROSS-2 regression: `receipts_root` internal-node combines
    /// prepend `RECEIPT_INTERNAL_DST`. For a 2-receipt input the root
    /// is one internal-node hash; that hash must differ from a
    /// domainless BLAKE3 over the same 64-byte leaf concatenation.
    /// Pins the leaf-vs-internal separation.
    #[test]
    fn sh_cross_2_receipts_root_internal_nodes_include_internal_dst() {
        let r1 = CrossShardReceipt {
            message_id: 1,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            success: true,
            result_hash: [0xAA; 32],
            processed_at: 10,
        };
        let r2 = CrossShardReceipt {
            message_id: 2,
            from_shard: ShardId(0),
            to_shard: ShardId(2),
            success: false,
            result_hash: [0xBB; 32],
            processed_at: 20,
        };
        let root = CrossShardRouter::receipts_root(&[r1.clone(), r2.clone()]);

        // What an attacker-controlled "internal node" without DST
        // would produce.
        let mut bare = Vec::new();
        bare.extend_from_slice(&r1.receipt_hash());
        bare.extend_from_slice(&r2.receipt_hash());
        let domainless_internal = *blake3::hash(&bare).as_bytes();

        assert_ne!(
            root, domainless_internal,
            "internal DST must be mixed into the Merkle internal-node hash"
        );
        assert_eq!(
            RECEIPT_INTERNAL_DST,
            b"EVAPORCHAIN_V1_CROSS_SHARD_RECEIPT_INTERNAL\0"
        );
    }

    /// SH-CROSS-2: leaf and internal DSTs must themselves differ —
    /// the H2 attack class is "leaf preimage that hashes to an
    /// internal-node value". Using the same DST for both would
    /// silently re-introduce the collision surface.
    #[test]
    fn sh_cross_2_leaf_and_internal_dsts_are_distinct() {
        assert_ne!(
            RECEIPT_LEAF_DST, RECEIPT_INTERNAL_DST,
            "leaf and internal DSTs must be distinct or the H2 \
             attack class re-opens (attacker crafts a leaf preimage \
             whose BLAKE3 image matches an internal-node hash)"
        );
    }

    #[test]
    fn test_energy_prioritization() {
        let mut router = CrossShardRouter::new();
        router.send(make_msg(0, 1, 10)); // low energy — deprioritized
        router.send(make_msg(0, 1, 9999)); // high energy — first
        router.send(make_msg(0, 1, 500)); // medium
        let msgs = router.drain_for_shard(ShardId(1));
        assert_eq!(msgs[0].target_energy, 9999);
        assert_eq!(msgs[1].target_energy, 500);
        assert_eq!(msgs[2].target_energy, 10);
    }

    /// T1.20 — `send()` assigns sequential ids starting at 0 (lines
    /// 67-74). Existing tests use `send()` but never inspect the
    /// returned id, so the monotonic-id contract was uncovered.
    #[test]
    fn t1_20_send_returns_sequential_ids() {
        let mut router = CrossShardRouter::new();
        let id0 = router.send(make_msg(0, 1, 100));
        let id1 = router.send(make_msg(0, 1, 200));
        let id2 = router.send(make_msg(0, 2, 300));
        assert_eq!(id0, 0);
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    /// T1.20 — `drain_for_shard` removes the queue (line 82). A second
    /// drain of the same shard must return empty, not the same set.
    #[test]
    fn t1_20_drain_for_shard_clears_the_queue() {
        let mut router = CrossShardRouter::new();
        router.send(make_msg(0, 1, 100));
        router.send(make_msg(0, 1, 200));
        assert_eq!(router.drain_for_shard(ShardId(1)).len(), 2);
        assert!(
            router.drain_for_shard(ShardId(1)).is_empty(),
            "second drain must be empty (queue was removed)"
        );
        assert_eq!(router.queue_depth(ShardId(1)), 0);
    }

    /// T1.20 — `acknowledge` on an unknown message_id is a no-op
    /// (line 91: `remove` returns `Option`, ignored). Other pending
    /// receipts must NOT be disturbed.
    #[test]
    fn t1_20_acknowledge_unknown_id_is_noop() {
        let mut router = CrossShardRouter::new();
        let real_id = router.send(make_msg(0, 1, 100));
        let fake_receipt = CrossShardReceipt {
            message_id: 9_999_999,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            success: true,
            result_hash: [0u8; 32],
            processed_at: 999,
        };
        router.acknowledge(fake_receipt);
        assert_eq!(
            router.pending_count(),
            1,
            "ack on unknown id must not remove real entry"
        );
        // Sanity: acknowledging the real id still works.
        let real_receipt = CrossShardReceipt {
            message_id: real_id,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            success: true,
            result_hash: [0u8; 32],
            processed_at: 1000,
        };
        router.acknowledge(real_receipt);
        assert_eq!(router.pending_count(), 0);
    }

    /// T1.20 — `receipts_root` dedups by `message_id` (lines 107-111).
    /// A receipt repeated in the input slice must not change the root
    /// vs the single-receipt case.
    #[test]
    fn t1_20_receipts_root_dedups_by_message_id() {
        let r = CrossShardReceipt {
            message_id: 7,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            success: true,
            result_hash: [0x11; 32],
            processed_at: 100,
        };
        let single = CrossShardRouter::receipts_root(std::slice::from_ref(&r));
        let doubled = CrossShardRouter::receipts_root(&[r.clone(), r.clone()]);
        assert_eq!(
            single, doubled,
            "second copy of same message_id must be filtered"
        );
    }

    /// T1.20 — `receipts_root` for two distinct receipts builds the
    /// even-count Merkle pair (lines 116-119, the `chunk.len() > 1`
    /// branch). Distinct inputs must produce a distinct root from
    /// either single-receipt root.
    #[test]
    fn t1_20_receipts_root_two_distinct_receipts() {
        let r1 = CrossShardReceipt {
            message_id: 1,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            success: true,
            result_hash: [0xAA; 32],
            processed_at: 10,
        };
        let r2 = CrossShardReceipt {
            message_id: 2,
            from_shard: ShardId(0),
            to_shard: ShardId(2),
            success: false,
            result_hash: [0xBB; 32],
            processed_at: 20,
        };
        let root = CrossShardRouter::receipts_root(&[r1.clone(), r2.clone()]);
        let solo1 = CrossShardRouter::receipts_root(std::slice::from_ref(&r1));
        let solo2 = CrossShardRouter::receipts_root(std::slice::from_ref(&r2));
        assert_ne!(root, solo1);
        assert_ne!(root, solo2);
        assert_ne!(solo1, solo2);
    }

    /// SH-CROSS-1 regression: `pending_receipts` is bounded by
    /// `MAX_PENDING_RECEIPTS`. Sending one over the cap evicts the
    /// oldest entry (FIFO) — the count stays at the cap.
    #[test]
    fn sh_cross_1_pending_receipts_is_bounded() {
        let mut router = CrossShardRouter::new();
        // Cheaper test: directly seed pending_receipts at the cap to
        // avoid actually queueing MAX_PENDING_RECEIPTS messages.
        for i in 0..MAX_PENDING_RECEIPTS as u64 {
            router.pending_receipts.insert(
                i,
                CrossShardMessage {
                    id: i,
                    from_shard: ShardId(0),
                    to_shard: ShardId(1),
                    target_object: [0u8; 20],
                    payload: MessagePayload::Transfer {
                        from: [0u8; 20],
                        amount: 0,
                    },
                    target_energy: 0,
                    timestamp: 0,
                },
            );
        }
        router.next_id = MAX_PENDING_RECEIPTS as u64;
        assert_eq!(router.pending_receipts.len(), MAX_PENDING_RECEIPTS);

        // One more send must evict the oldest (id=0) and keep count
        // at the cap.
        let new_id = router.send(make_msg(0, 2, 100));
        assert_eq!(
            router.pending_receipts.len(),
            MAX_PENDING_RECEIPTS,
            "pending count must remain at the cap, not grow"
        );
        assert!(
            !router.pending_receipts.contains_key(&0),
            "oldest pending entry (id=0) must have been evicted"
        );
        assert!(
            router.pending_receipts.contains_key(&new_id),
            "newly sent message must be retained"
        );
    }

    /// SH-CROSS-1 regression: a single destination shard whose
    /// queue is never drained does not grow beyond
    /// `MAX_PER_SHARD_QUEUE`. Per-shard FIFO eviction at the front.
    #[test]
    fn sh_cross_1_per_shard_queue_is_bounded() {
        let mut router = CrossShardRouter::new();
        // Pre-fill the destination queue to the cap (cheaper than
        // calling send() that many times — same effect on the
        // internal queue).
        let dest = ShardId(7);
        let q = router.queues.entry(dest).or_default();
        for i in 0..MAX_PER_SHARD_QUEUE as u64 {
            q.push_back(CrossShardMessage {
                id: i,
                from_shard: ShardId(0),
                to_shard: dest,
                target_object: [0u8; 20],
                payload: MessagePayload::Transfer {
                    from: [0u8; 20],
                    amount: 0,
                },
                target_energy: 0,
                timestamp: 0,
            });
        }
        router.next_id = MAX_PER_SHARD_QUEUE as u64;
        assert_eq!(router.queue_depth(dest), MAX_PER_SHARD_QUEUE);

        // Sending one more must evict the front (oldest queued) and
        // keep queue at cap.
        router.send(make_msg(0, dest.0, 999));
        assert_eq!(
            router.queue_depth(dest),
            MAX_PER_SHARD_QUEUE,
            "per-shard queue must remain at the cap"
        );
    }

    /// SH-CROSS-1: `next_id` uses saturating arithmetic. Hitting
    /// u64::MAX must not panic in debug, and the next send must
    /// stay at u64::MAX (the ID space is effectively exhausted,
    /// not corrupted).
    #[test]
    fn sh_cross_1_next_id_saturates() {
        let mut router = CrossShardRouter::new();
        router.next_id = u64::MAX - 1;
        let id1 = router.send(make_msg(0, 1, 100));
        assert_eq!(id1, u64::MAX - 1);
        let id2 = router.send(make_msg(0, 1, 100));
        // Saturating: next_id was u64::MAX, msg.id=u64::MAX, then
        // saturating_add stays at u64::MAX.
        assert_eq!(id2, u64::MAX);
        let id3 = router.send(make_msg(0, 1, 100));
        // Third send: msg.id was u64::MAX (collision with id2);
        // pending_receipts inserts at the same key so we have one
        // entry. Documents the saturation contract — not a
        // pretty-state guarantee, just no-panic.
        assert_eq!(id3, u64::MAX);
    }

    /// T1.20 — `receipts_root` odd-count case pads the last leaf with
    /// itself (lines 120-122: `else { combined.extend_from_slice(&chunk[0]); }`).
    /// Three distinct receipts exercise the odd-count chunk branch
    /// at the leaf level.
    #[test]
    fn t1_20_receipts_root_odd_count_pads_last_leaf() {
        let mk = |id: u64, hash_byte: u8| CrossShardReceipt {
            message_id: id,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            success: true,
            result_hash: [hash_byte; 32],
            processed_at: id * 10,
        };
        let r1 = mk(1, 0x11);
        let r2 = mk(2, 0x22);
        let r3 = mk(3, 0x33);
        let root = CrossShardRouter::receipts_root(&[r1.clone(), r2.clone(), r3.clone()]);
        // Must be deterministic.
        let root_again = CrossShardRouter::receipts_root(&[r1.clone(), r2.clone(), r3.clone()]);
        assert_eq!(root, root_again);
        // Must differ from the 2-receipt root (the odd-count padding
        // changes the tree shape vs even-count).
        let root2 = CrossShardRouter::receipts_root(&[r1, r2]);
        assert_ne!(root, root2);
    }
}

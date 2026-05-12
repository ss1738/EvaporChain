//! Cross-shard message protocol.
//!
//! Messages between shards carry energy metadata so the router can
//! deprioritize messages to dying objects — they'll evaporate soon anyway.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};

use super::shard_assignment::ShardId;

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
        self.next_id += 1;
        let id = msg.id;
        let to = msg.to_shard;
        self.pending_receipts.insert(id, msg.clone());
        self.queues.entry(to).or_default().push_back(msg);
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
        assert!(router.drain_for_shard(ShardId(1)).is_empty(),
            "second drain must be empty (queue was removed)");
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
        let root_again =
            CrossShardRouter::receipts_root(&[r1.clone(), r2.clone(), r3.clone()]);
        assert_eq!(root, root_again);
        // Must differ from the 2-receipt root (the odd-count padding
        // changes the tree shape vs even-count).
        let root2 = CrossShardRouter::receipts_root(&[r1, r2]);
        assert_ne!(root, root2);
    }
}

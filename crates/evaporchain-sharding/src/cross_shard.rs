//! Cross-shard message protocol.
//!
//! Messages between shards carry energy metadata so the router can
//! deprioritize messages to dying objects — they'll evaporate soon anyway.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

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
#[derive(Debug, Default)]
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
        msgs.sort_by(|a, b| b.target_energy.cmp(&a.target_energy));
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
        let hashes: Vec<[u8; 32]> = receipts.iter().map(|r| r.receipt_hash()).collect();
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
        let root = CrossShardRouter::receipts_root(&[r.clone()]);
        // Single receipt: root = hash(receipt_hash || receipt_hash)
        let h = r.receipt_hash();
        let mut combined = Vec::new();
        combined.extend_from_slice(&h);
        combined.extend_from_slice(&h);
        assert_eq!(root, *blake3::hash(&combined).as_bytes());
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
        router.send(make_msg(0, 1, 10));   // low energy — deprioritized
        router.send(make_msg(0, 1, 9999)); // high energy — first
        router.send(make_msg(0, 1, 500));  // medium
        let msgs = router.drain_for_shard(ShardId(1));
        assert_eq!(msgs[0].target_energy, 9999);
        assert_eq!(msgs[1].target_energy, 500);
        assert_eq!(msgs[2].target_energy, 10);
    }
}

//! Bridge between the node's execution layer and the sharding subsystem.
//!
//! Tracks per-shard health metrics during block execution, identifies
//! compaction candidates, and routes cross-shard messages.

use evaporchain_sharding::{
    ShardConfig, ShardId, ShardHealth,
    CompactionCandidate, ShardCompactionProof,
    CrossShardMessage, CrossShardReceipt, CrossShardRouter,
    compact_shard, shard_for_object,
    compaction::find_candidates,
};
use std::collections::HashMap;

pub struct ShardBridge {
    config: ShardConfig,
    router: CrossShardRouter,
    shard_metrics: HashMap<u16, ShardHealthTracker>,
    compaction_energy_threshold: u64,
}

struct ShardHealthTracker {
    total_objects: u64,
    live_objects: u64,
    total_energy: u64,
    energy_sum_for_avg: u64,
    objects_for_avg: u64,
}

impl ShardHealthTracker {
    fn new() -> Self {
        Self {
            total_objects: 0,
            live_objects: 0,
            total_energy: 0,
            energy_sum_for_avg: 0,
            objects_for_avg: 0,
        }
    }

    fn to_health(&self, shard_id: ShardId) -> ShardHealth {
        let avg_half_life = if self.objects_for_avg > 0 {
            self.energy_sum_for_avg / self.objects_for_avg
        } else {
            0
        };
        ShardHealth {
            shard_id,
            total_objects: self.total_objects,
            live_objects: self.live_objects,
            total_energy: self.total_energy,
            avg_half_life,
        }
    }
}

impl ShardBridge {
    pub fn new(num_shards: u16) -> Self {
        Self {
            config: ShardConfig::new(num_shards),
            router: CrossShardRouter::new(),
            shard_metrics: HashMap::new(),
            compaction_energy_threshold: 100,
        }
    }

    pub fn shard_for(&self, object_id: &[u8; 20]) -> ShardId {
        shard_for_object(object_id, &self.config)
    }

    pub fn record_object(&mut self, object_id: &[u8; 20], energy: u64, half_life: u64, alive: bool) {
        let shard = self.shard_for(object_id);
        let tracker = self.shard_metrics.entry(shard.0).or_insert_with(ShardHealthTracker::new);
        tracker.total_objects += 1;
        if alive {
            tracker.live_objects += 1;
            tracker.total_energy += energy;
        }
        tracker.energy_sum_for_avg += half_life;
        tracker.objects_for_avg += 1;
    }

    pub fn reset_metrics(&mut self) {
        self.shard_metrics.clear();
    }

    pub fn shard_healths(&self) -> Vec<ShardHealth> {
        self.shard_metrics
            .iter()
            .map(|(&id, tracker)| tracker.to_health(ShardId(id)))
            .collect()
    }

    pub fn find_compaction_candidates(&self) -> Vec<CompactionCandidate> {
        let healths = self.shard_healths();
        find_candidates(&healths, self.compaction_energy_threshold)
    }

    pub fn compact(&self, candidate: &CompactionCandidate) -> ShardCompactionProof {
        let health = self.shard_metrics.get(&candidate.shard.0)
            .map(|t| t.to_health(candidate.shard))
            .unwrap_or(ShardHealth {
                shard_id: candidate.shard,
                total_objects: 0,
                live_objects: candidate.remaining_objects,
                total_energy: 0,
                avg_half_life: 0,
            });
        compact_shard(candidate.shard, candidate.merge_into, &health)
    }

    pub fn send_message(&mut self, msg: CrossShardMessage) -> u64 {
        self.router.send(msg)
    }

    pub fn drain_shard(&mut self, shard: ShardId) -> Vec<CrossShardMessage> {
        self.router.drain_for_shard(shard)
    }

    pub fn acknowledge(&mut self, receipt: CrossShardReceipt) {
        self.router.acknowledge(receipt);
    }

    pub fn pending_messages(&self) -> usize {
        self.router.pending_count()
    }

    pub fn num_shards(&self) -> u16 {
        self.config.num_shards
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj_id(byte: u8) -> [u8; 20] {
        let mut id = [0u8; 20];
        id[0] = byte;
        id
    }

    #[test]
    fn test_shard_bridge_basic() {
        let mut bridge = ShardBridge::new(4);
        bridge.record_object(&obj_id(0), 1000, 100, true);
        bridge.record_object(&obj_id(1), 500, 50, true);
        bridge.record_object(&obj_id(2), 0, 100, false);

        let healths = bridge.shard_healths();
        assert!(!healths.is_empty());
    }

    #[test]
    fn test_deterministic_shard_assignment() {
        let bridge = ShardBridge::new(16);
        let id = obj_id(0xAB);
        let s1 = bridge.shard_for(&id);
        let s2 = bridge.shard_for(&id);
        assert_eq!(s1, s2);
    }

    #[test]
    fn test_compaction_candidates() {
        let mut bridge = ShardBridge::new(4);
        // Shard with dead objects
        let mut dead_id = [0u8; 20];
        dead_id[0] = 0;
        dead_id[1] = 0;
        bridge.record_object(&dead_id, 0, 100, false);

        let candidates = bridge.find_compaction_candidates();
        // May or may not find candidates depending on shard assignment
        // but the logic should not panic
        let _ = candidates;
    }

    #[test]
    fn test_cross_shard_routing() {
        let mut bridge = ShardBridge::new(4);
        use evaporchain_sharding::cross_shard::MessagePayload;

        let msg = CrossShardMessage {
            id: 0,
            from_shard: ShardId(0),
            to_shard: ShardId(1),
            target_object: [0u8; 20],
            payload: MessagePayload::Transfer { from: [1u8; 20], amount: 100 },
            target_energy: 5000,
            timestamp: 1000,
        };

        let id = bridge.send_message(msg);
        assert_eq!(bridge.pending_messages(), 1);

        let msgs = bridge.drain_shard(ShardId(1));
        assert_eq!(msgs.len(), 1);
    }
}

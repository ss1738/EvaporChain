//! Dead-shard compaction.
//!
//! When all objects in a shard have evaporated, the shard can be compacted
//! (merged into its neighbor). This is unique to EvaporChain — thermodynamic
//! decay guarantees that dead shards have zero dangling cross-shard references.

use serde::{Deserialize, Serialize};
use super::shard_assignment::ShardId;

/// Health metrics for a shard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardHealth {
    pub shard_id: ShardId,
    pub total_objects: u64,
    pub live_objects: u64,
    pub total_energy: u64,
    pub avg_half_life: u64,
}

impl ShardHealth {
    pub fn liveness_ratio(&self) -> f64 {
        if self.total_objects == 0 {
            return 0.0;
        }
        self.live_objects as f64 / self.total_objects as f64
    }

    pub fn is_dead(&self) -> bool {
        self.live_objects == 0 && self.total_energy == 0
    }

    pub fn is_cold(&self, energy_threshold: u64) -> bool {
        self.total_energy <= energy_threshold
    }
}

/// A shard identified as a compaction candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionCandidate {
    pub shard: ShardId,
    pub merge_into: ShardId,
    pub reason: CompactionReason,
    pub remaining_objects: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompactionReason {
    AllEvaporated,
    BelowEnergyThreshold { total_energy: u64, threshold: u64 },
}

/// Proof that a shard compaction was valid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardCompactionProof {
    pub source_shard: ShardId,
    pub target_shard: ShardId,
    pub objects_reassigned: u64,
    pub energy_at_compaction: u64,
    pub proof_hash: [u8; 32],
}

impl ShardCompactionProof {
    pub fn compute_hash(&self) -> [u8; 32] {
        let mut data = Vec::new();
        data.extend_from_slice(&self.source_shard.0.to_le_bytes());
        data.extend_from_slice(&self.target_shard.0.to_le_bytes());
        data.extend_from_slice(&self.objects_reassigned.to_le_bytes());
        data.extend_from_slice(&self.energy_at_compaction.to_le_bytes());
        *blake3::hash(&data).as_bytes()
    }
}

/// Find compaction candidates from a set of shard health metrics.
pub fn find_candidates(
    healths: &[ShardHealth],
    energy_threshold: u64,
) -> Vec<CompactionCandidate> {
    let mut candidates = Vec::new();
    for health in healths {
        if health.is_dead() {
            let merge_into = neighbor_shard(health.shard_id);
            candidates.push(CompactionCandidate {
                shard: health.shard_id,
                merge_into,
                reason: CompactionReason::AllEvaporated,
                remaining_objects: 0,
            });
        } else if health.is_cold(energy_threshold) && health.live_objects > 0 {
            let merge_into = neighbor_shard(health.shard_id);
            candidates.push(CompactionCandidate {
                shard: health.shard_id,
                merge_into,
                reason: CompactionReason::BelowEnergyThreshold {
                    total_energy: health.total_energy,
                    threshold: energy_threshold,
                },
                remaining_objects: health.live_objects,
            });
        }
    }
    candidates
}

/// Compact a dead shard: produces a proof of valid compaction.
pub fn compact_shard(
    source: ShardId,
    target: ShardId,
    health: &ShardHealth,
) -> ShardCompactionProof {
    let mut proof = ShardCompactionProof {
        source_shard: source,
        target_shard: target,
        objects_reassigned: health.live_objects,
        energy_at_compaction: health.total_energy,
        proof_hash: [0u8; 32],
    };
    proof.proof_hash = proof.compute_hash();
    proof
}

/// Get the neighbor shard for merging (XOR with 1 — pairs shards 0↔1, 2↔3, etc.).
fn neighbor_shard(shard: ShardId) -> ShardId {
    ShardId(shard.0 ^ 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_health(id: u16, live: u64, energy: u64) -> ShardHealth {
        ShardHealth {
            shard_id: ShardId(id),
            total_objects: live + 10,
            live_objects: live,
            total_energy: energy,
            avg_half_life: 100,
        }
    }

    #[test]
    fn test_dead_shard_detection() {
        let h = ShardHealth {
            shard_id: ShardId(0),
            total_objects: 100,
            live_objects: 0,
            total_energy: 0,
            avg_half_life: 0,
        };
        assert!(h.is_dead());
    }

    #[test]
    fn test_live_shard_not_dead() {
        let h = make_health(0, 10, 5000);
        assert!(!h.is_dead());
    }

    #[test]
    fn test_cold_shard_detection() {
        let h = make_health(0, 5, 50);
        assert!(h.is_cold(100));
        assert!(!h.is_cold(10));
    }

    #[test]
    fn test_liveness_ratio() {
        let h = ShardHealth {
            shard_id: ShardId(0),
            total_objects: 100,
            live_objects: 25,
            total_energy: 1000,
            avg_half_life: 50,
        };
        assert!((h.liveness_ratio() - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_liveness_ratio_empty() {
        let h = ShardHealth {
            shard_id: ShardId(0),
            total_objects: 0,
            live_objects: 0,
            total_energy: 0,
            avg_half_life: 0,
        };
        assert_eq!(h.liveness_ratio(), 0.0);
    }

    #[test]
    fn test_find_candidates_dead() {
        let healths = vec![
            make_health(0, 10, 5000),
            ShardHealth {
                shard_id: ShardId(1),
                total_objects: 50,
                live_objects: 0,
                total_energy: 0,
                avg_half_life: 0,
            },
        ];
        let candidates = find_candidates(&healths, 100);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].shard, ShardId(1));
        assert_eq!(candidates[0].merge_into, ShardId(0)); // XOR neighbor
    }

    #[test]
    fn test_find_candidates_cold() {
        let healths = vec![make_health(2, 3, 50)];
        let candidates = find_candidates(&healths, 100);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].remaining_objects, 3);
    }

    #[test]
    fn test_no_candidates_healthy() {
        let healths = vec![make_health(0, 100, 50000)];
        assert!(find_candidates(&healths, 100).is_empty());
    }

    #[test]
    fn test_compact_shard_produces_proof() {
        let health = make_health(4, 2, 30);
        let proof = compact_shard(ShardId(4), ShardId(5), &health);
        assert_eq!(proof.source_shard, ShardId(4));
        assert_eq!(proof.target_shard, ShardId(5));
        assert_eq!(proof.objects_reassigned, 2);
        assert_ne!(proof.proof_hash, [0u8; 32]);
    }

    #[test]
    fn test_compact_proof_hash_deterministic() {
        let health = make_health(0, 5, 100);
        let p1 = compact_shard(ShardId(0), ShardId(1), &health);
        let p2 = compact_shard(ShardId(0), ShardId(1), &health);
        assert_eq!(p1.proof_hash, p2.proof_hash);
    }

    #[test]
    fn test_neighbor_shard_pairing() {
        assert_eq!(neighbor_shard(ShardId(0)), ShardId(1));
        assert_eq!(neighbor_shard(ShardId(1)), ShardId(0));
        assert_eq!(neighbor_shard(ShardId(2)), ShardId(3));
        assert_eq!(neighbor_shard(ShardId(3)), ShardId(2));
    }
}

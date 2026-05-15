//! Dead-shard compaction.
//!
//! When all objects in a shard have evaporated, the shard can be compacted
//! (merged into its neighbor). This is unique to EvaporChain — thermodynamic
//! decay guarantees that dead shards have zero dangling cross-shard references.

use super::shard_assignment::ShardId;
use serde::{Deserialize, Serialize};

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

/// Domain-separation tag for `ShardCompactionProof::compute_hash`.
///
/// SH-COMPACT-2: prevents cross-domain collisions where another
/// hash input — `CrossShardReceipt::receipt_hash`,
/// `EnergyVerkleTrie` leaf hashes, MMR nullifier leaves, etc. —
/// could be parsed by an attacker as a valid compaction proof
/// (or vice versa). Matches the DST pattern already in use for
/// `bls_portable` PoP, MMR (H4), `deriveAddress` (L2), and the
/// HashToCurve domain (L6/L7).
///
/// The trailing NUL is intentional: it forces every well-formed
/// input to begin with a 38-byte fixed prefix that no other
/// EvaporChain hash domain can naturally produce.
pub const SHARD_COMPACTION_PROOF_DST: &[u8] = b"EVAPORCHAIN_V1_SHARD_COMPACTION_PROOF\0";

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
        data.extend_from_slice(SHARD_COMPACTION_PROOF_DST);
        data.extend_from_slice(&self.source_shard.0.to_le_bytes());
        data.extend_from_slice(&self.target_shard.0.to_le_bytes());
        data.extend_from_slice(&self.objects_reassigned.to_le_bytes());
        data.extend_from_slice(&self.energy_at_compaction.to_le_bytes());
        *blake3::hash(&data).as_bytes()
    }
}

/// Find compaction candidates from a set of shard health metrics.
pub fn find_candidates(healths: &[ShardHealth], energy_threshold: u64) -> Vec<CompactionCandidate> {
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

    /// SH-COMPACT-2 regression: `compute_hash` prepends
    /// `SHARD_COMPACTION_PROOF_DST` to the BLAKE3 input. The hash
    /// must differ from a domainless BLAKE3 over the same fields,
    /// proving the DST is actually mixed in. Without DST another
    /// BLAKE3 hash over the same 4-field tuple shape could
    /// collide (e.g. a future cross-shard receipt that happened
    /// to share the field layout).
    #[test]
    fn sh_compact_2_compute_hash_includes_dst() {
        let proof = ShardCompactionProof {
            source_shard: ShardId(3),
            target_shard: ShardId(2),
            objects_reassigned: 17,
            energy_at_compaction: 42,
            proof_hash: [0u8; 32],
        };
        let with_dst = proof.compute_hash();

        // What the hash WOULD have been pre-DST.
        let mut bare = Vec::new();
        bare.extend_from_slice(&proof.source_shard.0.to_le_bytes());
        bare.extend_from_slice(&proof.target_shard.0.to_le_bytes());
        bare.extend_from_slice(&proof.objects_reassigned.to_le_bytes());
        bare.extend_from_slice(&proof.energy_at_compaction.to_le_bytes());
        let without_dst = *blake3::hash(&bare).as_bytes();

        assert_ne!(
            with_dst, without_dst,
            "DST must be mixed into the hash — domainless input would collide"
        );

        // Pin the DST value itself so a refactor that renames the
        // constant is forced to update this test, surfacing the
        // protocol-version change.
        assert_eq!(
            SHARD_COMPACTION_PROOF_DST,
            b"EVAPORCHAIN_V1_SHARD_COMPACTION_PROOF\0"
        );
    }

    #[test]
    fn test_neighbor_shard_pairing() {
        assert_eq!(neighbor_shard(ShardId(0)), ShardId(1));
        assert_eq!(neighbor_shard(ShardId(1)), ShardId(0));
        assert_eq!(neighbor_shard(ShardId(2)), ShardId(3));
        assert_eq!(neighbor_shard(ShardId(3)), ShardId(2));
    }

    /// T1.20 — `is_dead` requires BOTH `live_objects == 0` AND
    /// `total_energy == 0` (line 29). The existing `test_dead_shard_detection`
    /// only covers the both-zero positive case. This pins the AND
    /// semantics by exercising each leaky-half case: a shard with no
    /// live objects but residual energy is NOT dead (energy may still
    /// be redeemable via on_evaporate hooks); a shard with energy=0
    /// but a live object is NOT dead either (the object is in the
    /// grace window).
    #[test]
    fn t1_20_is_dead_requires_both_zero() {
        let energy_but_no_live = ShardHealth {
            shard_id: ShardId(0),
            total_objects: 10,
            live_objects: 0,
            total_energy: 500,
            avg_half_life: 100,
        };
        assert!(
            !energy_but_no_live.is_dead(),
            "residual energy must keep the shard non-dead"
        );
        let live_but_no_energy = ShardHealth {
            shard_id: ShardId(0),
            total_objects: 10,
            live_objects: 1,
            total_energy: 0,
            avg_half_life: 100,
        };
        assert!(
            !live_but_no_energy.is_dead(),
            "a live object (grace-window) keeps the shard non-dead"
        );
    }

    /// T1.20 — `is_cold` uses `<=` so exact-threshold energy is cold
    /// (line 33). Boundary pin.
    #[test]
    fn t1_20_is_cold_at_exact_threshold() {
        let h = make_health(0, 5, 100);
        assert!(h.is_cold(100), "exact threshold must be cold (<= boundary)");
        // One unit over → not cold.
        let h_over = make_health(0, 5, 101);
        assert!(!h_over.is_cold(100), "one unit over must not be cold");
    }

    /// T1.20 — `find_candidates` in a mixed-health batch must classify
    /// each shard independently and only emit candidates for dead /
    /// cold-with-live cases. Tests the loop body's branching across
    /// all three states (dead → AllEvaporated, cold-with-live →
    /// BelowEnergyThreshold, healthy → nothing) in a single call.
    #[test]
    fn t1_20_find_candidates_mixed_health_states() {
        let healths = vec![
            make_health(0, 100, 50_000), // healthy → no candidate
            ShardHealth {
                shard_id: ShardId(1),
                total_objects: 50,
                live_objects: 0,
                total_energy: 0,
                avg_half_life: 0,
            }, // dead → AllEvaporated
            make_health(2, 3, 50), // cold + live → BelowEnergyThreshold
        ];
        let candidates = find_candidates(&healths, 100);
        assert_eq!(candidates.len(), 2);
        // Order-sensitive: iteration over `healths`, so shard 1 (dead)
        // is emitted before shard 2 (cold).
        assert_eq!(candidates[0].shard, ShardId(1));
        assert!(matches!(
            candidates[0].reason,
            CompactionReason::AllEvaporated
        ));
        assert_eq!(candidates[0].merge_into, ShardId(0)); // XOR neighbor
        assert_eq!(candidates[1].shard, ShardId(2));
        match candidates[1].reason {
            CompactionReason::BelowEnergyThreshold {
                total_energy,
                threshold,
            } => {
                assert_eq!(total_energy, 50);
                assert_eq!(threshold, 100);
            }
            ref other => panic!("expected BelowEnergyThreshold, got {other:?}"),
        }
    }

    /// T1.20 — `compute_hash` must commit to every field on the
    /// proof. The existing `test_compact_proof_hash_deterministic`
    /// only checks determinism for one input; this varies each field
    /// independently so a future refactor that dropped one would
    /// silently collapse distinct proofs.
    #[test]
    fn t1_20_compact_proof_hash_distinguishes_each_field() {
        let base = ShardCompactionProof {
            source_shard: ShardId(4),
            target_shard: ShardId(5),
            objects_reassigned: 7,
            energy_at_compaction: 100,
            proof_hash: [0u8; 32],
        };
        let base_hash = base.compute_hash();
        // Vary source_shard.
        let mut diff = base.clone();
        diff.source_shard = ShardId(8);
        assert_ne!(base_hash, diff.compute_hash(), "source_shard must be in digest");
        // Vary target_shard.
        let mut diff = base.clone();
        diff.target_shard = ShardId(99);
        assert_ne!(base_hash, diff.compute_hash(), "target_shard must be in digest");
        // Vary objects_reassigned.
        let mut diff = base.clone();
        diff.objects_reassigned = 8;
        assert_ne!(base_hash, diff.compute_hash(), "objects_reassigned must be in digest");
        // Vary energy_at_compaction.
        let mut diff = base;
        diff.energy_at_compaction = 101;
        assert_ne!(base_hash, diff.compute_hash(), "energy_at_compaction must be in digest");
    }
}

//! Deterministic shard assignment based on object ID.
//!
//! EvaporChain shards are power-of-2 partitions. An object's shard is
//! determined by the leading bits of its 20-byte ID — no hash table,
//! no directory, just a bit mask.

use serde::{Deserialize, Serialize};

/// Shard identifier — a u16 supporting up to 65536 shards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ShardId(pub u16);

impl std::fmt::Display for ShardId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "shard-{}", self.0)
    }
}

/// Shard configuration — always power-of-2 shard count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardConfig {
    pub num_shards: u16,
    pub shard_bits: u8,
}

impl ShardConfig {
    pub fn new(num_shards: u16) -> Self {
        assert!(
            num_shards > 0 && num_shards.is_power_of_two(),
            "num_shards must be a power of 2"
        );
        let shard_bits = num_shards.trailing_zeros() as u8;
        Self {
            num_shards,
            shard_bits,
        }
    }
}

/// Deterministic shard assignment: uses first 2 bytes of object_id,
/// masked to shard_bits.
pub fn shard_for_object(object_id: &[u8; 20], config: &ShardConfig) -> ShardId {
    if config.num_shards == 1 {
        return ShardId(0);
    }
    let raw = u16::from_be_bytes([object_id[0], object_id[1]]);
    let mask = config.num_shards - 1;
    ShardId(raw & mask)
}

/// Range of shards (inclusive on both ends).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardRange {
    pub start: ShardId,
    pub end: ShardId,
}

impl ShardRange {
    pub fn contains(&self, id: ShardId) -> bool {
        id.0 >= self.start.0 && id.0 <= self.end.0
    }

    pub fn len(&self) -> usize {
        (self.end.0 as usize).saturating_sub(self.start.0 as usize) + 1
    }

    pub fn is_empty(&self) -> bool {
        self.end.0 < self.start.0
    }
}

/// Assign shards to a validator using round-robin.
pub fn validator_shards(
    validator_id: u64,
    num_validators: u64,
    config: &ShardConfig,
) -> Vec<ShardId> {
    if num_validators == 0 {
        return vec![];
    }
    (0..config.num_shards)
        .filter(|s| (*s as u64) % num_validators == validator_id % num_validators)
        .map(ShardId)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shard_config_power_of_two() {
        let config = ShardConfig::new(16);
        assert_eq!(config.num_shards, 16);
        assert_eq!(config.shard_bits, 4);
    }

    #[test]
    #[should_panic]
    fn test_shard_config_non_power_of_two_panics() {
        ShardConfig::new(3);
    }

    #[test]
    fn test_shard_assignment_deterministic() {
        let config = ShardConfig::new(16);
        let id = [
            0xAB, 0xCD, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let s1 = shard_for_object(&id, &config);
        let s2 = shard_for_object(&id, &config);
        assert_eq!(s1, s2);
    }

    #[test]
    fn test_shard_assignment_different_ids() {
        let config = ShardConfig::new(256);
        let id1 = [
            0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let id2 = [
            0x00, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_ne!(
            shard_for_object(&id1, &config),
            shard_for_object(&id2, &config)
        );
    }

    #[test]
    fn test_shard_assignment_single_shard() {
        let config = ShardConfig::new(1);
        let id = [
            0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(shard_for_object(&id, &config), ShardId(0));
    }

    #[test]
    fn test_shard_assignment_within_bounds() {
        let config = ShardConfig::new(64);
        for byte in 0..=255u8 {
            let id = [
                byte, byte, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ];
            let shard = shard_for_object(&id, &config);
            assert!(shard.0 < 64, "shard {} out of bounds", shard.0);
        }
    }

    #[test]
    fn test_validator_shards_round_robin() {
        let config = ShardConfig::new(8);
        let v0 = validator_shards(0, 3, &config);
        let v1 = validator_shards(1, 3, &config);
        let v2 = validator_shards(2, 3, &config);
        // Every shard assigned to exactly one validator
        let mut all: Vec<ShardId> = [v0, v1, v2].concat();
        all.sort();
        let expected: Vec<ShardId> = (0..8).map(ShardId).collect();
        assert_eq!(all, expected);
    }

    #[test]
    fn test_validator_shards_zero_validators() {
        let config = ShardConfig::new(4);
        assert!(validator_shards(0, 0, &config).is_empty());
    }

    #[test]
    fn test_shard_range_contains() {
        let range = ShardRange {
            start: ShardId(2),
            end: ShardId(5),
        };
        assert!(!range.contains(ShardId(1)));
        assert!(range.contains(ShardId(2)));
        assert!(range.contains(ShardId(4)));
        assert!(range.contains(ShardId(5)));
        assert!(!range.contains(ShardId(6)));
    }

    #[test]
    fn test_shard_range_len() {
        let range = ShardRange {
            start: ShardId(0),
            end: ShardId(7),
        };
        assert_eq!(range.len(), 8);
    }

    /// T1.20 — ShardId::Display format (lines 14-16). Previously
    /// unreached.
    #[test]
    fn t1_20_shard_id_display() {
        let s = ShardId(42);
        assert_eq!(format!("{}", s), "shard-42");
    }

    /// T1.20 — ShardRange::len and is_empty (lines 67-69, 63-65).
    #[test]
    fn t1_20_shard_range_len_and_empty() {
        let r = ShardRange {
            start: ShardId(2),
            end: ShardId(5),
        };
        assert_eq!(r.len(), 4);
        assert!(!r.is_empty());

        let empty = ShardRange {
            start: ShardId(5),
            end: ShardId(2),
        };
        assert!(empty.is_empty());
    }

    /// T1.20 — `ShardConfig::new(0)` panics (line 28-31 assertion).
    /// The existing `test_shard_config_non_power_of_two_panics` covers
    /// the `is_power_of_two()` half of the assertion; this pins the
    /// `num_shards > 0` half. (0 is a corner case because `0` is also
    /// not a power of two, but the assertion message attributes the
    /// failure to the power-of-2 check; the contract is that EITHER
    /// condition violation panics.)
    #[test]
    #[should_panic]
    fn t1_20_shard_config_zero_panics() {
        ShardConfig::new(0);
    }

    /// T1.20 — `shard_for_object` reads the first two ID bytes as
    /// BIG-ENDIAN (line 46). Pinning endianness explicitly prevents a
    /// silent refactor to `from_le_bytes` from re-bucketing every
    /// object on the chain.
    #[test]
    fn t1_20_shard_for_object_is_big_endian() {
        let config = ShardConfig::new(256); // mask = 0xFF → uses low byte after BE read
        // id = 0x12 0x34 ... — BE reads as 0x1234; masked & 0xFF = 0x34 = 52.
        let id = [
            0x12, 0x34, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let shard = shard_for_object(&id, &config);
        assert_eq!(
            shard,
            ShardId(0x34),
            "BE read of (0x12, 0x34) masked to 8 bits must be 0x34"
        );

        // Mirror-image LE would have masked to 0x12. Explicit
        // anti-LE assertion documents the contract.
        assert_ne!(shard, ShardId(0x12), "must NOT be LE-byte interpretation");
    }

    /// T1.20 — `validator_shards` uses `validator_id % num_validators`
    /// (line 82). A `validator_id` larger than `num_validators` must
    /// produce the same assignment as `validator_id % num_validators`.
    /// Pinning this prevents a future refactor that silently changed
    /// the modulo semantics from breaking validators with high IDs.
    #[test]
    fn t1_20_validator_shards_validator_id_modulo_wraps() {
        let config = ShardConfig::new(8);
        let mod3 = validator_shards(0, 3, &config);
        let wrap_3 = validator_shards(3, 3, &config); // 3 % 3 == 0
        let wrap_300 = validator_shards(300, 3, &config); // 300 % 3 == 0
        assert_eq!(mod3, wrap_3);
        assert_eq!(mod3, wrap_300);
        // And id=1 ≠ id=0 in the same modulo class.
        assert_ne!(validator_shards(1, 3, &config), mod3);
    }

    /// T1.20 — `ShardRange::len` boundary: `start == end` returns 1
    /// (single-shard range), not 0 (line 64: `+ 1` ensures inclusive
    /// on both ends). The existing `test_shard_range_len` covers an
    /// 8-element range; this pins the single-element corner.
    #[test]
    fn t1_20_shard_range_single_shard_has_len_one() {
        let r = ShardRange {
            start: ShardId(7),
            end: ShardId(7),
        };
        assert_eq!(r.len(), 1, "single-shard range is inclusive on both ends");
        assert!(!r.is_empty());
        assert!(r.contains(ShardId(7)));
        assert!(!r.contains(ShardId(6)));
        assert!(!r.contains(ShardId(8)));
    }
}

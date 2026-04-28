//! `Partition` — assignment of items (indices `0..n`) to shards.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type ShardId = u32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Partition {
    /// `assignments[i]` = shard id of item `i`. Length = number of items.
    pub assignments: Vec<ShardId>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PartitionError {
    #[error("partition is empty — at least one item required")]
    Empty,
    #[error("shard id {got} exceeds max_shards {max}")]
    ShardOutOfRange { got: ShardId, max: ShardId },
}

impl Partition {
    pub fn new(assignments: Vec<ShardId>) -> Result<Self, PartitionError> {
        if assignments.is_empty() {
            return Err(PartitionError::Empty);
        }
        Ok(Self { assignments })
    }

    /// Number of items.
    pub fn n(&self) -> usize {
        self.assignments.len()
    }

    /// Distinct shard ids actually used by this partition.
    pub fn shard_count(&self) -> usize {
        let mut seen = std::collections::BTreeSet::new();
        for s in &self.assignments {
            seen.insert(*s);
        }
        seen.len()
    }

    /// Items per shard, sorted by shard id.
    pub fn shards(&self) -> std::collections::BTreeMap<ShardId, Vec<usize>> {
        let mut out: std::collections::BTreeMap<ShardId, Vec<usize>> = Default::default();
        for (i, &s) in self.assignments.iter().enumerate() {
            out.entry(s).or_default().push(i);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_rejected() {
        assert!(matches!(
            Partition::new(vec![]).unwrap_err(),
            PartitionError::Empty
        ));
    }

    #[test]
    fn single_shard_partition() {
        let p = Partition::new(vec![0, 0, 0, 0]).unwrap();
        assert_eq!(p.n(), 4);
        assert_eq!(p.shard_count(), 1);
        assert_eq!(p.shards().len(), 1);
    }

    #[test]
    fn two_shard_partition() {
        let p = Partition::new(vec![0, 1, 0, 1]).unwrap();
        assert_eq!(p.shard_count(), 2);
        let shards = p.shards();
        assert_eq!(shards[&0], vec![0, 2]);
        assert_eq!(shards[&1], vec![1, 3]);
    }
}

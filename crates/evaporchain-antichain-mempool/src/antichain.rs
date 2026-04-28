//! `Antichain` — set of mutually-concurrent block ids on a `LightCone`.
//!
//! The newtype carries the *invariant* that every pair of members is
//! concurrent on the parent DAG. Construction goes through
//! [`Antichain::from_set`] which validates; every other API preserves
//! the invariant by construction.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

use evaporchain_light_cone::{is_concurrent, BlockId, LightCone};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Antichain {
    members: BTreeSet<BlockId>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AntichainError {
    #[error("blocks {a:?} and {b:?} are comparable on the DAG — antichain invariant violated")]
    Comparable { a: BlockId, b: BlockId },
    #[error("block {0:?} is absent from the LightCone")]
    Absent(BlockId),
}

impl Antichain {
    /// The empty antichain. Trivially valid on any LightCone.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Validate `members` against `lc` and wrap. Every member must be
    /// present on the DAG; every pair must be concurrent.
    pub fn from_set(
        members: BTreeSet<BlockId>,
        lc: &LightCone,
    ) -> Result<Self, AntichainError> {
        for m in &members {
            if !lc.contains(m) {
                return Err(AntichainError::Absent(*m));
            }
        }
        let v: Vec<BlockId> = members.iter().copied().collect();
        for i in 0..v.len() {
            for j in (i + 1)..v.len() {
                if !is_concurrent(lc, v[i], v[j]) {
                    return Err(AntichainError::Comparable { a: v[i], b: v[j] });
                }
            }
        }
        Ok(Self { members })
    }

    pub fn members(&self) -> &BTreeSet<BlockId> {
        &self.members
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn contains(&self, id: &BlockId) -> bool {
        self.members.contains(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_light_cone::Block;

    fn id(b: u8) -> BlockId {
        [b; 32]
    }

    fn diamond() -> LightCone {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 800, 2)).unwrap();
        lc
    }

    #[test]
    fn empty_antichain_always_valid() {
        let lc = LightCone::new();
        let a = Antichain::from_set(BTreeSet::new(), &lc).unwrap();
        assert!(a.is_empty());
    }

    #[test]
    fn singleton_always_valid() {
        let lc = diamond();
        let a = Antichain::from_set([id(0)].into_iter().collect(), &lc).unwrap();
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn concurrent_pair_is_valid_antichain() {
        let lc = diamond();
        let a =
            Antichain::from_set([id(1), id(2)].into_iter().collect(), &lc).unwrap();
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn comparable_pair_rejected() {
        let lc = diamond();
        let err = Antichain::from_set([id(0), id(1)].into_iter().collect(), &lc).unwrap_err();
        assert!(matches!(err, AntichainError::Comparable { .. }));
    }

    #[test]
    fn absent_member_rejected() {
        let lc = diamond();
        let err =
            Antichain::from_set([id(99)].into_iter().collect(), &lc).unwrap_err();
        assert!(matches!(err, AntichainError::Absent(_)));
    }
}

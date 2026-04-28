//! `Trajectory` — ordered `Vec<BlockId>` representing one candidate
//! chain trajectory through the light-cone DAG.
//!
//! Order is significant: the *trajectory* is read from oldest
//! (genesis-side) to newest (head). Validity (i.e. each `traj[i]` is a
//! parent of `traj[i+1]`) is checked by the consensus crate; this
//! newtype is intentionally a thin wrapper.

use serde::{Deserialize, Serialize};

use evaporchain_light_cone::BlockId;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Trajectory(pub Vec<BlockId>);

impl Trajectory {
    pub fn new(blocks: Vec<BlockId>) -> Self {
        Self(blocks)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &BlockId> {
        self.0.iter()
    }

    pub fn head(&self) -> Option<&BlockId> {
        self.0.last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_trajectory_has_no_head() {
        let t = Trajectory::default();
        assert!(t.is_empty());
        assert_eq!(t.head(), None);
    }

    #[test]
    fn head_is_last_block() {
        let t = Trajectory::new(vec![[0u8; 32], [1u8; 32], [2u8; 32]]);
        assert_eq!(t.len(), 3);
        assert_eq!(t.head(), Some(&[2u8; 32]));
    }
}

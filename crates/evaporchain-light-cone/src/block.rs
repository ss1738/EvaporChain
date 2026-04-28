//! `Block` + `BlockId` types for the light-cone DAG.
//!
//! Genuinely minimal — the consensus crate keeps only what the
//! partial-order math needs. Payload, signatures, etc. live in the
//! existing `evaporchain-consensus`/`evaporchain-types` block types
//! and are referenced *by id* from light-cone.

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

/// 32-byte block identifier — typically `blake3(canonical_block_bytes)`.
/// Light-cone is hash-agnostic; consumers fill the bytes.
pub type BlockId = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Block {
    pub id: BlockId,
    /// Parents in the partial order. Empty `parents` = genesis.
    pub parents: Vec<BlockId>,
    /// Block's seed energy at the epoch it was *observed* (= its native
    /// "now"). The chain-global λ-decays this from `observed_epoch`
    /// onward; see `arrow::time_arrow_holds_at`.
    pub energy: Energy,
    /// The epoch the block was observed/produced. Reads as a Lamport
    /// clock when wall-clock isn't available — see Decay-Lamport Time
    /// in INVENTION_STACK.md §4.1 #3 (separate crate, weeks 20-24).
    pub observed_epoch: u64,
}

impl Block {
    pub fn new(
        id: BlockId,
        parents: Vec<BlockId>,
        energy: Energy,
        observed_epoch: u64,
    ) -> Self {
        Self {
            id,
            parents,
            energy,
            observed_epoch,
        }
    }

    pub fn is_genesis(&self) -> bool {
        self.parents.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genesis_has_no_parents() {
        let g = Block::new([0u8; 32], vec![], 1_000, 0);
        assert!(g.is_genesis());
    }

    #[test]
    fn child_has_parents() {
        let g = Block::new([0u8; 32], vec![], 1_000, 0);
        let c = Block::new([1u8; 32], vec![g.id], 900, 1);
        assert!(!c.is_genesis());
        assert_eq!(c.parents, vec![[0u8; 32]]);
    }
}

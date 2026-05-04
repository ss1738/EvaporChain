//! `EnergyLeaf` — an MMR leaf carrying an energy and minted-at
//! epoch. The leaf hash binds value, energy, and mint-epoch so a
//! prover cannot substitute a higher energy.

use serde::{Deserialize, Serialize};

pub const LEAF_TAG: &[u8] = b"evaporchain:epa-mmr:leaf:v1\0";

/// One leaf of the MMR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnergyLeaf {
    /// 32-byte commitment to whatever the leaf represents (a
    /// transaction, a state diff, an oracle attestation). The MMR
    /// is agnostic.
    pub value_hash: [u8; 32],
    /// Current energy. The verifier's floor check uses this.
    pub energy: u64,
    /// Epoch at which the leaf was first appended. Useful for
    /// off-chain decay reconstruction; binds into the leaf hash.
    pub minted_at_epoch: u64,
}

impl EnergyLeaf {
    pub fn new(value_hash: [u8; 32], energy: u64, minted_at_epoch: u64) -> Self {
        Self {
            value_hash,
            energy,
            minted_at_epoch,
        }
    }

    pub fn hash(&self) -> [u8; 32] {
        leaf_hash(&self.value_hash, self.energy, self.minted_at_epoch)
    }
}

/// Domain-tagged leaf hash. Binds value || energy || minted_at —
/// a prover cannot present `(value, energy=very_high)` if the
/// committed leaf was minted with `energy=low`.
pub fn leaf_hash(value_hash: &[u8; 32], energy: u64, minted_at_epoch: u64) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(LEAF_TAG);
    h.update(value_hash);
    h.update(&energy.to_le_bytes());
    h.update(&minted_at_epoch.to_le_bytes());
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_hash_is_deterministic() {
        let l = EnergyLeaf::new([0xAA; 32], 1000, 100);
        assert_eq!(l.hash(), l.hash());
    }

    #[test]
    fn leaf_hash_changes_with_energy() {
        let l1 = EnergyLeaf::new([0xAA; 32], 1000, 100);
        let l2 = EnergyLeaf::new([0xAA; 32], 999, 100);
        assert_ne!(l1.hash(), l2.hash());
    }

    #[test]
    fn leaf_hash_changes_with_value() {
        let l1 = EnergyLeaf::new([0xAA; 32], 1000, 100);
        let l2 = EnergyLeaf::new([0xBB; 32], 1000, 100);
        assert_ne!(l1.hash(), l2.hash());
    }

    #[test]
    fn leaf_hash_changes_with_minted_at() {
        let l1 = EnergyLeaf::new([0xAA; 32], 1000, 100);
        let l2 = EnergyLeaf::new([0xAA; 32], 1000, 101);
        assert_ne!(l1.hash(), l2.hash());
    }

    #[test]
    fn leaf_hash_uses_domain_tag() {
        // The leaf hash with the tag MUST differ from a naive
        // BLAKE3 of the same fields without the tag.
        let l = EnergyLeaf::new([0xAA; 32], 1000, 100);
        let with = l.hash();
        let mut naive = blake3::Hasher::new();
        naive.update(&l.value_hash);
        naive.update(&l.energy.to_le_bytes());
        naive.update(&l.minted_at_epoch.to_le_bytes());
        let without: [u8; 32] = *naive.finalize().as_bytes();
        assert_ne!(with, without);
    }

    #[test]
    fn round_trip_serde() {
        let l = EnergyLeaf::new([0xAA; 32], 1000, 100);
        let s = serde_json::to_string(&l).unwrap();
        let back: EnergyLeaf = serde_json::from_str(&s).unwrap();
        assert_eq!(l, back);
    }
}

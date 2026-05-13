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

    /// T1.20 — the free `leaf_hash` function and the `EnergyLeaf::hash`
    /// method MUST produce identical output for the same inputs.
    /// Pins the delegation contract so a future divergence (one
    /// computes-with-tag, the other computes-without) is caught.
    #[test]
    fn t1_20_leaf_hash_free_fn_matches_method() {
        let value = [0x99u8; 32];
        let energy = 12_345u64;
        let minted = 67_890u64;
        let from_method = EnergyLeaf::new(value, energy, minted).hash();
        let from_fn = leaf_hash(&value, energy, minted);
        assert_eq!(from_method, from_fn);
    }

    /// T1.20 — the order of `energy` and `minted_at_epoch` in the
    /// hash input MATTERS. A refactor that silently swapped the two
    /// `update()` calls (lines 44-45) would break consensus across
    /// asymmetrically-upgraded validators. Existing
    /// `leaf_hash_changes_with_energy` / `_with_minted_at` only test
    /// each field's individual contribution.
    #[test]
    fn t1_20_leaf_hash_field_order_matters() {
        let value = [0x55u8; 32];
        // Same numeric values, swapped roles.
        let h_a = leaf_hash(&value, 100, 200);
        let h_b = leaf_hash(&value, 200, 100);
        assert_ne!(
            h_a, h_b,
            "swapping (energy, minted_at) must produce a different hash"
        );
    }

    /// T1.20 — endianness pin: `energy` and `minted_at_epoch` are
    /// hashed via `to_le_bytes()` (lines 44-45). A silent switch to
    /// `to_be_bytes` would fork the chain. Recompute the expected
    /// hash by hand under the LE convention and compare.
    #[test]
    fn t1_20_leaf_hash_endianness_is_little_endian() {
        let value = [0x33u8; 32];
        let energy: u64 = 0x0102_0304_0506_0708;
        let minted: u64 = 0x1112_1314_1516_1718;
        let computed = leaf_hash(&value, energy, minted);
        let mut expected = blake3::Hasher::new();
        expected.update(LEAF_TAG);
        expected.update(&value);
        expected.update(&energy.to_le_bytes());
        expected.update(&minted.to_le_bytes());
        let expected: [u8; 32] = *expected.finalize().as_bytes();
        assert_eq!(computed, expected);
        // Cross-check: BE recomputation must differ.
        let mut be = blake3::Hasher::new();
        be.update(LEAF_TAG);
        be.update(&value);
        be.update(&energy.to_be_bytes());
        be.update(&minted.to_be_bytes());
        let be: [u8; 32] = *be.finalize().as_bytes();
        assert_ne!(computed, be, "leaf_hash must be LE, not BE");
    }
}

//! `MeraCommitment` — the compact authenticated commitment derived from the tree.
//!
//! Structure:
//! - `root_hash`:    blake3 of the root tensor (32 bytes).
//! - `layer_hashes`: one hash per layer, covering all site tensors at that depth.
//!   Allows a light client to verify that a specific layer was produced honestly.
//! - `n_accounts`:  number of physical accounts (before padding).
//! - `depth`:       number of layers (including physical).

use crate::layer::hash_site;
use crate::tree::MeraTree;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeraCommitment {
    /// blake3 of the root tensor.
    pub root_hash: [u8; 32],
    /// Per-layer aggregate hash: blake3 of all site-hashes concatenated.
    pub layer_hashes: Vec<[u8; 32]>,
    /// Number of physical accounts (unpadded).
    pub n_accounts: usize,
    /// Total depth (including physical layer 0).
    pub depth: usize,
    /// λ half-life used to build this commitment.
    pub lambda_half_life: u64,
}

#[derive(Debug, Error)]
pub enum MeraCommitmentError {
    #[error("layer {0} out of range (depth={1})")]
    LayerOutOfRange(usize, usize),
    #[error("layer hash mismatch at depth {0}")]
    LayerHashMismatch(usize),
}

impl MeraCommitment {
    /// Derive a commitment from a built tree.
    pub fn from_tree(tree: &MeraTree) -> Self {
        let root_hash = tree.root_hash();
        let layer_hashes: Vec<[u8; 32]> = tree
            .layers
            .iter()
            .map(|sites| {
                let mut hasher = blake3::Hasher::new();
                for site in sites {
                    hasher.update(&hash_site(site));
                }
                *hasher.finalize().as_bytes()
            })
            .collect();

        Self {
            root_hash,
            layer_hashes,
            n_accounts: tree.n_accounts,
            depth: tree.depth(),
            lambda_half_life: tree.lambda_half_life,
        }
    }

    /// Verify that a given layer's hash matches this commitment.
    pub fn verify_layer_hash(
        &self,
        layer_index: usize,
        claimed_hash: &[u8; 32],
    ) -> Result<(), MeraCommitmentError> {
        let stored =
            self.layer_hashes
                .get(layer_index)
                .ok_or(MeraCommitmentError::LayerOutOfRange(
                    layer_index,
                    self.depth,
                ))?;
        if stored != claimed_hash {
            return Err(MeraCommitmentError::LayerHashMismatch(layer_index));
        }
        Ok(())
    }

    /// 32-byte summary: blake3 of (root_hash || all layer_hashes).
    /// This is what goes into the block header.
    pub fn header_bytes(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.root_hash);
        for lh in &self.layer_hashes {
            hasher.update(lh);
        }
        hasher.update(&(self.n_accounts as u64).to_le_bytes());
        hasher.update(&(self.lambda_half_life).to_le_bytes());
        *hasher.finalize().as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::MeraTree;

    fn make_commitment(energies: &[u64]) -> (MeraCommitment, MeraTree) {
        let tree = MeraTree::build(energies, 4096, 100);
        let c = MeraCommitment::from_tree(&tree);
        (c, tree)
    }

    #[test]
    fn commitment_depth_matches_tree() {
        let (c, t) = make_commitment(&[1, 2, 3, 4]);
        assert_eq!(c.depth, t.depth());
    }

    #[test]
    fn commitment_layer_count_matches_depth() {
        let (c, _) = make_commitment(&[1, 2, 3, 4]);
        assert_eq!(c.layer_hashes.len(), c.depth);
    }

    #[test]
    fn verify_layer_hash_passes() {
        let (c, _) = make_commitment(&[100, 200, 300, 400]);
        let hash = c.layer_hashes[0];
        assert!(c.verify_layer_hash(0, &hash).is_ok());
    }

    #[test]
    fn verify_layer_hash_fails_on_wrong_hash() {
        let (c, _) = make_commitment(&[100, 200, 300, 400]);
        let wrong = [0xFFu8; 32];
        assert!(c.verify_layer_hash(0, &wrong).is_err());
    }

    #[test]
    fn commitment_is_deterministic() {
        let e = vec![111u64, 222, 333, 444];
        let (c1, _) = make_commitment(&e);
        let (c2, _) = make_commitment(&e);
        assert_eq!(c1.root_hash, c2.root_hash);
        assert_eq!(c1.header_bytes(), c2.header_bytes());
    }

    #[test]
    fn single_account_change_changes_root() {
        let mut e = vec![100u64; 8];
        let (c1, _) = make_commitment(&e);
        e[3] = 999_999;
        let (c2, _) = make_commitment(&e);
        assert_ne!(c1.root_hash, c2.root_hash);
    }
}

//! Per-account MERA opening proof.
//!
//! A `MeraProof` allows a light client to verify that account `i` has energy
//! `e` without downloading the full state — only O(log N) tensors are needed.
//!
//! # Proof structure
//!
//! At each layer ℓ, account i's site tensor is recomputed from:
//!   - its leaf encoding `encode_energy(e)`, and
//!   - its sibling tensor at layer 0 (the paired account).
//!
//! The proof stores the sibling tensors at each layer, the layer tensors
//! (needed to recompute the disentangler output), and the root hash to
//! check against the commitment.
//!
//! Verification recomputes the path bottom-up and checks that the resulting
//! root hash matches `commitment.root_hash`.

use crate::commitment::MeraCommitment;
use crate::layer::MeraLayer;
use crate::tensor::encode_energy;
use crate::tree::MeraTree;
use thiserror::Error;

/// One node in the proof path: the sibling tensor at that layer.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ProofNode {
    /// Layer index.
    pub layer: usize,
    /// Position of the account's tensor within this layer.
    pub position: usize,
    /// The sibling tensor (the paired site that was coarse-grained alongside this one).
    pub sibling: Vec<f64>,
    /// Whether the account is the LEFT (even) or RIGHT (odd) of its pair.
    pub is_left: bool,
}

/// MERA opening proof for a single account.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct MeraProof {
    /// Account index in the original (unpadded) slice.
    pub account_index: usize,
    /// Path nodes from leaf (layer 0) to just below the root.
    pub path: Vec<ProofNode>,
    /// λ half-life (must match the commitment).
    pub lambda_half_life: u64,
    /// Base half-life τ₀.
    pub base_half_life: u64,
}

#[derive(Debug, Error)]
pub enum ProofVerifyError {
    #[error("account index {0} out of range (n_accounts={1})")]
    AccountOutOfRange(usize, usize),
    #[error("lambda mismatch: proof={0}, commitment={1}")]
    LambdaMismatch(u64, u64),
    #[error("root hash mismatch: recomputed proof does not match commitment")]
    RootHashMismatch,
    #[error("proof path length {0} inconsistent with commitment depth {1}")]
    PathLengthMismatch(usize, usize),
}

impl MeraProof {
    /// Generate a proof for account `index` from a pre-built tree.
    pub fn generate(tree: &MeraTree, account_index: usize) -> Self {
        let depth = tree.depth();
        let mut path = Vec::with_capacity(depth - 1);

        let mut pos = account_index;
        for layer_idx in 0..(depth - 1) {
            let layer_sites = &tree.layers[layer_idx];
            let is_left = pos.is_multiple_of(2);
            let sibling_pos = if is_left { pos + 1 } else { pos - 1 };
            let sibling = layer_sites
                .get(sibling_pos)
                .cloned()
                .unwrap_or_else(|| layer_sites[pos].clone()); // out-of-bounds: self as sibling
            path.push(ProofNode {
                layer: layer_idx,
                position: pos,
                sibling: sibling.data,
                is_left,
            });
            pos /= 2;
        }

        Self {
            account_index,
            path,
            lambda_half_life: tree.lambda_half_life,
            base_half_life: tree.base_half_life,
        }
    }

    /// Verify this proof against a commitment.
    pub fn verify(
        &self,
        account_index: usize,
        energy: u64,
        commitment: &MeraCommitment,
    ) -> Result<(), ProofVerifyError> {
        if account_index >= commitment.n_accounts {
            return Err(ProofVerifyError::AccountOutOfRange(
                account_index,
                commitment.n_accounts,
            ));
        }
        if self.lambda_half_life != commitment.lambda_half_life {
            return Err(ProofVerifyError::LambdaMismatch(
                self.lambda_half_life,
                commitment.lambda_half_life,
            ));
        }
        if self.path.len() != commitment.depth - 1 {
            return Err(ProofVerifyError::PathLengthMismatch(
                self.path.len(),
                commitment.depth,
            ));
        }

        // Start from the leaf encoding.
        let mut current: Vec<f64> = encode_energy(energy).data;

        for node in &self.path {
            let layer = MeraLayer::new(self.lambda_half_life, self.base_half_life, node.layer);
            let (left, right) = if node.is_left {
                (current.as_slice(), node.sibling.as_slice())
            } else {
                (node.sibling.as_slice(), current.as_slice())
            };
            current = layer.apply(left, right, None);
        }

        // `current` is now the recomputed root tensor.
        let recomputed_root = *blake3::hash(
            &current
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<u8>>(),
        )
        .as_bytes();

        if recomputed_root != commitment.root_hash {
            return Err(ProofVerifyError::RootHashMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commitment::MeraCommitment;
    use crate::tree::MeraTree;

    fn setup(energies: &[u64]) -> (MeraTree, MeraCommitment) {
        let tree = MeraTree::build(energies, 4096, 100);
        let c = MeraCommitment::from_tree(&tree);
        (tree, c)
    }

    #[test]
    fn proof_verify_all_accounts() {
        let energies = vec![1_000u64, 2_000, 3_000, 4_000];
        let (tree, commitment) = setup(&energies);
        for (i, &e) in energies.iter().enumerate() {
            let proof = MeraProof::generate(&tree, i);
            assert!(
                proof.verify(i, e, &commitment).is_ok(),
                "account {i} proof should verify"
            );
        }
    }

    #[test]
    fn proof_wrong_energy_fails() {
        let energies = vec![1_000u64, 2_000, 3_000, 4_000];
        let (tree, commitment) = setup(&energies);
        let proof = MeraProof::generate(&tree, 0);
        assert!(
            proof.verify(0, 9_999_999, &commitment).is_err(),
            "wrong energy should fail verification"
        );
    }

    #[test]
    fn proof_wrong_account_index_fails() {
        let energies = vec![1_000u64, 2_000, 3_000, 4_000];
        let (tree, commitment) = setup(&energies);
        let proof = MeraProof::generate(&tree, 0);
        // Use proof for account 0 but verify with account 1's energy.
        assert!(
            proof.verify(1, 2_000, &commitment).is_err(),
            "proof for wrong account should fail"
        );
    }

    #[test]
    fn proof_out_of_range_account_fails() {
        let energies = vec![1_000u64, 2_000];
        let (tree, commitment) = setup(&energies);
        let proof = MeraProof::generate(&tree, 0);
        assert!(matches!(
            proof.verify(99, 1_000, &commitment),
            Err(ProofVerifyError::AccountOutOfRange(99, 2))
        ));
    }

    #[test]
    fn proof_lambda_mismatch_fails() {
        let energies = vec![1_000u64, 2_000, 3_000, 4_000];
        let tree = MeraTree::build(&energies, 4096, 100);
        let c_different_lambda = MeraCommitment::from_tree(&MeraTree::build(&energies, 2048, 100));
        let proof = MeraProof::generate(&tree, 0);
        assert!(
            proof.verify(0, 1_000, &c_different_lambda).is_err(),
            "lambda mismatch should fail"
        );
    }

    #[test]
    fn proof_path_length_proportional_to_depth() {
        let e8 = vec![1u64; 8];
        let e16 = vec![1u64; 16];
        let (t8, _) = setup(&e8);
        let (t16, _) = setup(&e16);
        let p8 = MeraProof::generate(&t8, 0);
        let p16 = MeraProof::generate(&t16, 0);
        assert_eq!(
            p16.path.len(),
            p8.path.len() + 1,
            "each doubling of accounts adds one proof layer"
        );
    }
}

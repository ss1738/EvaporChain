//! Full MERA tree built from a slice of account energy values.
//!
//! Builds log₂(N) layers, each halving the site count, until a single
//! root tensor remains.  Stores all intermediate tensors for proof generation.

use crate::layer::{hash_site, MeraLayer};
use crate::tensor::{encode_energy, Tensor, CHI};

/// The full MERA tree for a set of account energies.
///
/// `layers[ℓ][i]` = the CHI-dimensional site tensor at layer ℓ, position i.
/// `layers[0]` = physical encoding of input energies (leaf layer).
/// `layers[depth-1]` = single root tensor.
pub struct MeraTree {
    /// Site tensors at every layer. `layers[ℓ]` has `ceil(N / 2^ℓ)` entries.
    pub layers: Vec<Vec<Tensor>>,
    /// MERA layer parameters (one per coarse-graining step).
    pub mera_layers: Vec<MeraLayer>,
    /// Number of physical accounts (before padding).
    pub n_accounts: usize,
    /// λ half-life used to build tensors.
    pub lambda_half_life: u64,
    /// Base half-life τ₀.
    pub base_half_life: u64,
}

impl MeraTree {
    /// Build a MERA tree from raw account energies.
    ///
    /// Pads to the next power of two with zero-energy phantom accounts so
    /// the binary tree is balanced.
    pub fn build(energies: &[u64], lambda_half_life: u64, base_half_life: u64) -> Self {
        let n_accounts = energies.len();
        let padded_n = energies.len().next_power_of_two().max(2);
        let depth = padded_n.trailing_zeros() as usize + 1; // +1 for physical layer

        // Physical layer: encode each energy as a CHI-vector.
        let mut physical: Vec<Tensor> = energies
            .iter()
            .map(|&e| encode_energy(e))
            .collect();
        // Pad with zero-energy phantoms.
        while physical.len() < padded_n {
            physical.push(encode_energy(0));
        }

        let mut layers: Vec<Vec<Tensor>> = vec![physical];
        let mut mera_layers: Vec<MeraLayer> = Vec::new();

        for ℓ in 0..(depth - 1) {
            let layer = MeraLayer::new(lambda_half_life, base_half_life, ℓ);
            let current = layers.last().unwrap();
            let mut next: Vec<Tensor> = Vec::with_capacity(current.len() / 2);

            // Energy threshold for filtration: accounts below E·(1/2)^ℓ are masked.
            // We use a small threshold (0.01 of unit norm) to avoid masking real accounts.
            let filter_threshold = Some(0.01f64);

            for pair in current.chunks(2) {
                let left  = &pair[0].data;
                let right = if pair.len() > 1 { &pair[1].data } else { &pair[0].data };
                let out = layer.apply(left, right, filter_threshold);
                next.push(Tensor::from_vec(out));
            }

            mera_layers.push(layer);
            layers.push(next);
        }

        Self { layers, mera_layers, n_accounts, lambda_half_life, base_half_life }
    }

    /// Root tensor (single CHI-vector at the top layer).
    pub fn root(&self) -> &Tensor {
        self.layers.last().unwrap().first().unwrap()
    }

    /// Hash of the root tensor.
    pub fn root_hash(&self) -> [u8; 32] {
        hash_site(self.root())
    }

    /// Number of layers (including physical).
    pub fn depth(&self) -> usize {
        self.layers.len()
    }

    /// Site tensor at layer `ℓ`, position `i`.
    pub fn site(&self, layer: usize, index: usize) -> Option<&Tensor> {
        self.layers.get(layer)?.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_tree(energies: &[u64]) -> MeraTree {
        MeraTree::build(energies, 4096, 100)
    }

    #[test]
    fn tree_single_account() {
        let t = default_tree(&[1_000_000]);
        assert_eq!(t.n_accounts, 1);
        assert!(t.root().data.len() == CHI);
    }

    #[test]
    fn tree_power_of_two() {
        let energies: Vec<u64> = (1..=8).map(|i| i * 1_000_000).collect();
        let t = default_tree(&energies);
        assert_eq!(t.layers[0].len(), 8);
        assert_eq!(t.layers[1].len(), 4);
        assert_eq!(t.layers[2].len(), 2);
        assert_eq!(t.layers[3].len(), 1);
    }

    #[test]
    fn tree_non_power_of_two_pads() {
        let t = default_tree(&[1, 2, 3]);
        assert_eq!(t.n_accounts, 3);
        assert_eq!(t.layers[0].len(), 4); // padded to 4
    }

    #[test]
    fn root_is_chi_vector() {
        let t = default_tree(&[100, 200, 300, 400]);
        assert_eq!(t.root().data.len(), CHI);
    }

    #[test]
    fn different_energies_different_root() {
        let t1 = default_tree(&[1_000, 2_000, 3_000, 4_000]);
        let t2 = default_tree(&[4_000, 3_000, 2_000, 1_000]);
        assert_ne!(t1.root_hash(), t2.root_hash(),
            "different orderings must produce different commitments");
    }

    #[test]
    fn same_energies_same_root() {
        let e = vec![1_000_000u64, 2_000_000, 3_000_000, 4_000_000];
        let t1 = MeraTree::build(&e, 4096, 100);
        let t2 = MeraTree::build(&e, 4096, 100);
        assert_eq!(t1.root_hash(), t2.root_hash(), "commitment must be deterministic");
    }

    #[test]
    fn different_lambda_different_root() {
        let e = vec![1_000_000u64; 4];
        let t1 = MeraTree::build(&e, 4096, 100);
        let t2 = MeraTree::build(&e, 2048, 100);
        assert_ne!(t1.root_hash(), t2.root_hash());
    }
}

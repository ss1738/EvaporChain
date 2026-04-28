//! Star tree — every leaf attached to a single internal node, edge
//! weight = `tropical_weight(leaf_energy)`. Pairwise distance from
//! leaf `i` to leaf `j` (i ≠ j) is `weight(e_i) ⊗ weight(e_j)` (tropical
//! multiplication = ordinary `+`).
//!
//! Star trees are the simplest tree-metric topology and trivially
//! satisfy the four-point condition: for any 4 leaves the three
//! pairwise sums are all equal to `Σ weights`. They are the baseline
//! tree shape every more-elaborate construction folds back onto.

use evaporchain_types::Energy;

use crate::matrix::TropicalMatrix;
use crate::scalar::TropicalScalar;
use crate::weight::tropical_weight;

/// Build the n×n pairwise-distance matrix for the star tree on
/// `energies`. Diagonal entries are left at `TropicalScalar::Infinity`
/// (a leaf has no defined "self-distance" in the star-tree topology).
pub fn star_tree_distances(energies: &[Energy]) -> TropicalMatrix {
    let n = energies.len();
    let weights: Vec<TropicalScalar> = energies.iter().copied().map(tropical_weight).collect();
    let mut m = TropicalMatrix::new(n);
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue; // leave as Infinity
            }
            m.set(i, j, weights[i].mul(weights[j]));
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_tree_is_symmetric() {
        let m = star_tree_distances(&[1, 2, 4, 8]);
        assert!(m.is_symmetric());
    }

    #[test]
    fn star_tree_diagonal_is_infinity() {
        let m = star_tree_distances(&[1, 2, 4]);
        for i in 0..3 {
            assert_eq!(m.get(i, i), TropicalScalar::Infinity);
        }
    }

    #[test]
    fn star_tree_pairwise_known_values() {
        // weights: w(1)=0, w(2)=-1, w(4)=-2
        let m = star_tree_distances(&[1, 2, 4]);
        // d_01 = w(1) + w(2) = 0 + -1 = -1
        assert_eq!(m.get(0, 1), TropicalScalar::finite(-1));
        // d_02 = w(1) + w(4) = 0 + -2 = -2
        assert_eq!(m.get(0, 2), TropicalScalar::finite(-2));
        // d_12 = w(2) + w(4) = -1 + -2 = -3
        assert_eq!(m.get(1, 2), TropicalScalar::finite(-3));
    }

    #[test]
    fn star_tree_zero_energy_leaf_pulls_distance_to_infinity() {
        // A leaf at energy 0 has weight Infinity. Tropical mul with
        // Infinity absorbs → that leaf's distances to everything else
        // are Infinity.
        let m = star_tree_distances(&[1, 0, 4]);
        assert_eq!(m.get(0, 1), TropicalScalar::Infinity);
        assert_eq!(m.get(1, 2), TropicalScalar::Infinity);
        // The non-zero pair is unaffected.
        assert_eq!(m.get(0, 2), TropicalScalar::finite(-2));
    }

    #[test]
    fn empty_input_yields_zero_dim_matrix() {
        let m = star_tree_distances(&[]);
        assert_eq!(m.dim(), 0);
    }
}

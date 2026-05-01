//! Buneman 1971 four-point condition.
//!
//! A symmetric metric `d` on `n` points is a *tree-metric* (i.e. comes
//! from a phylogenetic tree where `d_ij` is the path distance) iff for
//! every quadruple `i, j, k, l`:
//!
//! ```text
//!   max(d_ij + d_kl, d_ik + d_jl, d_il + d_jk)
//!     is achieved by at least two of the three.
//! ```
//!
//! This is the operational form of the Speyer-Sturmfels theorem: the
//! tropical Grassmannian `Gr_{2,n}` (= the Plücker variety in tropical
//! algebra) coincides with the space of n-leaf phylogenetic trees, and
//! the four-point condition is exactly the cut-out equations.
//!
//! Skipped quadruples (entries equal to `Infinity` on either side) are
//! treated as "not enforceable" — the condition is checked vacuously.

use crate::matrix::TropicalMatrix;
use crate::scalar::TropicalScalar;

/// Returns `true` iff every quadruple in `m` satisfies the four-point
/// condition.
///
/// Quadratic-in-`n` checks: O(n^4) iterations, each O(1). For the
/// typical small-`n` use here (n in the dozens) this is trivial.
pub fn satisfies_four_point(m: &TropicalMatrix) -> bool {
    let n = m.dim();
    if !m.is_symmetric() {
        return false;
    }
    for i in 0..n {
        for j in (i + 1)..n {
            for k in (j + 1)..n {
                for l in (k + 1)..n {
                    let sums = pairwise_sums(m, i, j, k, l);
                    if !max_achieved_at_least_twice(sums) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

fn pairwise_sums(
    m: &TropicalMatrix,
    i: usize,
    j: usize,
    k: usize,
    l: usize,
) -> [TropicalScalar; 3] {
    let s1 = m.get(i, j).mul(m.get(k, l));
    let s2 = m.get(i, k).mul(m.get(j, l));
    let s3 = m.get(i, l).mul(m.get(j, k));
    [s1, s2, s3]
}

fn max_achieved_at_least_twice(sums: [TropicalScalar; 3]) -> bool {
    // If any entry is Infinity, treat the check as vacuously satisfied —
    // a partial-info matrix can't be falsified at this quadruple.
    if sums.iter().any(|s| s.is_infinity()) {
        return true;
    }
    let max = *sums.iter().max().unwrap();
    sums.iter().filter(|s| **s == max).count() >= 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::star::star_tree_distances;

    #[test]
    fn star_tree_satisfies_four_point() {
        let m = star_tree_distances(&[1, 2, 4, 8, 16]);
        assert!(satisfies_four_point(&m));
    }

    #[test]
    fn three_point_matrix_trivially_satisfies() {
        // No quadruples exist, so the check vacuously passes.
        let m = star_tree_distances(&[1, 2, 4]);
        assert!(satisfies_four_point(&m));
    }

    #[test]
    fn asymmetric_matrix_rejected() {
        let mut m = TropicalMatrix::new(4);
        for i in 0..4 {
            for j in 0..4 {
                m.set(i, j, TropicalScalar::finite(if i + j == 0 { 0 } else { 1 }));
            }
        }
        // Make asymmetric.
        m.set(0, 1, TropicalScalar::finite(99));
        assert!(!satisfies_four_point(&m));
    }

    #[test]
    fn random_non_tree_matrix_rejected() {
        // Construct a 4x4 matrix where the three pairwise sums are
        // all distinct → four-point condition fails.
        let mut m = TropicalMatrix::new(4);
        // d_01=1, d_02=2, d_03=3, d_12=4, d_13=5, d_23=6
        let dists = [
            (0, 1, 1),
            (0, 2, 2),
            (0, 3, 3),
            (1, 2, 4),
            (1, 3, 5),
            (2, 3, 6),
        ];
        for (i, j, v) in dists {
            m.set(i, j, TropicalScalar::finite(v));
            m.set(j, i, TropicalScalar::finite(v));
        }
        // Diagonal: leave as Infinity.
        // Sums for (0, 1, 2, 3):
        //   d_01 + d_23 = 1 + 6 = 7
        //   d_02 + d_13 = 2 + 5 = 7
        //   d_03 + d_12 = 3 + 4 = 7
        // All equal → satisfies four-point. Bad test case; build another.
        // Re-design: use distances that produce three distinct sums.
        m.set(2, 3, TropicalScalar::finite(99));
        m.set(3, 2, TropicalScalar::finite(99));
        // Now sums:
        //   d_01 + d_23 = 1 + 99 = 100
        //   d_02 + d_13 = 2 + 5 = 7
        //   d_03 + d_12 = 3 + 4 = 7
        // Max=100, achieved once → fails.
        assert!(!satisfies_four_point(&m));
    }

    #[test]
    fn matrix_with_infinity_entries_passes_vacuously() {
        // An incomplete distance matrix shouldn't fail four-point.
        let mut m = TropicalMatrix::new(4);
        // Leave most entries as Infinity, set a few finite.
        m.set(0, 1, TropicalScalar::finite(1));
        m.set(1, 0, TropicalScalar::finite(1));
        assert!(satisfies_four_point(&m));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::star::star_tree_distances;
    use proptest::prelude::*;

    proptest! {
        /// Property: any star-tree distance matrix satisfies four-point,
        /// regardless of the leaf-energy distribution.
        #[test]
        fn star_tree_always_satisfies_four_point(
            energies in proptest::collection::vec(1u64..1_000_000, 4..10),
        ) {
            let m = star_tree_distances(&energies);
            prop_assert!(satisfies_four_point(&m));
        }
    }
}

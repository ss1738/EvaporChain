//! Tropical Plücker commitment over the chain's energy history.
//!
//! Per `research/INVENTION_STACK.md` §A1.4 (Amendment 1, far-frontier
//! math that survived the L1 shipping filter):
//!
//! > Speyer-Sturmfels 2004 (tropical Grassmannian = phylogenetic trees).
//! > Tropical Plücker coords commit to *entire tree shape* canonically,
//! > not just root. Edge weights `−log(remaining energy)` — tropical
//! > (min, +) gives multiplicative aggregation = energy-product paths.
//!
//! ## What this crate provides
//!
//! - [`scalar`] — `TropicalScalar` over the (min, +) semiring with an
//!   `Infinity` element acting as the tropical zero.
//! - [`matrix`] — `TropicalMatrix` n×n distance matrix.
//! - [`weight`] — `tropical_weight(energy) = −log_2(energy)` as a
//!   `TropicalScalar`. Integer approximation via bit-length.
//! - [`star`] — pairwise-distance matrix for the *evaporative star
//!   tree* (every leaf attached to a single internal node, edge
//!   weight = `tropical_weight(leaf_energy)`).
//! - [`four_point`] — Buneman 1971 four-point condition: a metric is a
//!   tree-metric iff for all `i, j, k, l` the three pairwise sums have
//!   their *maximum* achieved by at least two of them.
//! - [`commitment`] — `plucker_commitment(matrix) -> [u8; 32]`.
//!   Domain-separated blake3 over the canonical serialization.
//!
//! ## Why tropical (min, +) is the right algebra here
//!
//! In tropical arithmetic, `⊕ = min` and `⊗ = +`. So a tropical
//! "product" along a path is the *sum* of edge weights, and a tropical
//! "sum" of paths is the *minimum*-weight path. With edge weights
//! `−log(energy)`, the tropical product along a path is
//! `−log(∏ energies)` — the path's *aggregate energy*, log-scaled. The
//! tropical Plücker coords thus aggregate energy *multiplicatively*
//! while the chain stores everything additively in `i64`.

pub mod commitment;
pub mod four_point;
pub mod matrix;
pub mod scalar;
pub mod star;
pub mod weight;

pub use commitment::plucker_commitment;
pub use four_point::satisfies_four_point;
pub use matrix::TropicalMatrix;
pub use scalar::TropicalScalar;
pub use star::star_tree_distances;
pub use weight::tropical_weight;

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "evaporchain-tropical implements the (min, +)
    /// semiring with `Infinity` as tropical zero. Tropical add = min,
    /// tropical mul = saturating-add. ZERO_T is the additive identity
    /// and ONE_T is the multiplicative identity. The evaporative star
    /// tree's pairwise distance matrix satisfies Buneman's four-point
    /// condition and produces a deterministic Plücker commitment."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Tropical semiring axioms.
        let x = TropicalScalar::finite(42);
        // Additive identity: x ⊕ ZERO_T = x.
        assert_eq!(x.add(TropicalScalar::ZERO_T), x);
        // Multiplicative identity: x ⊗ ONE_T = x.
        assert_eq!(x.mul(TropicalScalar::ONE_T), x);
        // Add is min, mul is plus.
        assert_eq!(
            TropicalScalar::finite(3).add(TropicalScalar::finite(5)),
            TropicalScalar::finite(3)
        );
        assert_eq!(
            TropicalScalar::finite(3).mul(TropicalScalar::finite(5)),
            TropicalScalar::finite(8)
        );
        // Infinity absorbs under multiplication (tropical zero).
        assert_eq!(x.mul(TropicalScalar::ZERO_T), TropicalScalar::ZERO_T);

        // Star tree on real energies: pairwise distance matrix
        // satisfies Buneman's four-point condition.
        let energies = vec![1_000u64, 2_000, 4_000, 8_000];
        let m = star_tree_distances(&energies);
        assert!(satisfies_four_point(&m), "star tree must be a tree-metric");

        // Plücker commitment is deterministic.
        let c1 = plucker_commitment(&m);
        let c2 = plucker_commitment(&m);
        assert_eq!(c1, c2);

        // Permuting energies → different commitment (canonical
        // serialisation is order-sensitive at the leaf level).
        let permuted = vec![8_000u64, 4_000, 2_000, 1_000];
        let m_p = star_tree_distances(&permuted);
        let c_p = plucker_commitment(&m_p);
        assert_ne!(c1, c_p);
    }
}

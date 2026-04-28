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

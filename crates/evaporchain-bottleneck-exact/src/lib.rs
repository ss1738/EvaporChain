//! Exact bottleneck distance via binary-searched threshold +
//! Hopcroft-Karp bipartite matching.
//!
//! ## Problem
//!
//! Given an `n × m` non-negative integer cost matrix `C[i][j]`,
//! find the minimum threshold `T` such that the bipartite graph
//! `G_T = { (i, j) : C[i][j] ≤ T }` admits a **perfect matching**
//! that saturates `min(n, m)` vertices.
//!
//! In the persistence-diagram / Wasserstein-bottleneck setting,
//! `min_{matching}` of `max_{matched edge}` cost = the bottleneck
//! distance.
//!
//! ## Algorithm
//!
//! 1. Collect the distinct values of `C[i][j]` (sorted).
//! 2. Binary-search for the smallest threshold T* in that list
//!    such that `G_{T*}` admits a perfect matching of size
//!    `min(n, m)`.
//! 3. Hopcroft-Karp at each probe: O(E √V) per check; binary
//!    search over O(n·m) distinct values gives a total of
//!    O(E √V · log(n·m)) — polynomial in input size.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Pure-integer cost matrix.** No floats; validator-
//!    deterministic byte-equality.
//!
//! 2. **Returns exact bottleneck.** The result satisfies:
//!    `T* = min { max C[i][π(i)] : π ∈ perfect matchings }`.
//!    Tested against brute-force on small inputs.
//!
//! 3. **Empty / dimension-mismatched inputs structurally
//!    handled.** Empty matrix → 0; mismatched dimensions → error.
//!
//! ## Module map
//!
//! - [`hopcroft_karp`] — bipartite-matching algorithm.
//! - [`bottleneck`] — binary-search driver + public
//!   [`bottleneck_exact`] entry point.

pub mod bottleneck;
pub mod hopcroft_karp;

pub use bottleneck::{bottleneck_exact, BottleneckError};
pub use hopcroft_karp::{max_matching_size, MatchingGraph};

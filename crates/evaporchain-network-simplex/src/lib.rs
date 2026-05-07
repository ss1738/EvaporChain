//! Exact transportation-LP solver via successive-shortest-path.
//!
//! **Companion: V2.** `evaporchain-network-simplex-v2` is a parallel
//! implementation (also Successive-Shortest-Path with reduced-cost
//! potentials). Note: as of 2026-05-07 the V1 and V2 module-doc
//! strings disagree on what V1 ships — V1 here describes itself as
//! SSP-with-Bellman-Ford-augmenting-paths, while V2's lib.rs frames
//! V1 as a greedy minimum-cost-cell heuristic. The discrepancy is
//! pre-existing and not resolved by this companion-pointer commit;
//! readers should consult the actual implementations to settle which
//! crate matches which algorithm. Behaviourally V1 and V2 should
//! agree on optimum cost; check via the `flow_invariants_match` test
//! patterns if relying on either for production.
//!
//! ## Problem
//!
//! Given:
//! - `supplies[i]` for `i ∈ 0..n` — non-negative supply at source `i`.
//! - `demands[j]` for `j ∈ 0..m` — non-negative demand at sink `j`.
//! - `cost[i][j]` — non-negative integer cost per unit transported.
//! - `Σ supplies == Σ demands` (balanced).
//!
//! Find an integer flow plan `flow[i][j] ≥ 0` such that:
//! - `Σ_j flow[i][j] = supplies[i]` for all `i`.
//! - `Σ_i flow[i][j] = demands[j]` for all `j`.
//! - `Σ_{i,j} flow[i][j] · cost[i][j]` is minimised.
//!
//! ## Algorithm
//!
//! V1 ships the **successive-shortest-path** (SSP) variant.
//! Pseudo-polynomial: O(F · (V·E)) where F = total flow. For
//! small inputs (n+m ≤ 32, supplies in 1..100) this runs in
//! microseconds. The full revised-network-simplex pivoting
//! (for adversarial worst cases) is V2.x.
//!
//! Algorithm sketch:
//! 1. Build a bipartite flow network.
//! 2. Repeatedly find a min-cost augmenting path from an
//!    unsaturated source to an unsaturated sink (via Bellman-Ford
//!    on potentials).
//! 3. Augment by the bottleneck capacity.
//! 4. Stop when all supply is shipped.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Pure-integer cost + integer flows.** Validator-deterministic.
//!
//! 2. **Returns the exact min-cost.** Tested against brute-force
//!    on small inputs.
//!
//! 3. **Balanced supply/demand required.** Imbalanced input ⇒
//!    `Imbalanced` error. The chain pre-balances at the higher
//!    layer (e.g., padding with a dummy sink for excess supply).
//!
//! ## Module map
//!
//! - [`transport`] — [`solve_transportation`] entry point +
//!   [`TransportError`].

pub mod transport;

pub use transport::{solve_transportation, TransportError, TransportSolution};

//! Transportation-LP solver via greedy minimum-cost-cell.
//!
//! V1 ships the bipartite-case greedy: iteratively pick the (i, j)
//! cell with min cost among un-fully-satisfied pairs and ship the
//! bottleneck quantity. This is full Successive-Shortest-Path
//! collapsed to its bipartite-special case — historically named "SSP"
//! in this codebase, but mechanically a greedy. Optimal on
//! Monge-property cost matrices and on bipartite assignment; can
//! underperform on arbitrary cost matrices.
//!
//! **Companion: V2.** `evaporchain-network-simplex-v2` ships the full
//! Successive-Shortest-Path with Dijkstra over reduced-cost
//! potentials and explicit augmenting paths. V2 handles adversarial
//! cost matrices where V1's greedy underperforms (concrete test:
//! `_v2::tests::adversarial_3x3_where_greedy_underperforms`). V1 and
//! V2 are peers — V1 is fast for the well-behaved case, V2 is
//! correct for the general case.
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
//! V1 ships the **bipartite greedy minimum-cost-cell** iteration.
//! For small inputs (n+m ≤ 32, supplies in 1..100) this runs in
//! microseconds. Full SSP with Dijkstra over reduced costs (for
//! adversarial worst cases) is `evaporchain-network-simplex-v2`,
//! which is a peer crate — both V1 and V2 are live in the workspace.
//!
//! Algorithm sketch:
//! 1. Initialise residual_supply = supplies, residual_demand = demands.
//! 2. Pick (i, j) with min cost[i][j] among cells where both
//!    residual_supply[i] > 0 and residual_demand[j] > 0.
//! 3. Ship `min(residual_supply[i], residual_demand[j])` units;
//!    decrement both residuals; accumulate `units * cost[i][j]` into
//!    total_cost.
//! 4. Repeat until all residuals zero.
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

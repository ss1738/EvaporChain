//! V2 — proper Successive-Shortest-Path solver for the
//! transportation LP.
//!
//! V1 (`evaporchain-network-simplex`) ships a greedy minimum-cost-cell
//! heuristic. The V1 source itself documents that the greedy is only
//! optimal for "well-behaved (diagonal-cheap)" cost matrices — full
//! network-simplex pivoting was reserved for V2.x. This crate is
//! that V2.
//!
//! ## Algorithm
//!
//! Standard Successive-Shortest-Path with reduced-cost potentials:
//!
//! 1. Build a directed flow network with super-source `s` connected
//!    to suppliers (capacity = supply, cost 0), suppliers to
//!    demanders (capacity = ∞ effectively `total_supply`, cost
//!    `cost[i][j]`), demanders to super-sink `t` (capacity =
//!    demand, cost 0).
//! 2. Initialise potentials `π[v] = 0` (valid because all original
//!    costs are non-negative).
//! 3. Repeatedly run Dijkstra from `s` using *reduced costs*
//!    `c'(u,v) = c(u,v) + π(u) − π(v)` (always non-negative).
//! 4. Augment along the shortest s→t path by the bottleneck
//!    residual capacity.
//! 5. Update potentials: `π[v] += dist[v]` for every reachable `v`.
//! 6. Stop when no augmenting path exists. Balanced supply/demand
//!    + integer capacities ⇒ termination with all flow shipped.
//!
//! Provably optimal for non-negative integer costs and balanced
//! transportation LPs. Pseudo-polynomial: `O(F · (V·log V + E))`
//! where `F = total flow`. For chain-typical inputs (n+m ≤ 64,
//! flows ≤ 10⁶) this runs in milliseconds.
//!
//! ## Same shape as V1
//!
//! `solve_transportation` has the same signature as V1's; callers
//! can swap backends without changing call sites. Errors mirror V1's
//! variants exactly.

pub mod ssp;

pub use ssp::{solve_transportation, TransportError, TransportSolution};

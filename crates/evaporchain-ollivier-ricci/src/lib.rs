//! Ollivier-Ricci Mixing Bound — gossip-SLA via discrete curvature.
//!
//! ## What this is
//!
//! Given a graph (the validator gossip mesh), at each node we
//! attach a "lazy random walk" probability distribution: with
//! some probability stay, with the rest distribute to neighbors.
//! For an edge `(u, v)`, the **Ollivier-Ricci curvature** is:
//!
//!   κ(u, v) = 1 − W₁(μ_u, μ_v) / d(u, v)
//!
//! where W₁ is the L¹-Wasserstein (earth-mover) distance between
//! the lazy-walk distributions at `u` and `v`, and d is graph
//! distance. Positive κ ⇒ fast mixing (small spectral gap on
//! the chain); negative κ ⇒ structural bottleneck.
//!
//! ## Use as a gossip SLA
//!
//! The chain advertises a `curvature_floor` (e.g., −0.1 in
//! micros). Any edge whose computed κ falls below the floor is
//! flagged as a network-health bottleneck — its peers contribute
//! less to mixing than the chain expects from a healthy mesh.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Pure integer fixed-point curvature.** All distances and
//!    probabilities in micros (1.0 = 10⁶). Curvature itself in
//!    micros. Validator-deterministic byte-equality.
//!
//! 2. **W₁ via the dual LP relaxation, brute-force matching.**
//!    For small neighborhoods (≤ 8 each side) we enumerate the
//!    transport plan via a simple greedy + bound. V2 = full
//!    Hungarian / network-simplex.
//!
//! 3. **Curvature floor is structural, not stochastic.** A
//!    sub-floor edge is REJECTED — the chain doesn't average it
//!    away. This is what makes the SLA falsifiable: a single
//!    bottlenecked edge fails the gate.
//!
//! ## What this crate does NOT do
//!
//! - Does NOT compute the gossip mesh. Caller passes the graph
//!   adjacency.
//! - Does NOT model the chain's spectral gap directly; the
//!   curvature is a lower bound on it (Lin-Lu-Yau / Ollivier).
//! - Does NOT implement weighted graphs. V1 is unweighted.
//!
//! ## Module map
//!
//! - [`graph`] — [`Graph`] adjacency + lazy-walk distribution.
//! - [`wasserstein`] — `wasserstein_1` between two
//!   distributions.
//! - [`curvature`] — `ollivier_ricci_micros` driver +
//!   `curvature_floor_check`.

pub mod curvature;
pub mod graph;
pub mod wasserstein;

pub use curvature::{
    curvature_floor_check, ollivier_ricci_micros, CurvatureError, MICROS,
};
pub use graph::{Graph, GraphError, NodeId};
pub use wasserstein::wasserstein_1;

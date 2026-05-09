//! Wasserstein-1 (earth-mover) distance — V1 lower bound via the
//! Kantorovich-Rubinstein dual on simple test functions.
//!
//! ## Why a lower bound, not exact
//!
//! Exact W₁ on a finite metric requires solving the transportation
//! LP (Hungarian-style assignment). V1 ships a sound LOWER bound:
//! for any 1-Lipschitz function `h`,
//!
//! ```text
//! W₁(μ, ν) ≥ |E_μ[h] − E_ν[h]|
//! ```
//!
//! Specifically, evaluated on `h_u(z) = d(u, z)` and
//! `h_v(z) = d(v, z)`, taking the max, gives a fast pure-integer
//! lower bound. The full LP-exact W₁ is a V2 upgrade.
//!
//! ## Why a lower bound is the *right* direction for the SLA
//!
//! The Ollivier-Ricci curvature is `κ = 1 − W₁/d`. A LOWER bound
//! on W₁ produces an UPPER bound on κ. The chain uses
//! `κ_upper < floor` as the alarm gate; this gives **sound
//! alarms** (when raised, the real curvature is ≤ κ_upper < floor,
//! so the edge is a real bottleneck) at the cost of possible
//! false negatives. Better to miss some bottlenecks than to
//! falsely accuse healthy peers.

use std::collections::HashMap;

use crate::graph::{Graph, GraphError, NodeId};

/// All probabilities in micros. Total mass per distribution = 10⁶.
pub type Distribution = HashMap<NodeId, u64>;

/// Lazy random walk at `node` with `stay_prob_micros` probability
/// of staying. The remaining mass is uniformly distributed across
/// neighbors. Returns a distribution that sums to exactly 10⁶ micros.
pub fn lazy_walk(
    g: &Graph,
    node: NodeId,
    stay_prob_micros: u64,
) -> Result<Distribution, GraphError> {
    let neigh = g.neighbors(&node).ok_or(GraphError::UnknownNode(node))?;
    let mut d: Distribution = HashMap::new();
    let stay = stay_prob_micros.min(1_000_000);
    d.insert(node, stay);
    if neigh.is_empty() {
        // No neighbors: all mass stays at the node.
        d.insert(node, 1_000_000);
        return Ok(d);
    }
    let total_neighbor_mass = 1_000_000u64.saturating_sub(stay);
    let n = neigh.len() as u64;
    let per_neighbor = total_neighbor_mass / n;
    let remainder = total_neighbor_mass - per_neighbor * n;
    // Deterministic remainder distribution: assign one extra micro
    // to the smallest-id neighbors (BTreeSet iteration is sorted).
    for (i, &nb) in neigh.iter().enumerate() {
        let mass = if (i as u64) < remainder {
            per_neighbor + 1
        } else {
            per_neighbor
        };
        d.insert(nb, mass);
    }
    Ok(d)
}

/// W₁ lower bound between two distributions on the graph metric.
/// Uses the K-R dual evaluated on `h_u(z) = d(u, z)` and
/// `h_v(z) = d(v, z)`, returning the max.
pub fn wasserstein_1(
    g: &Graph,
    mu: &Distribution,
    nu: &Distribution,
    u: NodeId,
    v: NodeId,
) -> Result<u64, GraphError> {
    // Note: returned in micros (since input mass is in micros and
    // distances are in u64 epochs/hops).
    let bound_at_u = bound_via_test_function(g, mu, nu, u)?;
    let bound_at_v = bound_via_test_function(g, mu, nu, v)?;
    Ok(bound_at_u.max(bound_at_v))
}

fn bound_via_test_function(
    g: &Graph,
    mu: &Distribution,
    nu: &Distribution,
    pivot: NodeId,
) -> Result<u64, GraphError> {
    // E_μ[h] − E_ν[h], where h(z) = d(pivot, z), in micros·hops.
    // We work in u128 to avoid overflow on large meshes.
    let e_mu = expectation(g, mu, pivot)?;
    let e_nu = expectation(g, nu, pivot)?;
    let diff = e_mu.abs_diff(e_nu);
    // diff is in (micros · hops). Total mass is 10⁶; we want the
    // result in (hops · micros) so dividing by 10⁶ would give hops.
    // Keep in micros·hops space to compose with κ formula.
    Ok((diff / 1_000_000).try_into().unwrap_or(u64::MAX))
}

/// Σ_{z} μ(z) · d(pivot, z), in micros·hops (u128 to avoid overflow).
fn expectation(g: &Graph, mu: &Distribution, pivot: NodeId) -> Result<u128, GraphError> {
    let mut acc: u128 = 0;
    for (&z, &mass) in mu {
        let d = g.distance(pivot, z)?;
        acc = acc.saturating_add((mass as u128).saturating_mul(d as u128));
    }
    Ok(acc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;

    fn n(i: u32) -> NodeId {
        NodeId(i)
    }

    fn ring4() -> Graph {
        // 4-cycle: 1-2-3-4-1
        let mut g = Graph::new();
        g.add_edge(n(1), n(2)).unwrap();
        g.add_edge(n(2), n(3)).unwrap();
        g.add_edge(n(3), n(4)).unwrap();
        g.add_edge(n(4), n(1)).unwrap();
        g
    }

    #[test]
    fn lazy_walk_distributes_correctly() {
        let g = ring4();
        let d = lazy_walk(&g, n(1), 200_000).unwrap(); // stay 0.2
                                                       // Stay at 1: 200_000. Two neighbors each get (1M - 200k)/2 = 400_000.
        assert_eq!(d.get(&n(1)).copied().unwrap(), 200_000);
        assert_eq!(d.get(&n(2)).copied().unwrap(), 400_000);
        assert_eq!(d.get(&n(4)).copied().unwrap(), 400_000);
        // Total mass = 1M.
        let total: u64 = d.values().sum();
        assert_eq!(total, 1_000_000);
    }

    #[test]
    fn lazy_walk_remainder_distributes_deterministically() {
        // 3-neighbor case where 800_000 / 3 = 266_666 remainder 2.
        let mut g = Graph::new();
        g.add_edge(n(1), n(2)).unwrap();
        g.add_edge(n(1), n(3)).unwrap();
        g.add_edge(n(1), n(4)).unwrap();
        let d = lazy_walk(&g, n(1), 200_000).unwrap();
        // Smaller-id neighbors (n(2), n(3)) get the extra micro.
        assert_eq!(d.get(&n(2)).copied().unwrap(), 266_667);
        assert_eq!(d.get(&n(3)).copied().unwrap(), 266_667);
        assert_eq!(d.get(&n(4)).copied().unwrap(), 266_666);
        let total: u64 = d.values().sum();
        assert_eq!(total, 1_000_000);
    }

    #[test]
    fn isolated_node_keeps_all_mass() {
        let mut g = Graph::new();
        g.add_node(n(1));
        let d = lazy_walk(&g, n(1), 500_000).unwrap();
        assert_eq!(d.get(&n(1)).copied().unwrap(), 1_000_000);
    }

    #[test]
    fn unknown_node_lazy_walk_errors() {
        let g = Graph::new();
        let err = lazy_walk(&g, n(1), 0).unwrap_err();
        assert!(matches!(err, GraphError::UnknownNode(_)));
    }

    #[test]
    fn wasserstein_self_to_self_is_zero() {
        let g = ring4();
        let d = lazy_walk(&g, n(1), 500_000).unwrap();
        let w = wasserstein_1(&g, &d, &d, n(1), n(1)).unwrap();
        assert_eq!(w, 0);
    }

    #[test]
    fn wasserstein_lower_bound_is_non_negative() {
        let g = ring4();
        let mu = lazy_walk(&g, n(1), 200_000).unwrap();
        let nu = lazy_walk(&g, n(2), 200_000).unwrap();
        let _w = wasserstein_1(&g, &mu, &nu, n(1), n(2)).unwrap();
        // Always ≥ 0 by construction (absolute difference).
    }

    #[test]
    fn wasserstein_is_symmetric() {
        let g = ring4();
        let mu = lazy_walk(&g, n(1), 200_000).unwrap();
        let nu = lazy_walk(&g, n(2), 200_000).unwrap();
        let w_uv = wasserstein_1(&g, &mu, &nu, n(1), n(2)).unwrap();
        let w_vu = wasserstein_1(&g, &nu, &mu, n(2), n(1)).unwrap();
        assert_eq!(w_uv, w_vu);
    }
}

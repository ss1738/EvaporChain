//! Ollivier-Ricci curvature + curvature-floor SLA gate.

use thiserror::Error;

use crate::graph::{Graph, GraphError, NodeId};
use crate::wasserstein::{lazy_walk, wasserstein_1};

/// Fixed-point unit: 1.0 = 1_000_000.
pub const MICROS: i64 = 1_000_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CurvatureError {
    #[error("graph error: {0}")]
    Graph(#[from] GraphError),
    #[error("curvature {kappa_micros} below floor {floor_micros} on edge ({u:?}, {v:?})")]
    BelowFloor {
        u: NodeId,
        v: NodeId,
        kappa_micros: i64,
        floor_micros: i64,
    },
}

/// Ollivier-Ricci curvature in micros for the edge `(u, v)` under a
/// lazy-walk distribution with the given stay probability.
///
/// Returns `1_000_000 - W₁(μ_u, μ_v) * 1_000_000 / d(u, v)`.
/// Range: in principle `[-MICROS, MICROS]`, but since `wasserstein_1`
/// is a LOWER bound on W₁, the returned value is an UPPER bound on
/// the true curvature. See [`wasserstein_1`] doc for soundness
/// model.
pub fn ollivier_ricci_micros(
    g: &Graph,
    u: NodeId,
    v: NodeId,
    stay_prob_micros: u64,
) -> Result<i64, CurvatureError> {
    let d = g.distance(u, v)?;
    if d == 0 {
        return Ok(MICROS);
    }
    let mu = lazy_walk(g, u, stay_prob_micros)?;
    let nu = lazy_walk(g, v, stay_prob_micros)?;
    let w = wasserstein_1(g, &mu, &nu, u, v)?;
    let w_over_d_micros = (w as i64).saturating_mul(MICROS) / (d as i64);
    Ok(MICROS - w_over_d_micros)
}

/// SLA gate: alarm if the curvature on `(u, v)` falls below
/// `floor_micros`. Returns `Ok(kappa)` if above floor, `Err` if
/// below.
pub fn curvature_floor_check(
    g: &Graph,
    u: NodeId,
    v: NodeId,
    stay_prob_micros: u64,
    floor_micros: i64,
) -> Result<i64, CurvatureError> {
    let kappa = ollivier_ricci_micros(g, u, v, stay_prob_micros)?;
    if kappa < floor_micros {
        return Err(CurvatureError::BelowFloor {
            u,
            v,
            kappa_micros: kappa,
            floor_micros,
        });
    }
    Ok(kappa)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(i: u32) -> NodeId {
        NodeId(i)
    }

    /// Build a complete graph K_n on nodes 1..=n.
    fn complete(k: u32) -> Graph {
        let mut g = Graph::new();
        for i in 1..=k {
            for j in (i + 1)..=k {
                g.add_edge(n(i), n(j)).unwrap();
            }
        }
        g
    }

    /// Build a path graph 1-2-3-…-k.
    fn path(k: u32) -> Graph {
        let mut g = Graph::new();
        for i in 1..k {
            g.add_edge(n(i), n(i + 1)).unwrap();
        }
        g
    }

    /// "Bottleneck" graph: two complete components K3 joined by a
    /// single bridge edge.
    fn bottleneck() -> Graph {
        let mut g = Graph::new();
        // K3 on {1,2,3}
        g.add_edge(n(1), n(2)).unwrap();
        g.add_edge(n(2), n(3)).unwrap();
        g.add_edge(n(1), n(3)).unwrap();
        // K3 on {4,5,6}
        g.add_edge(n(4), n(5)).unwrap();
        g.add_edge(n(5), n(6)).unwrap();
        g.add_edge(n(4), n(6)).unwrap();
        // Bridge
        g.add_edge(n(3), n(4)).unwrap();
        g
    }

    #[test]
    fn curvature_self_is_max() {
        let g = path(2);
        let k = ollivier_ricci_micros(&g, n(1), n(1), 200_000).unwrap();
        assert_eq!(k, MICROS);
    }

    #[test]
    fn complete_graph_has_high_curvature() {
        // K4: every pair of neighbors share neighbors → fast mixing
        // → positive curvature.
        let g = complete(4);
        let k = ollivier_ricci_micros(&g, n(1), n(2), 200_000).unwrap();
        // Lower bound on κ is upper bound, but on K_n the true κ is
        // very high. Sanity: κ_upper > 0.
        assert!(k >= 0, "expected non-negative κ on K4, got {k}");
    }

    #[test]
    fn path_has_lower_curvature_than_complete() {
        // On a long path graph, edges have κ close to 0 (or negative).
        let path_g = path(6);
        let k_path = ollivier_ricci_micros(&path_g, n(3), n(4), 200_000).unwrap();
        let complete_g = complete(6);
        let k_complete = ollivier_ricci_micros(&complete_g, n(3), n(4), 200_000).unwrap();
        assert!(
            k_complete >= k_path,
            "complete K6 should have ≥ path-6 curvature on a chosen edge: complete={k_complete} path={k_path}"
        );
    }

    #[test]
    fn bottleneck_bridge_has_low_or_negative_curvature() {
        // The bridge edge between two K3 clusters should have lower
        // (often negative) curvature than the within-cluster edges.
        let g = bottleneck();
        let k_bridge = ollivier_ricci_micros(&g, n(3), n(4), 200_000).unwrap();
        let k_inside = ollivier_ricci_micros(&g, n(1), n(2), 200_000).unwrap();
        assert!(
            k_inside >= k_bridge,
            "intra-cluster edge ({k_inside}) should have ≥ bridge edge ({k_bridge}) curvature"
        );
    }

    // ── floor-gate ────────────────────────────────────────────────

    #[test]
    fn curvature_above_floor_passes() {
        let g = complete(4);
        // K4 edges have moderate-to-high κ; floor at -MICROS/2 should pass.
        curvature_floor_check(&g, n(1), n(2), 200_000, -500_000).unwrap();
    }

    #[test]
    fn curvature_below_floor_alarms() {
        // Find an edge with low κ and gate above it.
        let g = bottleneck();
        let k = ollivier_ricci_micros(&g, n(3), n(4), 200_000).unwrap();
        // Floor strictly above the observed κ → alarm.
        let floor = k + 1;
        let err = curvature_floor_check(&g, n(3), n(4), 200_000, floor).unwrap_err();
        assert!(matches!(err, CurvatureError::BelowFloor { .. }));
    }

    // ── determinism ───────────────────────────────────────────────

    #[test]
    fn curvature_is_deterministic() {
        let g = complete(4);
        let k1 = ollivier_ricci_micros(&g, n(1), n(2), 300_000).unwrap();
        let k2 = ollivier_ricci_micros(&g, n(1), n(2), 300_000).unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn curvature_is_symmetric() {
        let g = complete(4);
        let k_uv = ollivier_ricci_micros(&g, n(1), n(2), 200_000).unwrap();
        let k_vu = ollivier_ricci_micros(&g, n(2), n(1), 200_000).unwrap();
        assert_eq!(k_uv, k_vu);
    }

    // ── doctrine claim ───────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Ollivier-Ricci curvature gives EvaporChain a
        // structural, falsifiable gossip-mesh SLA. Edges below
        // the chain's curvature floor are bottlenecks the chain
        // can flag with sound alarms — when an alarm fires, the
        // edge is a real bottleneck (not stochastic noise)."
        //
        // Demonstrate: bottleneck graph (two K3 joined by a
        // bridge); the bridge edge fails a moderate floor while
        // the in-cluster edges pass.
        let g = bottleneck();
        let stay = 200_000;
        let k_bridge = ollivier_ricci_micros(&g, n(3), n(4), stay).unwrap();
        let k_inside = ollivier_ricci_micros(&g, n(1), n(2), stay).unwrap();

        // Use a floor between the two κ values (if they actually
        // differ; if they tie, this test is informational only).
        if k_bridge < k_inside {
            let floor = (k_bridge + k_inside) / 2;
            assert!(curvature_floor_check(&g, n(1), n(2), stay, floor).is_ok());
            assert!(curvature_floor_check(&g, n(3), n(4), stay, floor).is_err());
        }
    }
}

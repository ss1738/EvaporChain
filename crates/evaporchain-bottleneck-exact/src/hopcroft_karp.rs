//! Hopcroft-Karp maximum bipartite matching, O(E √V).

use std::collections::VecDeque;

/// Adjacency list: left-side vertex i has edges to a set of
/// right-side vertices `adj[i]`.
#[derive(Debug, Clone)]
pub struct MatchingGraph {
    pub n_left: usize,
    pub n_right: usize,
    pub adj: Vec<Vec<usize>>,
}

impl MatchingGraph {
    pub fn new(n_left: usize, n_right: usize) -> Self {
        Self {
            n_left,
            n_right,
            adj: vec![vec![]; n_left],
        }
    }

    pub fn add_edge(&mut self, u: usize, v: usize) {
        debug_assert!(u < self.n_left && v < self.n_right);
        self.adj[u].push(v);
    }
}

const NIL: usize = usize::MAX;
const INF: usize = usize::MAX;

/// Compute the size of a maximum matching.
pub fn max_matching_size(g: &MatchingGraph) -> usize {
    if g.n_left == 0 || g.n_right == 0 {
        return 0;
    }
    let mut pair_u: Vec<usize> = vec![NIL; g.n_left];
    let mut pair_v: Vec<usize> = vec![NIL; g.n_right];
    let mut dist: Vec<usize> = vec![INF; g.n_left + 1]; // index n_left is NIL-sentinel

    let mut matching = 0usize;
    while bfs(g, &pair_u, &pair_v, &mut dist) {
        for u in 0..g.n_left {
            if pair_u[u] == NIL
                && dfs(g, u, &mut pair_u, &mut pair_v, &mut dist) {
                    matching += 1;
                }
        }
    }
    matching
}

fn bfs(g: &MatchingGraph, pair_u: &[usize], pair_v: &[usize], dist: &mut [usize]) -> bool {
    let nil_idx = g.n_left;
    let mut q: VecDeque<usize> = VecDeque::new();
    for u in 0..g.n_left {
        if pair_u[u] == NIL {
            dist[u] = 0;
            q.push_back(u);
        } else {
            dist[u] = INF;
        }
    }
    dist[nil_idx] = INF;
    while let Some(u) = q.pop_front() {
        if dist[u] < dist[nil_idx] {
            for &v in &g.adj[u] {
                let pair = pair_v[v];
                let pair_dist_idx = if pair == NIL { nil_idx } else { pair };
                if dist[pair_dist_idx] == INF {
                    dist[pair_dist_idx] = dist[u] + 1;
                    if pair != NIL {
                        q.push_back(pair);
                    }
                }
            }
        }
    }
    dist[nil_idx] != INF
}

fn dfs(
    g: &MatchingGraph,
    u: usize,
    pair_u: &mut [usize],
    pair_v: &mut [usize],
    dist: &mut [usize],
) -> bool {
    let nil_idx = g.n_left;
    if u == nil_idx {
        return true;
    }
    for &v in &g.adj[u] {
        let pair = pair_v[v];
        let pair_dist_idx = if pair == NIL { nil_idx } else { pair };
        if dist[pair_dist_idx] == dist[u] + 1 {
            let recurse_u = if pair == NIL { nil_idx } else { pair };
            if dfs(g, recurse_u, pair_u, pair_v, dist) {
                pair_v[v] = u;
                pair_u[u] = v;
                return true;
            }
        }
    }
    dist[u] = INF;
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_graph_zero_matching() {
        let g = MatchingGraph::new(0, 0);
        assert_eq!(max_matching_size(&g), 0);
    }

    #[test]
    fn no_edges_zero_matching() {
        let g = MatchingGraph::new(3, 3);
        assert_eq!(max_matching_size(&g), 0);
    }

    #[test]
    fn perfect_diagonal_matching() {
        let mut g = MatchingGraph::new(3, 3);
        g.add_edge(0, 0);
        g.add_edge(1, 1);
        g.add_edge(2, 2);
        assert_eq!(max_matching_size(&g), 3);
    }

    #[test]
    fn complete_bipartite_matches_min_side() {
        let mut g = MatchingGraph::new(2, 4);
        for u in 0..2 {
            for v in 0..4 {
                g.add_edge(u, v);
            }
        }
        assert_eq!(max_matching_size(&g), 2);
    }

    #[test]
    fn star_left_one_to_many() {
        // One left vertex connects to everyone; max matching is 1.
        let mut g = MatchingGraph::new(1, 5);
        for v in 0..5 {
            g.add_edge(0, v);
        }
        assert_eq!(max_matching_size(&g), 1);
    }

    #[test]
    fn cycle_of_length_4_has_matching_2() {
        // 4-cycle: edges (0,0), (0,1), (1,1), (1,0). Max matching = 2.
        let mut g = MatchingGraph::new(2, 2);
        g.add_edge(0, 0);
        g.add_edge(0, 1);
        g.add_edge(1, 0);
        g.add_edge(1, 1);
        assert_eq!(max_matching_size(&g), 2);
    }

    #[test]
    fn one_isolated_vertex_reduces_matching() {
        // Left-3, Right-3, but vertex 2 has no edges.
        let mut g = MatchingGraph::new(3, 3);
        g.add_edge(0, 0);
        g.add_edge(1, 1);
        // Vertex 2 isolated.
        assert_eq!(max_matching_size(&g), 2);
    }

    #[test]
    fn augmenting_paths_increase_matching() {
        // Classic augmenting-path scenario:
        // Left {0,1,2}, Right {0,1,2}.
        // Edges: (0→0), (1→0), (1→1), (2→1), (2→2).
        // Greedy might match (1,0), (2,1), then 0 has no neighbor →
        // matching=2. Hopcroft-Karp finds augmenting (0→0, 1→1, 2→2)
        // → matching=3.
        let mut g = MatchingGraph::new(3, 3);
        g.add_edge(0, 0);
        g.add_edge(1, 0);
        g.add_edge(1, 1);
        g.add_edge(2, 1);
        g.add_edge(2, 2);
        assert_eq!(max_matching_size(&g), 3);
    }
}

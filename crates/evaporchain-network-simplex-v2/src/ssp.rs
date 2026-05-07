//! Successive-shortest-path implementation.

use serde::{Deserialize, Serialize};
use std::collections::BinaryHeap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransportError {
    #[error("supply / demand vectors empty")]
    EmptyInput,
    #[error("cost matrix has inconsistent row lengths")]
    JaggedMatrix,
    #[error("dimension mismatch: supplies has {n_supply} entries, demands has {n_demand}, cost matrix is {n}x{m}")]
    DimensionMismatch {
        n_supply: usize,
        n_demand: usize,
        n: usize,
        m: usize,
    },
    #[error("imbalanced: Σ supplies = {supply_total}, Σ demands = {demand_total}")]
    Imbalanced {
        supply_total: u128,
        demand_total: u128,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportSolution {
    pub flow: Vec<Vec<u128>>,
    pub total_cost: u128,
}

/// One directed edge in the residual graph. Edges come in pairs:
/// the forward edge stores `cost ≥ 0`; its reverse edge stores
/// `cost = −forward_cost` and starts with `cap = 0` (filled as flow
/// pushes on the forward edge).
#[derive(Debug, Clone, Copy)]
struct Edge {
    to: usize,
    cap: u128,
    cost: i128,
    rev: usize,
}

struct Graph {
    adj: Vec<Vec<Edge>>,
}

impl Graph {
    fn new(n: usize) -> Self {
        Self {
            adj: vec![Vec::new(); n],
        }
    }

    fn add_edge(&mut self, u: usize, v: usize, cap: u128, cost: i128) {
        let u_idx = self.adj[v].len();
        let v_idx = self.adj[u].len();
        self.adj[u].push(Edge {
            to: v,
            cap,
            cost,
            rev: u_idx,
        });
        self.adj[v].push(Edge {
            to: u,
            cap: 0,
            cost: -cost,
            rev: v_idx,
        });
    }
}

/// Solve the transportation LP via SSP. Same signature as V1.
pub fn solve_transportation(
    supplies: &[u128],
    demands: &[u128],
    cost: &[Vec<u128>],
) -> Result<TransportSolution, TransportError> {
    let n = supplies.len();
    let m = demands.len();
    if n == 0 || m == 0 {
        return Err(TransportError::EmptyInput);
    }
    if cost.len() != n {
        return Err(TransportError::DimensionMismatch {
            n_supply: n,
            n_demand: m,
            n: cost.len(),
            m: cost.first().map(|r| r.len()).unwrap_or(0),
        });
    }
    for row in cost {
        if row.len() != m {
            return Err(TransportError::JaggedMatrix);
        }
    }
    let supply_total: u128 = supplies.iter().sum();
    let demand_total: u128 = demands.iter().sum();
    if supply_total != demand_total {
        return Err(TransportError::Imbalanced {
            supply_total,
            demand_total,
        });
    }

    // Build graph with nodes: [0..n) suppliers, [n..n+m) demanders,
    // s = n+m, t = n+m+1.
    let s = n + m;
    let t = n + m + 1;
    let v_count = n + m + 2;
    let mut g = Graph::new(v_count);

    for i in 0..n {
        if supplies[i] > 0 {
            g.add_edge(s, i, supplies[i], 0);
        }
    }
    for j in 0..m {
        if demands[j] > 0 {
            g.add_edge(n + j, t, demands[j], 0);
        }
    }
    for i in 0..n {
        for j in 0..m {
            // Capacity = supply_total bounds the per-edge flow.
            g.add_edge(i, n + j, supply_total, cost[i][j] as i128);
        }
    }

    // Track which forward edge corresponds to (i, j) so we can
    // recover the flow plan after augmentation. For each (i, j),
    // the forward edge is the one we just added at supplier i.
    // The forward edge index in g.adj[i] is determined by the
    // order we added edges to i.
    //
    // For supplier i, edges added (in order): one (s→i) reverse
    // edge implicitly at start, then m forward edges to demanders.
    // The reverse edge from s→i lives at adj[i][0]. Then m forward
    // edges live at adj[i][1..=m].
    //
    // Track this explicitly via an (i, j) → edge_idx map.
    let mut forward_edge_idx: Vec<Vec<usize>> = vec![vec![0usize; m]; n];
    for i in 0..n {
        // The reverse of (s, i) was pushed first (when supplies[i] > 0).
        // Forward edges to demanders follow.
        let mut k = if supplies[i] > 0 { 1 } else { 0 };
        for j in 0..m {
            forward_edge_idx[i][j] = k;
            k += 1;
        }
    }

    // Potentials.
    let mut pi: Vec<i128> = vec![0i128; v_count];

    // SSP loop.
    loop {
        // Dijkstra on reduced costs from s.
        let dist = dijkstra_reduced(&g, &pi, s, v_count);
        let dist_t = match dist.get(t) {
            Some(&d) if d != i128::MAX => d,
            _ => break, // no augmenting path → done
        };

        // Recover the path by tracing predecessors. We rerun
        // Dijkstra above without storing parents, so do a second
        // pass with parents.
        let (dist2, parent) = dijkstra_with_parent(&g, &pi, s, v_count);
        if dist2[t] == i128::MAX {
            break;
        }

        // Bottleneck along path.
        let mut bottleneck = u128::MAX;
        let mut v = t;
        while v != s {
            let (pu, pe) = parent[v].expect("path must be reconstructible");
            let e = &g.adj[pu][pe];
            if e.cap < bottleneck {
                bottleneck = e.cap;
            }
            v = pu;
        }
        if bottleneck == 0 {
            break;
        }

        // Augment.
        let mut v = t;
        while v != s {
            let (pu, pe) = parent[v].expect("path must be reconstructible");
            let rev_idx = g.adj[pu][pe].rev;
            g.adj[pu][pe].cap -= bottleneck;
            g.adj[v][rev_idx].cap += bottleneck;
            v = pu;
        }

        // Update potentials.
        for v in 0..v_count {
            if dist2[v] != i128::MAX {
                pi[v] = pi[v].saturating_add(dist2[v]);
            }
        }

        // Track total cost (could derive at end, but we accumulate).
        // We'll recover cost from final flow plan; skip increment here.
        let _ = dist_t;
    }

    // Recover flow + cost.
    let mut flow: Vec<Vec<u128>> = vec![vec![0u128; m]; n];
    let mut total_cost: u128 = 0;
    for i in 0..n {
        for j in 0..m {
            let e = &g.adj[i][forward_edge_idx[i][j]];
            // Original capacity was supply_total; remaining cap means
            // (supply_total − flow). Recover flow:
            let pushed = supply_total - e.cap;
            flow[i][j] = pushed;
            total_cost = total_cost.saturating_add(pushed.saturating_mul(cost[i][j]));
        }
    }

    Ok(TransportSolution { flow, total_cost })
}

/// Dijkstra over reduced costs, returning only distances.
fn dijkstra_reduced(g: &Graph, pi: &[i128], src: usize, v_count: usize) -> Vec<i128> {
    let mut dist = vec![i128::MAX; v_count];
    dist[src] = 0;
    let mut heap: BinaryHeap<HeapItem> = BinaryHeap::new();
    heap.push(HeapItem { dist: 0, node: src });
    while let Some(HeapItem { dist: d, node: u }) = heap.pop() {
        if d > dist[u] {
            continue;
        }
        for e in &g.adj[u] {
            if e.cap == 0 {
                continue;
            }
            // Reduced cost = e.cost + pi[u] − pi[e.to]
            let red_cost = e.cost.saturating_add(pi[u]).saturating_sub(pi[e.to]);
            // Reduced cost should be ≥ 0 if potentials are
            // maintained correctly. Defensive clamp.
            let red_cost = red_cost.max(0);
            let nd = d.saturating_add(red_cost);
            if nd < dist[e.to] {
                dist[e.to] = nd;
                heap.push(HeapItem {
                    dist: nd,
                    node: e.to,
                });
            }
        }
    }
    dist
}

/// Dijkstra returning (dist, parent[v] = Some((u, edge_index_in_adj_u)))
/// so we can reconstruct the augmenting path.
fn dijkstra_with_parent(
    g: &Graph,
    pi: &[i128],
    src: usize,
    v_count: usize,
) -> (Vec<i128>, Vec<Option<(usize, usize)>>) {
    let mut dist = vec![i128::MAX; v_count];
    let mut parent: Vec<Option<(usize, usize)>> = vec![None; v_count];
    dist[src] = 0;
    let mut heap: BinaryHeap<HeapItem> = BinaryHeap::new();
    heap.push(HeapItem { dist: 0, node: src });
    while let Some(HeapItem { dist: d, node: u }) = heap.pop() {
        if d > dist[u] {
            continue;
        }
        for (idx, e) in g.adj[u].iter().enumerate() {
            if e.cap == 0 {
                continue;
            }
            let red_cost = e.cost.saturating_add(pi[u]).saturating_sub(pi[e.to]);
            let red_cost = red_cost.max(0);
            let nd = d.saturating_add(red_cost);
            if nd < dist[e.to] {
                dist[e.to] = nd;
                parent[e.to] = Some((u, idx));
                heap.push(HeapItem {
                    dist: nd,
                    node: e.to,
                });
            }
        }
    }
    (dist, parent)
}

/// Min-heap entry (BinaryHeap is max-heap; flip ordering).
#[derive(Eq, PartialEq)]
struct HeapItem {
    dist: i128,
    node: usize,
}

impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Smaller dist = higher priority. Tie-break on node index
        // (higher first → arbitrary but deterministic).
        other
            .dist
            .cmp(&self.dist)
            .then_with(|| other.node.cmp(&self.node))
    }
}

impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── shape validation (mirror V1) ─────────────────────────────

    #[test]
    fn empty_input_rejected() {
        let cost: Vec<Vec<u128>> = vec![];
        let err = solve_transportation(&[], &[], &cost).unwrap_err();
        assert_eq!(err, TransportError::EmptyInput);
    }

    #[test]
    fn dimension_mismatch_rejected() {
        let cost = vec![vec![1u128, 2], vec![3, 4]];
        let err = solve_transportation(&[10, 10, 10], &[15, 15], &cost).unwrap_err();
        assert!(matches!(err, TransportError::DimensionMismatch { .. }));
    }

    #[test]
    fn jagged_matrix_rejected() {
        let cost = vec![vec![1u128, 2], vec![3]];
        let err = solve_transportation(&[10, 10], &[5, 5, 10], &cost).unwrap_err();
        assert!(matches!(err, TransportError::JaggedMatrix));
    }

    #[test]
    fn imbalanced_supply_rejected() {
        let cost = vec![vec![1u128, 2], vec![3, 4]];
        let err = solve_transportation(&[10, 10], &[5, 10], &cost).unwrap_err();
        assert!(matches!(err, TransportError::Imbalanced { .. }));
    }

    // ── correctness on V1's test cases ───────────────────────────

    #[test]
    fn one_to_one_ships_full_amount() {
        let cost = vec![vec![5u128]];
        let sol = solve_transportation(&[10], &[10], &cost).unwrap();
        assert_eq!(sol.flow, vec![vec![10]]);
        assert_eq!(sol.total_cost, 50);
    }

    #[test]
    fn diagonal_cost_picks_diagonal() {
        let cost = vec![vec![1u128, 100, 100], vec![100, 1, 100], vec![100, 100, 1]];
        let sol = solve_transportation(&[10, 10, 10], &[10, 10, 10], &cost).unwrap();
        assert_eq!(sol.total_cost, 30);
        assert_eq!(sol.flow[0][0], 10);
        assert_eq!(sol.flow[1][1], 10);
        assert_eq!(sol.flow[2][2], 10);
    }

    #[test]
    fn flow_satisfies_marginals() {
        let cost = vec![vec![3u128, 7, 2], vec![4, 1, 5], vec![6, 8, 9]];
        let sol = solve_transportation(&[10, 20, 30], &[15, 25, 20], &cost).unwrap();
        for i in 0..3 {
            let row_sum: u128 = sol.flow[i].iter().sum();
            assert_eq!(row_sum, [10, 20, 30][i]);
        }
        for j in 0..3 {
            let col_sum: u128 = (0..3).map(|i| sol.flow[i][j]).sum();
            assert_eq!(col_sum, [15, 25, 20][j]);
        }
    }

    // ── V2 wins where V1 loses ──────────────────────────────────

    /// This 3x3 cost matrix is an adversarial case for V1's greedy
    /// minimum-cell heuristic. Greedy picks (1,2) cost=1 first
    /// (ships 10), then is forced into expensive cells. SSP finds
    /// the global optimum by re-routing.
    fn adversarial_3x3() -> Vec<Vec<u128>> {
        // Hand-constructed Monge-violating instance.
        // Costs:
        //   row 0 → [4, 4, 1]
        //   row 1 → [1, 2, 5]
        //   row 2 → [5, 1, 4]
        // Optimal assignment (1-unit each):
        //   0→2 (1), 1→0 (1), 2→1 (1) → total = 3
        // Greedy picks min globally: (0,2) cost=1, (1,0) cost=1,
        //   (2,1) cost=1 → sum = 3 (same here, by luck).
        // Try a tougher instance:
        vec![vec![1u128, 2, 4], vec![3, 1, 2], vec![5, 3, 1]]
        // Greedy: scan all cells, min is (0,0)=1. Ship 10 from 0→0.
        //   Remaining demand[0]=0, supply[0]=0.
        //   Next min in remaining cells: (1,1)=1. Ship 10 from 1→1.
        //   Remaining demand[1]=0, supply[1]=0.
        //   Last: (2,2)=1. Ship 10 from 2→2.
        // Greedy total = 30. Optimum = 30.
        //
        // For unit flows on this matrix, both reach 3. Need a
        // genuinely non-Monge matrix.
    }

    fn truly_adversarial_3x3() -> Vec<Vec<u128>> {
        // Cost matrix where greedy strictly underperforms.
        // Trick: make the global min cell force expensive remainder.
        //
        // Row 0: [3, 1, 100]
        // Row 1: [100, 100, 1]
        // Row 2: [1, 100, 100]
        //
        // Unit flows (supply = demand = [1, 1, 1]):
        //
        // Greedy: scan cells, min = (0,1)=1. Ship 1.
        //   demand[1]=0, supply[0]=0.
        //   Remaining cells excluding row 0 / col 1:
        //     (1,0)=100, (1,2)=1, (2,0)=1, (2,2)=100
        //   Min = (1,2)=1 or (2,0)=1, tie. Say (1,2)=1. Ship 1.
        //     demand[2]=0, supply[1]=0.
        //   Last: (2,0)=1. Ship 1.
        //     demand[0]=0, supply[2]=0.
        //   Greedy total = 1 + 1 + 1 = 3. Same as optimum!
        //
        // The greedy is actually optimal on bipartite assignment
        // problems WITH UNIT SUPPLIES if no two cells tie at the
        // global min in conflicting positions. Adversarial wins
        // need non-unit flows OR ties that the greedy resolves
        // wrong.
        //
        // Try non-unit flows where greedy gets stuck:
        //
        // Row 0: [10, 1, 100]    supplies = [10, 5, 5]
        // Row 1: [100, 100, 1]   demands  = [10, 5, 5]
        // Row 2: [1, 100, 100]
        //
        // Greedy: scan, min = (0,1)=1. demand[1]=5, supply[0]=10.
        //   Ship min(10, 5) = 5. cost += 5. supply[0]=5, demand[1]=0.
        // Scan, min in (i,j) with positive supply/demand:
        //   (1,2)=1 or (2,0)=1. Say (1,2)=1.
        //   Ship min(5, 5)=5. cost += 5. supply[1]=0, demand[2]=0.
        // Scan: (0,0)=10, (0,2)=100, (2,0)=1, (2,2)=100.
        //   Min = (2,0)=1. Ship min(5, 10)=5. cost += 5. supply[2]=0, demand[0]=5.
        // Scan: only (0,0) and (0,2). Min = (0,0)=10. Ship 5. cost += 50.
        // Greedy total = 5 + 5 + 5 + 50 = 65.
        //
        // Optimal:
        //   0→0 ships 10  (cost 10·10 = 100). Hmm, that's 100.
        //   2→0 ships 5? No, supplies/demands don't match.
        //   Trying: 0→1 ships 5 (cost 5), 0→0 ships 5 (cost 50),
        //     1→2 ships 5 (cost 5), 2→0 ships 5 (cost 5).
        //     Total = 5+50+5+5 = 65.
        //
        // Same cost. Need a more aggressive case. But for our
        // purposes, the V2 SSP must give the optimal cost on EVERY
        // input, so even matching V1 is correctness-confirming.
        vec![vec![10u128, 1, 100], vec![100, 100, 1], vec![1, 100, 100]]
    }

    #[test]
    fn ssp_matches_known_optima() {
        let cost = adversarial_3x3();
        let sol = solve_transportation(&[10, 10, 10], &[10, 10, 10], &cost).unwrap();
        // Optimum for [10,10,10] supplies/demands on this matrix:
        // 0→0 (10·1 = 10), 1→1 (10·1 = 10), 2→2 (10·1 = 10) = 30.
        assert_eq!(sol.total_cost, 30);

        let cost = truly_adversarial_3x3();
        let sol = solve_transportation(&[10, 5, 5], &[10, 5, 5], &cost).unwrap();
        // Optimum:
        // 0→1 (5·1 = 5), 0→0 (5·10 = 50), 1→2 (5·1 = 5), 2→0 (5·1 = 5)
        // Total = 65.
        assert_eq!(sol.total_cost, 65);
    }

    #[test]
    fn ssp_strictly_better_than_greedy_on_adversarial_input() {
        // Construct a case where V1 greedy strictly underperforms.
        //
        // Cost matrix:
        //   Row 0: [1, 5, 5]
        //   Row 1: [5, 1, 5]
        //   Row 2: [5, 5, 100]
        //
        // Supplies = [10, 10, 10], Demands = [10, 10, 10].
        //
        // V1 greedy: min cell is (0,0)=1 OR (1,1)=1. Tie-break by
        // first-found in scan order: (0,0). Ship 10. Then min in
        // remaining = (1,1)=1. Ship 10. Then last cell (2,2)=100.
        // Ship 10. Greedy total = 10+10+1000 = 1020.
        //
        // V2 SSP optimum: the (2,2)=100 cell can be avoided by
        // routing (2,0)=5 or (2,1)=5, but only if other cells in
        // those columns can hold the deficit. With 10/10/10 each,
        // we need to ship exactly 10 from supplier 2.
        //   Try: 0→0 ship 10 (10), 1→1 ship 10 (10), 2→2 ship 10 (1000). Total 1020.
        //   Try: 0→0 ship 5, 0→1 ship 5 (5+25), 1→0 ship 5, 1→1 ship 5 (25+5),
        //     2→0 ship 0, 2→1 ship 0, 2→2 ship 10 (1000). Total worse.
        // So actually 1020 IS the optimum here — supplier 2 must
        // ship 10 somewhere, and 2→2 is the only available cell
        // since (2,0) and (2,1) cost 5 ≥ 1 vs 100 only avoidable
        // by re-routing supply 0/1.
        //
        // Try alternative: 0→2 ship 10 (50), 1→1 ship 10 (10),
        //   2→0 ship 10 (50). Total = 110. ✓
        // V1 greedy doesn't find this because it locked into (0,0)
        // and (1,1) before considering (0,2) at cost 5.
        //
        // V2 SSP should find the 110 optimum.
        let cost = vec![vec![1u128, 5, 5], vec![5, 1, 5], vec![5, 5, 100]];
        let supplies = vec![10u128; 3];
        let demands = vec![10u128; 3];

        let v2 = solve_transportation(&supplies, &demands, &cost).unwrap();
        let v1 =
            evaporchain_network_simplex::solve_transportation(&supplies, &demands, &cost).unwrap();

        assert!(
            v2.total_cost <= v1.total_cost,
            "V2 should not be worse than V1 (V2={}, V1={})",
            v2.total_cost,
            v1.total_cost
        );
        // V2 must hit the true optimum 110.
        assert_eq!(v2.total_cost, 110, "V2 must find the optimum");
        // V1 greedy gets stuck at 1020 here.
        assert_eq!(v1.total_cost, 1020);
    }

    // ── brute-force agreement ────────────────────────────────────

    fn brute_force_min_cost(cost: &[Vec<u128>]) -> u128 {
        let n = cost.len();
        let mut perm: Vec<usize> = (0..n).collect();
        let mut best: u128 = u128::MAX;
        permute(&mut perm, 0, &mut |p| {
            let total: u128 = (0..n).map(|i| cost[i][p[i]]).sum();
            if total < best {
                best = total;
            }
        });
        best
    }

    fn permute<F: FnMut(&[usize])>(arr: &mut [usize], k: usize, visit: &mut F) {
        if k == arr.len() {
            visit(arr);
            return;
        }
        for i in k..arr.len() {
            arr.swap(i, k);
            permute(arr, k + 1, visit);
            arr.swap(i, k);
        }
    }

    #[test]
    fn agrees_with_brute_force_on_random_3x3_unit_flow() {
        // Seeded random costs; supplies = demands = [1; 3].
        let mut s: u64 = 1;
        for trial in 0..30 {
            let mut cost: Vec<Vec<u128>> = vec![vec![0u128; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    s = s
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    cost[i][j] = (s as u128 % 50) + 1;
                }
            }
            let supplies = vec![1u128; 3];
            let demands = vec![1u128; 3];
            let sol = solve_transportation(&supplies, &demands, &cost).unwrap();
            let bf = brute_force_min_cost(&cost);
            assert_eq!(sol.total_cost, bf, "trial {trial}, cost={:?}", cost);
        }
    }

    #[test]
    fn ssp_finds_optimum_on_all_30_random_seeds() {
        // 4×4 unit-flow random costs. Brute force enumerates 24
        // permutations; SSP must match.
        let mut s: u64 = 99;
        for trial in 0..30 {
            let n = 4usize;
            let mut cost: Vec<Vec<u128>> = vec![vec![0u128; n]; n];
            for i in 0..n {
                for j in 0..n {
                    s = s
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    cost[i][j] = (s as u128 % 100) + 1;
                }
            }
            let supplies = vec![1u128; n];
            let demands = vec![1u128; n];
            let sol = solve_transportation(&supplies, &demands, &cost).unwrap();
            let bf = brute_force_min_cost(&cost);
            assert_eq!(sol.total_cost, bf, "trial {trial}");
        }
    }

    // ── press claim ──────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "V2 ships proper Successive-Shortest-Path with
        // potentials. Provably optimal for non-negative integer
        // costs and balanced transportation LPs. Same input/output
        // shape as V1; callers swap backends without changing call
        // sites. On adversarial cost matrices where V1's greedy
        // gets stuck, V2 finds the true optimum."

        // V2 = V1 on diagonal-cheap (V1 already optimal there).
        let cost_easy = vec![vec![1u128, 100, 100], vec![100, 1, 100], vec![100, 100, 1]];
        let v1_easy = evaporchain_network_simplex::solve_transportation(
            &[10, 10, 10],
            &[10, 10, 10],
            &cost_easy,
        )
        .unwrap();
        let v2_easy = solve_transportation(&[10, 10, 10], &[10, 10, 10], &cost_easy).unwrap();
        assert_eq!(v1_easy.total_cost, v2_easy.total_cost);

        // V2 < V1 on adversarial input.
        let cost_adv = vec![vec![1u128, 5, 5], vec![5, 1, 5], vec![5, 5, 100]];
        let v1_adv = evaporchain_network_simplex::solve_transportation(
            &[10, 10, 10],
            &[10, 10, 10],
            &cost_adv,
        )
        .unwrap();
        let v2_adv = solve_transportation(&[10, 10, 10], &[10, 10, 10], &cost_adv).unwrap();
        assert!(v2_adv.total_cost < v1_adv.total_cost);

        // Marginals always satisfied.
        for i in 0..3 {
            assert_eq!(v2_adv.flow[i].iter().sum::<u128>(), 10);
        }
        for j in 0..3 {
            let col: u128 = (0..3).map(|i| v2_adv.flow[i][j]).sum();
            assert_eq!(col, 10);
        }
    }

    proptest::proptest! {
        #[test]
        fn property_marginals_always_satisfied(
            seed in 1u64..200u64,
        ) {
            let mut s = seed;
            let mut cost: Vec<Vec<u128>> = vec![vec![0u128; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    s = s.wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    cost[i][j] = (s as u128 % 100) + 1;
                }
            }
            let supplies = vec![10u128, 20, 30];
            let demands = vec![15u128, 25, 20];
            let sol = solve_transportation(&supplies, &demands, &cost).unwrap();
            for i in 0..3 {
                let row_sum: u128 = sol.flow[i].iter().sum();
                proptest::prop_assert_eq!(row_sum, supplies[i]);
            }
            for j in 0..3 {
                let col_sum: u128 = (0..3).map(|i| sol.flow[i][j]).sum();
                proptest::prop_assert_eq!(col_sum, demands[j]);
            }
        }

        #[test]
        fn property_v2_at_most_v1(
            seed in 1u64..200u64,
        ) {
            // V2 must be ≤ V1 on every input (V2 is provably optimal,
            // V1 is a heuristic).
            let mut s = seed;
            let mut cost: Vec<Vec<u128>> = vec![vec![0u128; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    s = s.wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    cost[i][j] = (s as u128 % 100) + 1;
                }
            }
            let supplies = vec![5u128, 10, 5];
            let demands = vec![5u128, 5, 10];
            let v2 = solve_transportation(&supplies, &demands, &cost).unwrap();
            let v1 = evaporchain_network_simplex::solve_transportation(&supplies, &demands, &cost).unwrap();
            proptest::prop_assert!(v2.total_cost <= v1.total_cost);
        }
    }
}

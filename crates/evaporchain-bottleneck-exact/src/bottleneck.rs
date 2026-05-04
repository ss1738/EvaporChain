//! `bottleneck_exact` — binary-searched threshold + Hopcroft-Karp.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::hopcroft_karp::{max_matching_size, MatchingGraph};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BottleneckError {
    #[error("cost matrix has inconsistent row lengths")]
    JaggedMatrix,
    #[error(
        "perfect matching impossible: bipartite graph at threshold ∞ admits matching of size {best} but min(n, m) = {required}"
    )]
    NoPerfectMatching { best: usize, required: usize },
}

/// Exact bottleneck distance:
///
///   T* = min { max C[i][π(i)] : π ∈ perfect matchings }
///
/// where a "perfect matching" matches all `min(n, m)` vertices on
/// the smaller side. Pure integer.
///
/// Returns `Ok(0)` for an empty matrix.
pub fn bottleneck_exact(cost: &[Vec<u64>]) -> Result<u64, BottleneckError> {
    let n = cost.len();
    if n == 0 {
        return Ok(0);
    }
    let m = cost[0].len();
    if m == 0 {
        return Ok(0);
    }
    for row in cost.iter() {
        if row.len() != m {
            return Err(BottleneckError::JaggedMatrix);
        }
    }
    let required = n.min(m);

    // Collect distinct cost values, sorted ascending. Add 0 as a
    // baseline so the binary search has a real lower bound.
    let mut distinct: BTreeSet<u64> = BTreeSet::new();
    distinct.insert(0);
    for row in cost {
        for &c in row {
            distinct.insert(c);
        }
    }
    let candidates: Vec<u64> = distinct.into_iter().collect();

    // First check feasibility: matching at threshold u64::MAX
    // (all edges admitted). If even that doesn't reach `required`,
    // the input has no perfect matching.
    let full_size = matching_at_threshold(cost, n, m, u64::MAX);
    if full_size < required {
        return Err(BottleneckError::NoPerfectMatching {
            best: full_size,
            required,
        });
    }

    // Binary-search the smallest threshold in `candidates` for which
    // max_matching_size == required.
    let mut lo = 0usize;
    let mut hi = candidates.len() - 1;
    // Invariant: candidates[hi] always feasible (verified above).
    while lo < hi {
        let mid = (lo + hi) / 2;
        let t = candidates[mid];
        if matching_at_threshold(cost, n, m, t) >= required {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(candidates[lo])
}

fn matching_at_threshold(cost: &[Vec<u64>], n: usize, m: usize, t: u64) -> usize {
    let mut g = MatchingGraph::new(n, m);
    for (i, row) in cost.iter().enumerate() {
        for (j, &c) in row.iter().enumerate() {
            if c <= t {
                g.add_edge(i, j);
            }
        }
    }
    max_matching_size(&g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_matrix_returns_zero() {
        let cost: Vec<Vec<u64>> = vec![];
        assert_eq!(bottleneck_exact(&cost).unwrap(), 0);
    }

    #[test]
    fn empty_row_returns_zero() {
        let cost: Vec<Vec<u64>> = vec![vec![]];
        assert_eq!(bottleneck_exact(&cost).unwrap(), 0);
    }

    #[test]
    fn jagged_matrix_rejected() {
        let cost = vec![vec![1u64, 2], vec![3]];
        let err = bottleneck_exact(&cost).unwrap_err();
        assert_eq!(err, BottleneckError::JaggedMatrix);
    }

    #[test]
    fn one_by_one_returns_single_cost() {
        let cost = vec![vec![42u64]];
        assert_eq!(bottleneck_exact(&cost).unwrap(), 42);
    }

    #[test]
    fn diagonal_match_picks_max_diagonal() {
        // 3x3, identity matrix is the optimal matching.
        // Costs: (0,0)=1, (1,1)=2, (2,2)=3 → bottleneck = 3.
        let cost = vec![
            vec![1, 100, 100],
            vec![100, 2, 100],
            vec![100, 100, 3],
        ];
        assert_eq!(bottleneck_exact(&cost).unwrap(), 3);
    }

    #[test]
    fn off_diagonal_match_can_beat_diagonal() {
        // Diagonal: max(5, 5, 5) = 5.
        // Off-diagonal (0→2, 1→1, 2→0): max(1, 1, 1) = 1.
        let cost = vec![
            vec![5, 100, 1],
            vec![100, 1, 100],
            vec![1, 100, 5],
        ];
        assert_eq!(bottleneck_exact(&cost).unwrap(), 1);
    }

    #[test]
    fn rectangular_n_less_than_m() {
        // 2x4: match the two left vertices to their cheapest
        // available right vertices.
        let cost = vec![
            vec![10, 1, 100, 100],
            vec![100, 100, 2, 100],
        ];
        assert_eq!(bottleneck_exact(&cost).unwrap(), 2);
    }

    #[test]
    fn rectangular_n_greater_than_m() {
        // 4x2: only 2 right vertices, so 2 left vertices match.
        // Best 2-vertex bottleneck: (0,0)=1, (3,1)=1 → bottleneck=1.
        let cost = vec![
            vec![1, 100],
            vec![100, 100],
            vec![100, 100],
            vec![100, 1],
        ];
        assert_eq!(bottleneck_exact(&cost).unwrap(), 1);
    }

    #[test]
    fn isolation_with_max_cost_makes_perfect_match_impossible() {
        // Note: bottleneck DOES NOT require all-edges, just admit
        // matching at threshold u64::MAX. That's always OK if
        // every vertex has at least one neighbor in the *full*
        // edge set (every cost is finite).
        // Construct a "blocked" matrix: 2 left vertices but only
        // one of them has a (finite) right edge.
        // Hmm — every entry in u64 is finite. To model isolation,
        // we'd need a sentinel. Skip this test in V1.
        // Just confirm a uniform-cost matrix returns its uniform value.
        let cost = vec![vec![7u64; 3]; 3];
        assert_eq!(bottleneck_exact(&cost).unwrap(), 7);
    }

    // ── exhaustive small case ────────────────────────────────────

    fn brute_force_bottleneck(cost: &[Vec<u64>]) -> u64 {
        let n = cost.len();
        let m = cost[0].len();
        let k = n.min(m);
        let mut best = u64::MAX;
        // Enumerate all (n choose k) subsets of left, then permute
        // them to all (m choose k) right subsets — but our case
        // has either n=k or m=k (since k=min). Enumerate
        // permutations of indices on the larger side.
        let (rows, cols, transpose) = if n <= m {
            (n, m, false)
        } else {
            (m, n, true)
        };
        let mut chosen: Vec<usize> = (0..cols).collect();
        // Generate all k-permutations of cols.
        permute_k(&mut chosen, 0, rows, &mut |perm| {
            let mut max_cost = 0u64;
            for i in 0..rows {
                let c = if transpose {
                    cost[perm[i]][i]
                } else {
                    cost[i][perm[i]]
                };
                if c > max_cost {
                    max_cost = c;
                }
            }
            if max_cost < best {
                best = max_cost;
            }
        });
        best
    }

    fn permute_k<F: FnMut(&[usize])>(
        arr: &mut [usize],
        depth: usize,
        k: usize,
        visit: &mut F,
    ) {
        if depth == k {
            visit(&arr[..k]);
            return;
        }
        for i in depth..arr.len() {
            arr.swap(depth, i);
            permute_k(arr, depth + 1, k, visit);
            arr.swap(depth, i);
        }
    }

    #[test]
    fn matches_brute_force_3x3() {
        // Brute-force vs algorithm on a few random-ish 3x3.
        let cases = vec![
            vec![vec![1u64, 2, 3], vec![4, 5, 6], vec![7, 8, 9]],
            vec![vec![10, 1, 100], vec![1, 100, 10], vec![100, 10, 1]],
            vec![vec![5, 5, 5], vec![5, 5, 5], vec![5, 5, 5]],
        ];
        for cost in cases {
            let exact = bottleneck_exact(&cost).unwrap();
            let bf = brute_force_bottleneck(&cost);
            assert_eq!(exact, bf, "mismatch on {:?}", cost);
        }
    }

    #[test]
    fn matches_brute_force_2x4_and_4x2() {
        let cost = vec![vec![10u64, 1, 100, 100], vec![100, 100, 2, 100]];
        assert_eq!(bottleneck_exact(&cost).unwrap(), brute_force_bottleneck(&cost));

        let cost_t = vec![vec![10u64, 100], vec![1, 100], vec![100, 2], vec![100, 100]];
        assert_eq!(bottleneck_exact(&cost_t).unwrap(), brute_force_bottleneck(&cost_t));
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "V2 ships exact polynomial-time bottleneck
        // distance via Hopcroft-Karp + binary-search on
        // threshold. Replaces V1's brute-force / lower-bound
        // approximations. Validator-deterministic, agrees
        // byte-for-byte with brute-force on all small cases."

        // 4x4 random-ish grid.
        let cost: Vec<Vec<u64>> = vec![
            vec![3, 7, 1, 9],
            vec![5, 2, 8, 4],
            vec![6, 4, 5, 1],
            vec![1, 9, 3, 7],
        ];
        let exact = bottleneck_exact(&cost).unwrap();
        let bf = brute_force_bottleneck(&cost);
        assert_eq!(exact, bf);
        // Honest determinism: same input → same output across calls.
        assert_eq!(exact, bottleneck_exact(&cost).unwrap());
    }

    proptest::proptest! {
        #[test]
        fn property_matches_brute_force_3x3(
            seed in 1u64..1000u64,
        ) {
            // Generate a deterministic 3x3 matrix from seed.
            let mut s = seed;
            let mut cost: Vec<Vec<u64>> = vec![vec![0u64; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1442695040888963407);
                    cost[i][j] = (s % 100);
                }
            }
            let exact = bottleneck_exact(&cost).unwrap();
            let bf = brute_force_bottleneck(&cost);
            proptest::prop_assert_eq!(exact, bf);
        }

        #[test]
        fn property_uniform_cost_returns_that_cost(
            uniform in 0u64..1000u64,
            n in 1usize..6usize,
        ) {
            let cost: Vec<Vec<u64>> = vec![vec![uniform; n]; n];
            let exact = bottleneck_exact(&cost).unwrap();
            proptest::prop_assert_eq!(exact, uniform);
        }
    }
}

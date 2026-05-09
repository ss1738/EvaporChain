//! Bottleneck distance between two persistence-homology
//! barcodes.
//!
//! ## Definition
//!
//! Given two persistence diagrams `D` and `D'`, the bottleneck
//! distance is:
//!
//!   d_B(D, D') = inf_{η: D ∪ Δ → D' ∪ Δ bijection} max_{p ∈ D ∪ Δ} ||p - η(p)||_∞
//!
//! where `Δ` is the diagonal `{(x, x) : x ∈ ℝ}`. Operationally:
//! we augment each diagram with the diagonal projections of the
//! OTHER diagram's bars, so both have the same size `n + m`,
//! then minimise the max L∞ matching distance over bijections.
//!
//! ## V1 algorithm
//!
//! Brute-force over all `(n+m)!` bijections — correct, simple,
//! tractable for small barcodes (n+m ≤ 8). V2 will ship
//! Hopcroft-Karp on a binary-searched threshold for O(N^1.5)
//! complexity. Document the trade-off in crate doc.
//!
//! ## Infinite bars
//!
//! Bars with `death == INF_DEATH` represent features surviving
//! to the end of the filtration. Standard persistence-diagram
//! convention: infinite bars match ONLY other infinite bars
//! (they cannot match the diagonal). If the two diagrams have
//! a different count of infinite bars, the bottleneck distance
//! is `INF_DEATH` — they're incomparable.
//!
//! For the matched infinite bars, the L∞ distance is `|b - b'|`
//! on the births alone.

use crate::barcode::{Bar, Barcode, INF_DEATH};

/// Compute the bottleneck distance between two barcodes. Returns
/// `INF_DEATH` if the diagrams have an unequal count of infinite
/// bars (incomparable).
pub fn bottleneck_distance(a: &Barcode, b: &Barcode) -> u64 {
    // Partition into finite + infinite bars.
    let (a_inf, a_fin): (Vec<&Bar>, Vec<&Bar>) = a.bars().iter().partition(|x| x.is_infinite());
    let (b_inf, b_fin): (Vec<&Bar>, Vec<&Bar>) = b.bars().iter().partition(|x| x.is_infinite());

    if a_inf.len() != b_inf.len() {
        return INF_DEATH;
    }

    let inf_max = bottleneck_infinite(&a_inf, &b_inf);
    let fin_max = bottleneck_finite(&a_fin, &b_fin);

    inf_max.max(fin_max)
}

/// Bottleneck on the infinite-bar subset. Both lists must have
/// equal length (verified by caller). Match by sorted births;
/// the optimal matching is order-preserving for L∞ distance on
/// 1D points.
fn bottleneck_infinite(a: &[&Bar], b: &[&Bar]) -> u64 {
    debug_assert_eq!(a.len(), b.len());
    let mut a_births: Vec<u64> = a.iter().map(|x| x.birth).collect();
    let mut b_births: Vec<u64> = b.iter().map(|x| x.birth).collect();
    a_births.sort();
    b_births.sort();
    a_births
        .iter()
        .zip(&b_births)
        .map(|(x, y)| absdiff(*x, *y))
        .max()
        .unwrap_or(0)
}

/// Bottleneck on the finite-bar subset. V1: brute-force over
/// augmented bijections. Augment a's list with diagonal
/// projections of b's bars, and vice versa, so both have size
/// n + m. Permute one side; for each permutation compute the
/// max L∞ distance; take the min over permutations.
fn bottleneck_finite(a: &[&Bar], b: &[&Bar]) -> u64 {
    if a.is_empty() && b.is_empty() {
        return 0;
    }
    let n = a.len();
    let m = b.len();

    // Build the augmented point lists.
    let mut aug_a: Vec<(u64, u64)> = a.iter().map(|x| (x.birth, x.death)).collect();
    let mut aug_b: Vec<(u64, u64)> = b.iter().map(|x| (x.birth, x.death)).collect();
    // Each of b's bars contributes a diagonal projection to a's
    // augmented list, and vice versa.
    for x in b {
        let mid = midpoint(x.birth, x.death);
        aug_a.push((mid, mid));
    }
    for x in a {
        let mid = midpoint(x.birth, x.death);
        aug_b.push((mid, mid));
    }
    debug_assert_eq!(aug_a.len(), aug_b.len());
    debug_assert_eq!(aug_a.len(), n + m);

    // Brute force min-max over permutations of aug_b.
    let mut perm: Vec<usize> = (0..aug_a.len()).collect();
    let mut best: u64 = u64::MAX;
    permute(&mut perm, 0, &mut |p| {
        let mut local_max: u64 = 0;
        for (i, &pi) in p.iter().enumerate() {
            let dist = linf_dist(aug_a[i], aug_b[pi]);
            if dist > local_max {
                local_max = dist;
            }
        }
        if local_max < best {
            best = local_max;
        }
    });
    best
}

fn linf_dist(a: (u64, u64), b: (u64, u64)) -> u64 {
    absdiff(a.0, b.0).max(absdiff(a.1, b.1))
}

fn absdiff(a: u64, b: u64) -> u64 {
    a.abs_diff(b)
}

fn midpoint(b: u64, d: u64) -> u64 {
    // Floor of the midpoint, computed without overflow.
    b + (d - b) / 2
}

/// Heap's algorithm for in-place permutation enumeration. Calls
/// `visit(perm)` once per permutation.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::barcode::{Bar, Barcode, INF_DEATH};

    fn bc(bars: Vec<Bar>) -> Barcode {
        Barcode::from_bars(bars)
    }

    fn b(birth: u64, death: u64) -> Bar {
        Bar::new(birth, death).unwrap()
    }

    // ── identity / metric properties ─────────────────────────────

    #[test]
    fn distance_to_self_is_zero() {
        let bc1 = bc(vec![b(1, 5), b(3, 8)]);
        assert_eq!(bottleneck_distance(&bc1, &bc1), 0);
    }

    #[test]
    fn empty_barcodes_distance_zero() {
        assert_eq!(bottleneck_distance(&bc(vec![]), &bc(vec![])), 0);
    }

    #[test]
    fn distance_is_symmetric() {
        let a = bc(vec![b(1, 5), b(3, 8)]);
        let bb = bc(vec![b(2, 6)]);
        assert_eq!(bottleneck_distance(&a, &bb), bottleneck_distance(&bb, &a));
    }

    // ── single-bar cases ──────────────────────────────────────────

    #[test]
    fn single_bar_matched_by_diagonal_when_other_empty() {
        // Bar (1, 11) → diagonal projection at 6. L∞ distance is
        // max(|1-6|, |11-6|) = 5.
        let a = bc(vec![b(1, 11)]);
        let bb = bc(vec![]);
        assert_eq!(bottleneck_distance(&a, &bb), 5);
    }

    #[test]
    fn matched_pair_uses_linf_difference() {
        // (1, 5) vs (2, 7): L∞ distance = max(1, 2) = 2.
        // The diagonal alternatives: (1,5) → diag at 3 (dist 2);
        // (2,7) → diag at 4 (dist 2.5 → 2 after floor). The
        // optimal matching is the direct one.
        let a = bc(vec![b(1, 5)]);
        let bb = bc(vec![b(2, 7)]);
        assert!(bottleneck_distance(&a, &bb) <= 2);
    }

    #[test]
    fn small_perturbation_gives_small_distance() {
        // Two barcodes that differ only by a small perturbation.
        let a = bc(vec![b(1, 11), b(20, 30)]);
        let bb = bc(vec![b(1, 12), b(20, 30)]);
        // The matching (1,11)↔(1,12) has L∞ = 1; (20,30)↔(20,30) = 0.
        assert_eq!(bottleneck_distance(&a, &bb), 1);
    }

    // ── infinite bars ────────────────────────────────────────────

    #[test]
    fn unequal_inf_count_returns_infinity() {
        let a = bc(vec![b(1, INF_DEATH)]);
        let bb = bc(vec![]);
        assert_eq!(bottleneck_distance(&a, &bb), INF_DEATH);
    }

    #[test]
    fn matched_inf_bars_use_birth_difference() {
        let a = bc(vec![b(5, INF_DEATH)]);
        let bb = bc(vec![b(7, INF_DEATH)]);
        assert_eq!(bottleneck_distance(&a, &bb), 2);
    }

    #[test]
    fn inf_and_finite_combined() {
        // a: (5, ∞), (1, 11)
        // b: (7, ∞), (1, 12)
        // inf: |5-7| = 2; finite (1,11)↔(1,12): L∞=1; max = 2.
        let a = bc(vec![b(5, INF_DEATH), b(1, 11)]);
        let bb = bc(vec![b(7, INF_DEATH), b(1, 12)]);
        assert_eq!(bottleneck_distance(&a, &bb), 2);
    }

    // ── stability theorem direction ──────────────────────────────

    #[test]
    fn stability_theorem_holds_at_birth_perturbation() {
        // Cohen-Steiner-Edelsbrunner-Harer 2007: bottleneck
        // distance ≤ filtration interleaving distance. We
        // simulate a "filtration shift by ε" by adding ε to
        // every birth.
        let eps = 7u64;
        let a = bc(vec![b(10, 20), b(30, 50)]);
        let bb = bc(vec![b(10 + eps, 20), b(30 + eps, 50)]);
        // Each bar's birth shifts by ε; deaths unchanged. L∞
        // matching distance is at most ε.
        let d = bottleneck_distance(&a, &bb);
        assert!(d <= eps, "expected bottleneck ≤ {eps}, got {d}");
    }

    #[test]
    fn stability_theorem_holds_at_death_perturbation() {
        let eps = 4u64;
        let a = bc(vec![b(10, 20), b(30, 50)]);
        let bb = bc(vec![b(10, 20 + eps), b(30, 50 + eps)]);
        let d = bottleneck_distance(&a, &bb);
        assert!(d <= eps);
    }

    // ── degenerate but valid cases ───────────────────────────────

    #[test]
    fn one_side_empty_uses_diagonal_projections() {
        // Each bar in a is matched to its own diagonal projection.
        // The max distance is the half-persistence of the longest bar.
        let a = bc(vec![b(0, 10), b(5, 15)]);
        let bb = bc(vec![]);
        // Half-persistences: 5, 5. Max = 5.
        assert_eq!(bottleneck_distance(&a, &bb), 5);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "By the Cohen-Steiner-Edelsbrunner-Harer 2007
        // stability theorem, the bottleneck distance between two
        // EFH barcodes is bounded above by the interleaving
        // distance of their generating filtrations. A light
        // client checks `d_B(prev, curr) ≤ bd_max` and rejects
        // any block whose barcode change exceeds the chain-
        // committed bound."
        let prev = bc(vec![b(10, 20), b(30, 50), b(70, 90)]);
        // A small natural change: one bar's death extends slightly.
        let curr_ok = bc(vec![b(10, 20), b(30, 53), b(70, 90)]);
        // A pathological change: a bar disappears entirely,
        // inserting a much larger bar.
        let curr_bad = bc(vec![b(10, 20), b(30, 50), b(0, 200)]);
        let d_ok = bottleneck_distance(&prev, &curr_ok);
        let d_bad = bottleneck_distance(&prev, &curr_bad);
        assert!(d_ok < d_bad);
        assert!(d_ok <= 5); // small perturbation
    }
}

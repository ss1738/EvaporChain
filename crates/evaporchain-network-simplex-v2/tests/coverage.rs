//! Coverage tests for V2 Successive-Shortest-Path transportation LP
//! solver. V1 ships a greedy heuristic; V2 is provably optimal for
//! non-negative integer costs + balanced supply/demand.
//!
//! Existing in-module tests cover the happy paths. This file adds:
//!
//!   - Input-validation error paths (Empty / Dimension / Jagged /
//!     Imbalanced)
//!   - Solution invariants (row sums = supplies, column sums =
//!     demands, flow ⊆ [0, supply_total])
//!   - Optimality cross-checks against hand-computed optima
//!   - Trivial cases (1×1, zero-supply rows)
//!   - Error type Display + Eq ergonomics
//!   - Serde round-trip of `TransportSolution`

use evaporchain_network_simplex_v2::{
    solve_transportation, TransportError, TransportSolution,
};

// =================================================================
// Input validation
// =================================================================

#[test]
fn empty_supplies_errors() {
    let err = solve_transportation(&[], &[10], &[]).unwrap_err();
    assert_eq!(err, TransportError::EmptyInput);
}

#[test]
fn empty_demands_errors() {
    let err = solve_transportation(&[10], &[], &[vec![]]).unwrap_err();
    assert_eq!(err, TransportError::EmptyInput);
}

#[test]
fn cost_rows_mismatch_supplies_errors() {
    // 2 supplies, but cost matrix has only 1 row.
    let err = solve_transportation(
        &[10, 10],
        &[20],
        &[vec![1]],
    )
    .unwrap_err();
    match err {
        TransportError::DimensionMismatch { n_supply, n, .. } => {
            assert_eq!(n_supply, 2);
            assert_eq!(n, 1);
        }
        other => panic!("expected DimensionMismatch, got {other:?}"),
    }
}

#[test]
fn jagged_cost_matrix_errors() {
    // First row has 2 cols, second has 3 → JaggedMatrix.
    let err = solve_transportation(
        &[5, 5],
        &[5, 5],
        &[vec![1, 2], vec![3, 4, 5]],
    )
    .unwrap_err();
    assert_eq!(err, TransportError::JaggedMatrix);
}

#[test]
fn imbalanced_supplies_vs_demands_errors() {
    let err = solve_transportation(
        &[10, 10],
        &[5, 5],
        &[vec![1, 1], vec![1, 1]],
    )
    .unwrap_err();
    match err {
        TransportError::Imbalanced { supply_total, demand_total } => {
            assert_eq!(supply_total, 20);
            assert_eq!(demand_total, 10);
        }
        other => panic!("expected Imbalanced, got {other:?}"),
    }
}

#[test]
fn imbalanced_demand_greater_than_supply_errors() {
    let err = solve_transportation(
        &[5],
        &[10],
        &[vec![1]],
    )
    .unwrap_err();
    assert!(matches!(err, TransportError::Imbalanced { .. }));
}

// =================================================================
// Trivial / edge cases
// =================================================================

#[test]
fn trivial_one_by_one_ships_full_supply() {
    let sol = solve_transportation(&[7], &[7], &[vec![3]]).unwrap();
    assert_eq!(sol.flow, vec![vec![7]]);
    assert_eq!(sol.total_cost, 21); // 7 × 3
}

#[test]
fn one_by_one_zero_cost_succeeds() {
    let sol = solve_transportation(&[5], &[5], &[vec![0]]).unwrap();
    assert_eq!(sol.flow, vec![vec![5]]);
    assert_eq!(sol.total_cost, 0);
}

#[test]
fn supplier_with_zero_supply_contributes_no_flow() {
    // Supplier 0 has 0 supply, supplier 1 has all.
    let sol = solve_transportation(
        &[0, 10],
        &[10],
        &[vec![1], vec![3]],
    )
    .unwrap();
    assert_eq!(sol.flow[0][0], 0);
    assert_eq!(sol.flow[1][0], 10);
    assert_eq!(sol.total_cost, 30); // 10 × 3
}

// =================================================================
// Optimality cross-checks
// =================================================================

#[test]
fn two_by_two_routes_to_cheapest_per_destination() {
    // Both demanders take entire 5 from cheapest supplier each.
    // S0 → D0 (cost 1), S1 → D1 (cost 1).
    // Other diagonal: S0 → D1 (cost 5), S1 → D0 (cost 5).
    // Optimal: ship S0→D0, S1→D1. Cost = 5*1 + 5*1 = 10.
    let sol = solve_transportation(
        &[5, 5],
        &[5, 5],
        &[vec![1, 5], vec![5, 1]],
    )
    .unwrap();
    assert_eq!(sol.total_cost, 10);
    assert_eq!(sol.flow[0][0], 5);
    assert_eq!(sol.flow[1][1], 5);
    assert_eq!(sol.flow[0][1], 0);
    assert_eq!(sol.flow[1][0], 0);
}

#[test]
fn two_by_two_anti_diagonal_optimal() {
    // Anti-diagonal cheapest: S0→D1 and S1→D0 cost 1 each;
    // diagonal costs 5. Optimal = ship across the anti-diagonal.
    let sol = solve_transportation(
        &[3, 7],
        &[7, 3],
        &[vec![5, 1], vec![1, 5]],
    )
    .unwrap();
    assert_eq!(sol.flow[0][1], 3);
    assert_eq!(sol.flow[1][0], 7);
    // Cost = 3 + 7 = 10.
    assert_eq!(sol.total_cost, 10);
}

#[test]
fn three_by_three_uniform_cost_any_assignment_optimal() {
    // All costs equal → every feasible flow has the same total cost.
    // Pin: solver returns a feasible solution with the correct total.
    let sol = solve_transportation(
        &[10, 10, 10],
        &[15, 10, 5],
        &[vec![2, 2, 2], vec![2, 2, 2], vec![2, 2, 2]],
    )
    .unwrap();
    // Total cost = 30 (total flow) × 2 (per-unit cost) = 60.
    assert_eq!(sol.total_cost, 60);
    // Row sums = supplies.
    for i in 0..3 {
        let row_sum: u128 = sol.flow[i].iter().sum();
        assert_eq!(row_sum, 10, "row {i} sum must equal supplies[{i}]");
    }
    // Column sums = demands.
    for j in 0..3 {
        let col_sum: u128 = (0..3).map(|i| sol.flow[i][j]).sum();
        let expected = [15u128, 10, 5][j];
        assert_eq!(col_sum, expected, "col {j} sum must equal demands[{j}]");
    }
}

#[test]
fn zero_cost_route_is_used_preferentially() {
    // One zero-cost edge plus expensive alternatives. Optimal must
    // saturate the zero-cost edge.
    let sol = solve_transportation(
        &[10, 10],
        &[10, 10],
        &[
            vec![0, 100], // S0 → D0 is free
            vec![100, 50],
        ],
    )
    .unwrap();
    assert_eq!(sol.flow[0][0], 10, "free edge must be saturated");
    assert_eq!(sol.flow[1][1], 10, "remaining demand routed via S1→D1");
    assert_eq!(sol.total_cost, 0 + 10 * 50);
}

// =================================================================
// TransportError ergonomics
// =================================================================

#[test]
fn transport_error_displays_all_variants() {
    let e = TransportError::EmptyInput.to_string();
    let j = TransportError::JaggedMatrix.to_string();
    let d = TransportError::DimensionMismatch { n_supply: 2, n_demand: 3, n: 1, m: 2 }
        .to_string();
    let i = TransportError::Imbalanced { supply_total: 10, demand_total: 5 }.to_string();
    assert!(e.contains("empty"), "got: {e}");
    assert!(j.contains("inconsistent") || j.contains("jagged") || j.contains("row"), "got: {j}");
    assert!(d.contains("2") && d.contains("3"), "got: {d}");
    assert!(i.contains("10") && i.contains("5"), "got: {i}");
}

#[test]
fn transport_error_eq_discriminates() {
    let a = TransportError::Imbalanced { supply_total: 1, demand_total: 2 };
    let b = TransportError::Imbalanced { supply_total: 1, demand_total: 2 };
    let c = TransportError::Imbalanced { supply_total: 1, demand_total: 3 };
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, TransportError::EmptyInput);
}

// =================================================================
// Serde
// =================================================================

#[test]
fn transport_solution_serde_round_trips() {
    let sol = solve_transportation(
        &[3, 7],
        &[5, 5],
        &[vec![1, 4], vec![2, 3]],
    )
    .unwrap();
    let json = serde_json::to_string(&sol).expect("serialize");
    let back: TransportSolution = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, sol);
}

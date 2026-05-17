//! Coverage tests for the V1 greedy min-cost transportation solver.
//! V1 is the bipartite-case greedy; V2 (network-simplex-v2) is the
//! full SSP with Dijkstra over reduced costs.

use evaporchain_network_simplex::transport::{
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
fn dimension_mismatch_errors() {
    let err = solve_transportation(&[10, 10], &[20], &[vec![1]]).unwrap_err();
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
    let err = solve_transportation(&[5, 5], &[5, 5], &[vec![1, 2], vec![3, 4, 5]]).unwrap_err();
    assert_eq!(err, TransportError::JaggedMatrix);
}

#[test]
fn imbalanced_supplies_vs_demands_errors() {
    let err = solve_transportation(&[10, 10], &[5, 5], &[vec![1, 1], vec![1, 1]]).unwrap_err();
    match err {
        TransportError::Imbalanced { supply_total, demand_total } => {
            assert_eq!(supply_total, 20);
            assert_eq!(demand_total, 10);
        }
        other => panic!("expected Imbalanced, got {other:?}"),
    }
}

// =================================================================
// Trivial cases
// =================================================================

#[test]
fn trivial_one_by_one_ships_full_supply() {
    let sol = solve_transportation(&[7], &[7], &[vec![3]]).unwrap();
    assert_eq!(sol.flow, vec![vec![7]]);
    assert_eq!(sol.total_cost, 21);
}

#[test]
fn one_by_one_zero_cost_succeeds() {
    let sol = solve_transportation(&[5], &[5], &[vec![0]]).unwrap();
    assert_eq!(sol.flow, vec![vec![5]]);
    assert_eq!(sol.total_cost, 0);
}

#[test]
fn supplier_with_zero_supply_contributes_no_flow() {
    let sol = solve_transportation(&[0, 10], &[10], &[vec![1], vec![3]]).unwrap();
    assert_eq!(sol.flow[0][0], 0);
    assert_eq!(sol.flow[1][0], 10);
    assert_eq!(sol.total_cost, 30);
}

// =================================================================
// Greedy optimality (cases where greedy IS optimal)
// =================================================================

#[test]
fn diagonal_cheap_two_by_two() {
    let sol = solve_transportation(&[5, 5], &[5, 5], &[vec![1, 5], vec![5, 1]]).unwrap();
    assert_eq!(sol.total_cost, 10);
    assert_eq!(sol.flow[0][0], 5);
    assert_eq!(sol.flow[1][1], 5);
}

#[test]
fn anti_diagonal_two_by_two() {
    let sol = solve_transportation(&[3, 7], &[7, 3], &[vec![5, 1], vec![1, 5]]).unwrap();
    assert_eq!(sol.flow[0][1], 3);
    assert_eq!(sol.flow[1][0], 7);
    assert_eq!(sol.total_cost, 10);
}

#[test]
fn uniform_cost_satisfies_flow_invariants() {
    let sol = solve_transportation(
        &[10, 10, 10],
        &[15, 10, 5],
        &[vec![2; 3], vec![2; 3], vec![2; 3]],
    )
    .unwrap();
    assert_eq!(sol.total_cost, 60);
    for i in 0..3 {
        assert_eq!(sol.flow[i].iter().sum::<u128>(), 10);
    }
    for j in 0..3 {
        let col: u128 = (0..3).map(|i| sol.flow[i][j]).sum();
        assert_eq!(col, [15u128, 10, 5][j]);
    }
}

#[test]
fn zero_cost_route_used_preferentially() {
    let sol = solve_transportation(
        &[10, 10],
        &[10, 10],
        &[vec![0, 100], vec![100, 50]],
    )
    .unwrap();
    assert_eq!(sol.flow[0][0], 10);
    assert_eq!(sol.flow[1][1], 10);
    assert_eq!(sol.total_cost, 500);
}

// =================================================================
// Error ergonomics
// =================================================================

#[test]
fn transport_error_displays_all_variants() {
    assert!(TransportError::EmptyInput.to_string().contains("empty"));
    assert!(TransportError::JaggedMatrix.to_string().to_lowercase().contains("inconsistent")
        || TransportError::JaggedMatrix.to_string().to_lowercase().contains("jagged")
        || TransportError::JaggedMatrix.to_string().to_lowercase().contains("row"));
    assert!(TransportError::DimensionMismatch { n_supply: 2, n_demand: 3, n: 1, m: 2 }
        .to_string()
        .contains("2"));
    assert!(TransportError::Imbalanced { supply_total: 10, demand_total: 5 }
        .to_string()
        .contains("10"));
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
    let sol = solve_transportation(&[3, 7], &[5, 5], &[vec![1, 4], vec![2, 3]]).unwrap();
    let json = serde_json::to_string(&sol).unwrap();
    let back: TransportSolution = serde_json::from_str(&json).unwrap();
    assert_eq!(back, sol);
}

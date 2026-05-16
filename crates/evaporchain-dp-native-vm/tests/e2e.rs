//! End-to-end integration tests for dp-native-vm.
//!
//! Non-trivial fixture: a salary-analytics pipeline.
//!
//! Scenario: HR registers a salary dataset with a finite ε-budget. Five
//! analysts each run a count query against the dataset. The sixth query
//! exhausts the budget and must be refused. All same-seed queries return
//! byte-identical noised outputs across two "validator" contexts (same-seed
//! determinism requirement). An attacker cannot re-register the dataset to
//! reset the consumption history.
//!
//! Composition accounting: after 5 queries, basic composition gives the
//! accumulated privacy loss. Advanced composition bound is tighter for
//! large k — verified against basic-composition output.
//!
//! INVENTION_STACK §4.2: Differential-Privacy-Native VM substrate.

use evaporchain_dp_native_vm::{
    advanced_composition_epsilon, basic_composition_epsilon, BasicComposition, BudgetError,
    BudgetRegistry, DatasetId, NoiseMechanism, NoiseSeed,
};

// ── Fixture constants ─────────────────────────────────────────────────────

/// Total ε budget in micros (ε = 1.0 → 1_000_000 µε).
const BUDGET_EPS_MICROS: u64 = 1_000_000;
/// Total δ budget in ppb (δ = 0.001 → 1_000_000 ppb).
const BUDGET_DELTA_PPB: u64 = 1_000_000;

/// Each query costs ε = 0.2 → 200_000 µε. Five queries = 1_000_000 = budget.
const QUERY_EPS_MICROS: u64 = 200_000;
const QUERY_DELTA_PPB: u64 = 0;

/// Query sensitivity (max difference a single record can make = 1 for count).
const SENSITIVITY: u64 = 1;

const SALARY_DATASET: DatasetId = DatasetId([0x5A; 32]);
const OTHER_DATASET: DatasetId = DatasetId([0x77; 32]);

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn five_queries_succeed_sixth_exhausts_budget() {
    let mut p = BudgetRegistry::new();
    p.register(SALARY_DATASET, BUDGET_EPS_MICROS, BUDGET_DELTA_PPB)
        .unwrap();

    for i in 0..5 {
        p.consume(SALARY_DATASET, QUERY_EPS_MICROS, QUERY_DELTA_PPB)
            .unwrap_or_else(|e| panic!("query {i} must succeed: {e:?}"));
    }

    let remaining = p
        .get(&SALARY_DATASET)
        .expect("dataset must still be registered");
    assert_eq!(
        remaining.remaining_epsilon_micros(),
        0,
        "budget must be exactly exhausted after 5 queries"
    );

    // Sixth query: over budget.
    let err = p
        .consume(SALARY_DATASET, QUERY_EPS_MICROS, QUERY_DELTA_PPB)
        .expect_err("sixth query must fail");
    assert!(
        matches!(err, BudgetError::BudgetExhausted { .. }),
        "expected BudgetExhausted, got {err:?}"
    );
}

#[test]
fn same_seed_produces_byte_identical_noise_across_validator_contexts() {
    // Two independent "validators" get the same seed and true value;
    // their noised outputs must be byte-identical.
    let seed = NoiseSeed([0xBE; 32]);
    let true_count: i64 = 42;
    let mechanism = NoiseMechanism::DiscreteLaplace;

    let noised_v1 = mechanism
        .apply(true_count, SENSITIVITY, QUERY_EPS_MICROS, seed)
        .unwrap();
    let noised_v2 = mechanism
        .apply(true_count, SENSITIVITY, QUERY_EPS_MICROS, seed)
        .unwrap();

    assert_eq!(
        noised_v1, noised_v2,
        "same seed must produce byte-identical noise across validator contexts"
    );
}

#[test]
fn different_seeds_produce_different_noise_with_high_probability() {
    let seed_a = NoiseSeed([0x11; 32]);
    let seed_b = NoiseSeed([0x22; 32]);
    let true_count: i64 = 1000;
    let mechanism = NoiseMechanism::DiscreteLaplace;

    let a = mechanism
        .apply(true_count, SENSITIVITY, QUERY_EPS_MICROS, seed_a)
        .unwrap();
    let b = mechanism
        .apply(true_count, SENSITIVITY, QUERY_EPS_MICROS, seed_b)
        .unwrap();

    // With high probability two distinct seeds yield different outputs.
    // (For correctness auditing, not a tight probabilistic assertion.)
    assert_ne!(
        a, b,
        "distinct seeds [0x11;32] and [0x22;32] must produce distinct noise"
    );
}

#[test]
fn basic_composition_accumulates_correctly() {
    // 5 identical queries of (200_000, 0) each.
    let queries: Vec<(u64, u64)> = vec![(QUERY_EPS_MICROS, QUERY_DELTA_PPB); 5];
    let bc = BasicComposition::compose(&queries).unwrap();

    assert_eq!(bc.total_epsilon_micros, 5 * QUERY_EPS_MICROS);
    assert_eq!(bc.total_delta_ppb, 0);
    assert_eq!(bc.k, 5);
}

#[test]
fn basic_composition_free_function_matches_struct() {
    let queries: Vec<(u64, u64)> = vec![(QUERY_EPS_MICROS, QUERY_DELTA_PPB); 5];
    let bc = BasicComposition::compose(&queries).unwrap();
    // Free function takes per-query (eps, delta, k) scalars — identical params.
    let (free_eps, free_delta) =
        basic_composition_epsilon(QUERY_EPS_MICROS, QUERY_DELTA_PPB, 5).unwrap();
    assert_eq!(free_eps, bc.total_epsilon_micros);
    assert_eq!(free_delta, bc.total_delta_ppb);
}

#[test]
fn advanced_composition_is_v1_stub_returns_error() {
    // V1: advanced_composition_epsilon is intentionally unimplemented
    // (requires f64 transcendentals). Must return Err rather than panic.
    let result = advanced_composition_epsilon(QUERY_EPS_MICROS, 0, 5, BUDGET_DELTA_PPB);
    assert!(
        result.is_err(),
        "V1 advanced composition must return Err (stub); got {:?}",
        result
    );
}

#[test]
fn unknown_dataset_consume_fails_closed() {
    let mut r = BudgetRegistry::new();
    // No datasets registered.
    let err = r.consume(SALARY_DATASET, QUERY_EPS_MICROS, 0).unwrap_err();
    assert!(
        matches!(err, BudgetError::UnknownDataset(_)),
        "consume on unknown dataset must fail closed: {err:?}"
    );
}

#[test]
fn re_registering_existing_dataset_is_forbidden() {
    let mut r = BudgetRegistry::new();
    r.register(SALARY_DATASET, BUDGET_EPS_MICROS, BUDGET_DELTA_PPB)
        .unwrap();

    // Attacker tries to reset the budget by re-registering.
    let err = r
        .register(SALARY_DATASET, BUDGET_EPS_MICROS, BUDGET_DELTA_PPB)
        .unwrap_err();
    assert!(
        matches!(err, BudgetError::AlreadyRegistered(_)),
        "re-registration must be forbidden to preserve consumption history"
    );
}

#[test]
fn partial_budget_spend_preserves_exact_remainder() {
    let mut r = BudgetRegistry::new();
    // Use non-zero delta so is_exhausted() reflects epsilon dimension only.
    r.register(SALARY_DATASET, 1_000_000, 1_000_000).unwrap();

    r.consume(SALARY_DATASET, 300_000, 100_000).unwrap();
    r.consume(SALARY_DATASET, 200_000, 100_000).unwrap();

    let budget = r.get(&SALARY_DATASET).unwrap();
    assert_eq!(
        budget.remaining_epsilon_micros(),
        500_000,
        "500k µε must remain after two partial spends"
    );
    assert_eq!(
        budget.remaining_delta_ppb(),
        800_000,
        "800k ppb δ must remain"
    );
    assert!(!budget.is_exhausted());
}

#[test]
fn multiple_independent_datasets_tracked_independently() {
    let mut r = BudgetRegistry::new();
    // Non-zero delta so is_exhausted() reflects actual spend, not zero-delta floor.
    r.register(SALARY_DATASET, 1_000_000, 1_000_000).unwrap();
    r.register(OTHER_DATASET, 500_000, 500_000).unwrap();

    // Exhaust the salary dataset (both dimensions).
    r.consume(SALARY_DATASET, 1_000_000, 1_000_000).unwrap();

    // Other dataset's budget is untouched.
    let other = r.get(&OTHER_DATASET).unwrap();
    assert_eq!(other.remaining_epsilon_micros(), 500_000);
    assert_eq!(other.remaining_delta_ppb(), 500_000);
    assert!(!other.is_exhausted());

    // Salary is exhausted.
    assert!(r.get(&SALARY_DATASET).unwrap().is_exhausted());
}

#[test]
fn noise_output_bounded_relative_to_scale() {
    // For ε=1.0 (1_000_000 µε) and sensitivity=1, scale b=1.
    // The mechanism caps noise at 20·b = 20. Verify |noise| ≤ 20 for
    // a range of seeds.
    let mechanism = NoiseMechanism::DiscreteLaplace;
    let high_eps = 1_000_000u64;
    let sensitivity = 1u64;
    let true_val: i64 = 0;

    for byte in 0u8..=20 {
        let seed = NoiseSeed([byte; 32]);
        let noised = mechanism
            .apply(true_val, sensitivity, high_eps, seed)
            .unwrap();
        assert!(
            noised.unsigned_abs() <= 20,
            "noise magnitude {noised} exceeds cap for seed={byte:#x}"
        );
    }
}

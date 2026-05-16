//! Coverage tests for the Causal-CHSH parameter-sweep harness
//! (Lane O.4, `INVENTION_STACK.md §A1.10`). Drives the three cartel
//! injection models across rational grids to find the
//! detection-floor + discrimination-ceiling for the doctrine
//! thresholds {1.8 / 2.2 / 0.4}.
//!
//! Existing in-module tests cover the main run paths. This file
//! adds:
//!
//!   - `SweepGrid::doctrine` shape pin
//!   - `CartelKind` + `GridPoint` serde + Eq
//!   - `run_sweep` empty-honest error path
//!   - Custom (non-doctrine) grid produces correctly-shaped report
//!   - Determinism: same seed → same report
//!   - Different seeds → potentially-different rows (sanity, not strict)
//!   - `SweepError` Display
//!   - Per-row CartelKind grouping invariant (rows[0..11]=Coord,
//!     [11..22]=Coin, [22..33]=PRBox for doctrine grid)
//!   - `SweepReport` serde round-trip

use evaporchain_causal_chsh::chsh::ConcurrentPairSamples;
use evaporchain_causal_chsh::gate::GateThresholds;
use evaporchain_causal_chsh_sweep::{
    run_sweep, CartelKind, GridPoint, SweepError, SweepGrid, SweepReport,
};

fn honest_samples() -> ConcurrentPairSamples {
    // Symmetric ±1 pattern → mean of each correlator is 0 → S = 0
    // (well below the honest-ceiling 1.8). Each bucket needs to be
    // non-empty for run_sweep to start.
    ConcurrentPairSamples {
        samples_ab: vec![1i8, -1, 1, -1, 1, -1],
        samples_ab_prime: vec![1, -1, 1, -1, 1, -1],
        samples_a_prime_b: vec![1, -1, 1, -1, 1, -1],
        samples_a_prime_b_prime: vec![1, -1, 1, -1, 1, -1],
    }
}

fn empty_samples() -> ConcurrentPairSamples {
    ConcurrentPairSamples {
        samples_ab: vec![],
        samples_ab_prime: vec![],
        samples_a_prime_b: vec![],
        samples_a_prime_b_prime: vec![],
    }
}

// =================================================================
// SweepGrid::doctrine shape
// =================================================================

#[test]
fn doctrine_grid_has_11_points_per_model() {
    let g = SweepGrid::doctrine();
    assert_eq!(g.coordinated_subset.len(), 11);
    assert_eq!(g.biased_coin.len(), 11);
    assert_eq!(g.pr_box.len(), 11);
}

#[test]
fn doctrine_grid_spans_0_to_10_over_10() {
    let g = SweepGrid::doctrine();
    for (i, p) in g.coordinated_subset.iter().enumerate() {
        assert_eq!(p.num, i as u64);
        assert_eq!(p.den, 10);
    }
}

// =================================================================
// run_sweep error paths
// =================================================================

#[test]
fn run_sweep_with_all_empty_buckets_errors() {
    let g = SweepGrid::doctrine();
    let err = run_sweep(&empty_samples(), &g, GateThresholds::doctrine(), [0u8; 32]).unwrap_err();
    assert_eq!(err, SweepError::EmptyHonest);
}

#[test]
fn run_sweep_with_one_empty_bucket_errors() {
    let mut s = honest_samples();
    s.samples_ab_prime.clear();
    let g = SweepGrid::doctrine();
    let err = run_sweep(&s, &g, GateThresholds::doctrine(), [0u8; 32]).unwrap_err();
    assert_eq!(err, SweepError::EmptyHonest);
}

// =================================================================
// Report shape
// =================================================================

#[test]
fn doctrine_run_emits_33_rows_in_kind_order() {
    let r = run_sweep(
        &honest_samples(),
        &SweepGrid::doctrine(),
        GateThresholds::doctrine(),
        [42u8; 32],
    )
    .expect("doctrine run");
    assert_eq!(r.rows.len(), 33);
    // First 11 rows = CoordinatedSubset, next 11 = BiasedCoin, last 11 = PrBox.
    for row in &r.rows[0..11] {
        assert_eq!(row.kind, CartelKind::CoordinatedSubset);
    }
    for row in &r.rows[11..22] {
        assert_eq!(row.kind, CartelKind::BiasedCoin);
    }
    for row in &r.rows[22..33] {
        assert_eq!(row.kind, CartelKind::PrBox);
    }
}

#[test]
fn report_carries_honest_s_and_thresholds() {
    let r = run_sweep(
        &honest_samples(),
        &SweepGrid::doctrine(),
        GateThresholds::doctrine(),
        [0u8; 32],
    )
    .unwrap();
    // Honest correlators all 0 → S = 0.
    assert!(r.s_honest.abs() < 1e-9, "honest S must be ≈ 0, got {}", r.s_honest);
    assert_eq!(r.thresholds, GateThresholds::doctrine());
}

#[test]
fn custom_grid_produces_proportional_rows() {
    // 3 points per model = 9 rows.
    let g = SweepGrid {
        coordinated_subset: vec![GridPoint { num: 0, den: 1 }, GridPoint { num: 1, den: 1 }, GridPoint { num: 1, den: 2 }],
        biased_coin: vec![GridPoint { num: 0, den: 1 }, GridPoint { num: 1, den: 1 }, GridPoint { num: 1, den: 2 }],
        pr_box: vec![GridPoint { num: 0, den: 1 }, GridPoint { num: 1, den: 1 }, GridPoint { num: 1, den: 2 }],
    };
    let r = run_sweep(&honest_samples(), &g, GateThresholds::doctrine(), [1u8; 32]).unwrap();
    assert_eq!(r.rows.len(), 9);
}

#[test]
fn empty_grid_per_model_produces_zero_rows() {
    let g = SweepGrid {
        coordinated_subset: vec![],
        biased_coin: vec![],
        pr_box: vec![],
    };
    let r = run_sweep(&honest_samples(), &g, GateThresholds::doctrine(), [0u8; 32]).unwrap();
    assert_eq!(r.rows.len(), 0);
}

// =================================================================
// Determinism
// =================================================================

#[test]
fn same_seed_yields_byte_identical_report() {
    let g = SweepGrid::doctrine();
    let s = honest_samples();
    let t = GateThresholds::doctrine();
    let r1 = run_sweep(&s, &g, t, [99u8; 32]).unwrap();
    let r2 = run_sweep(&s, &g, t, [99u8; 32]).unwrap();
    let json1 = serde_json::to_string(&r1).unwrap();
    let json2 = serde_json::to_string(&r2).unwrap();
    assert_eq!(json1, json2, "seed-deterministic");
}

// =================================================================
// Type-level serde + eq
// =================================================================

#[test]
fn cartel_kind_serde_round_trips() {
    for k in [CartelKind::CoordinatedSubset, CartelKind::BiasedCoin, CartelKind::PrBox] {
        let json = serde_json::to_string(&k).unwrap();
        let back: CartelKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, k);
    }
}

#[test]
fn grid_point_serde_and_eq() {
    let p = GridPoint { num: 3, den: 10 };
    let json = serde_json::to_string(&p).unwrap();
    let back: GridPoint = serde_json::from_str(&json).unwrap();
    assert_eq!(back, p);
    assert_eq!(p, GridPoint { num: 3, den: 10 });
    assert_ne!(p, GridPoint { num: 4, den: 10 });
}

#[test]
fn sweep_report_serde_round_trips() {
    let r = run_sweep(
        &honest_samples(),
        &SweepGrid::doctrine(),
        GateThresholds::doctrine(),
        [0u8; 32],
    )
    .unwrap();
    let json = serde_json::to_string(&r).unwrap();
    let back: SweepReport = serde_json::from_str(&json).unwrap();
    assert_eq!(back.rows.len(), r.rows.len());
    assert_eq!(back.thresholds, r.thresholds);
}

// =================================================================
// SweepError ergonomics
// =================================================================

#[test]
fn sweep_error_empty_honest_displays() {
    let s = SweepError::EmptyHonest.to_string();
    assert!(s.contains("honest") || s.contains("empty"), "got: {s}");
}

#[test]
fn sweep_error_chsh_failure_displays_with_inner() {
    let s = SweepError::HonestChshFailure("inner detail".into()).to_string();
    assert!(s.contains("inner detail"), "inner must be propagated; got: {s}");
}

#[test]
fn sweep_error_eq_discriminates() {
    assert_eq!(SweepError::EmptyHonest, SweepError::EmptyHonest);
    assert_ne!(
        SweepError::EmptyHonest,
        SweepError::HonestChshFailure("x".into())
    );
    assert_eq!(
        SweepError::HonestChshFailure("a".into()),
        SweepError::HonestChshFailure("a".into())
    );
    assert_ne!(
        SweepError::HonestChshFailure("a".into()),
        SweepError::HonestChshFailure("b".into())
    );
}

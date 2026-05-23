//! §TUR Liveness Detector (§A1.3 Barato-Seifert) e2e
//!
//! Scenario: "EvaporChain cartel detection round" — a passive monitor
//! observes block-production rates (J) across four validators over
//! a 16-epoch window. CARTEL (3 colluding validators) produce exactly
//! 10 blocks per epoch — constant, zero variance. HONEST varies
//! naturally. The TUR oracle must flag the cartel and clear the honest.
//!
//! The suite proves: the bound is falsifiable (constants at finite Σ
//! = formal cartel proof); σ=0 = vacuously Ok; high-variance honest
//! traffic passes; the bound is monotone-decreasing in Σ.

use evaporchain_tur_liveness::{
    mean, relative_variance_fixed, variance, tur_bound_fixed,
    tur_check, Verdict, FIXED_POINT_SCALE,
};

// ── Cartel and honest sample windows ─────────────────────────────────────
/// All three cartel validators block-produce in lockstep: constant 10.
fn cartel_window() -> Vec<u64> { vec![10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10] }
/// Honest validator varies naturally around ~50 blocks/epoch.
fn honest_window() -> Vec<u64> { vec![40, 55, 48, 63, 50, 44, 58, 51, 47, 62, 53, 46, 57, 49, 52, 60] }

const SIGMA: u64 = 200; // typical entropy production over 16-epoch window

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn cartel_constant_production_flagged_as_violation() {
    // Constant window → relative_variance = 0 → below any finite bound
    // → TUR violation. This is the formal cartel proof.
    let verdict = tur_check(&cartel_window(), SIGMA);
    assert!(matches!(verdict, Verdict::Violation { .. }),
        "constant block production must be flagged: {:?}", verdict);
}

#[test]
fn honest_variable_production_passes() {
    // Naturally varying production satisfies TUR at typical sigma.
    let verdict = tur_check(&honest_window(), SIGMA);
    assert!(matches!(verdict, Verdict::Ok { .. }),
        "honest variable production must pass: {:?}", verdict);
}

#[test]
fn violation_carries_observed_and_bound() {
    // Structured error: the violation reports both the observed
    // relative variance and the TUR bound so operators know by how
    // much the cartel was below the threshold.
    let verdict = tur_check(&cartel_window(), SIGMA);
    match verdict {
        Verdict::Violation { observed, bound } => {
            assert_eq!(observed, 0, "constant window → relative_variance = 0");
            assert!(bound > 0, "TUR bound must be positive");
        }
        other => panic!("expected Violation, got {:?}", other),
    }
}

#[test]
fn zero_sigma_vacuously_ok_even_for_cartel() {
    // Without an entropy accounting (σ=0), the bound is +∞ and no
    // liveness assertion can be made. Cartel passes vacuously.
    let verdict = tur_check(&cartel_window(), 0);
    match verdict {
        Verdict::Ok { observed: 0, bound } => assert_eq!(bound, u128::MAX),
        other => panic!("expected Ok with MAX bound at σ=0, got {:?}", other),
    }
}

#[test]
fn bound_monotone_decreasing_in_sigma() {
    // Higher entropy production tightens the TUR bound (makes it
    // harder for a cartel to hide as "natural noise").
    let loose = tur_bound_fixed(50);
    let tight = tur_bound_fixed(200);
    assert!(loose > tight,
        "bound must shrink as Σ grows: {loose} should be > {tight}");
}

#[test]
fn tur_bound_exact_at_power_of_two_sigma() {
    // 2 / 2 = 1 → FIXED_POINT_SCALE; 2 / 4 = 0.5 → FIXED_POINT_SCALE/2.
    assert_eq!(tur_bound_fixed(2), FIXED_POINT_SCALE);
    assert_eq!(tur_bound_fixed(4), FIXED_POINT_SCALE / 2);
}

#[test]
fn stats_mean_variance_exact_on_known_sequence() {
    // {1, 2, 3, 4, 5}: mean=3, variance=(4+1+0+1+4)/5=2.
    assert_eq!(mean(&[1, 2, 3, 4, 5]), 3);
    assert_eq!(variance(&[1, 2, 3, 4, 5]), 2);
}

#[test]
fn relative_variance_zero_for_constants() {
    // Constants have zero variance → relative_variance = 0.
    assert_eq!(relative_variance_fixed(&[7, 7, 7, 7, 7]), 0);
}

#[test]
fn relative_variance_max_for_zero_mean() {
    // All-zero samples → mean = 0 → ratio undefined → u128::MAX.
    assert_eq!(relative_variance_fixed(&[0, 0, 0]), u128::MAX);
}

#[test]
fn empty_window_safe() {
    // Empty sample windows must not panic.
    assert_eq!(mean(&[]), 0);
    assert_eq!(variance(&[]), 0);
    // Empty → mean=0 → u128::MAX → passes vacuously even at finite σ.
    assert!(matches!(tur_check(&[], SIGMA), Verdict::Ok { .. }));
}

#[test]
fn single_sample_zero_variance() {
    // Single-sample window → variance = 0 → relative_variance = 0.
    assert_eq!(variance(&[42]), 0);
    assert_eq!(relative_variance_fixed(&[42]), 0);
}

#[test]
fn cartel_detection_full_arc() {
    // Full arc: three cartel validators and one honest validator.
    // Monitor reads 16-epoch windows. Cartels flagged; honest cleared.
    let cartel_a = tur_check(&cartel_window(), SIGMA);
    let cartel_b = tur_check(&cartel_window(), SIGMA);
    let cartel_c = tur_check(&cartel_window(), SIGMA);
    let honest   = tur_check(&honest_window(), SIGMA);

    assert!(matches!(cartel_a, Verdict::Violation { .. }), "cartel A must be flagged");
    assert!(matches!(cartel_b, Verdict::Violation { .. }), "cartel B must be flagged");
    assert!(matches!(cartel_c, Verdict::Violation { .. }), "cartel C must be flagged");
    assert!(matches!(honest,   Verdict::Ok { .. }),        "honest must be cleared");

    // Structured check: all three cartel violations carry observed=0.
    for v in [cartel_a, cartel_b, cartel_c] {
        assert!(matches!(v, Verdict::Violation { observed: 0, .. }),
            "cartel violation must report zero relative variance");
    }
}

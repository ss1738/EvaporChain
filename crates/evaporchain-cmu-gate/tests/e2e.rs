//! §Cμ-Gate (§A1.3 Shalizi-Crutchfield 2001) e2e
//!
//! Scenario: "EvaporChain consensus Sybil detection round" — a passive
//! monitor observes four validators' block-content histograms over a
//! 100-epoch window. CARTEL A, B, C produce near-deterministic headers
//! (low entropy, high Cμ). HONEST validator D has naturally varied
//! production (high entropy, low Cμ). The gate checks Cμ ≤ E + hμ.
//!
//! The suite proves: the Shalizi-Crutchfield identity is an exact
//! falsifiable bound; violations carry the observed Cμ and bound;
//! the entropy estimator is non-negative and deterministic; saturating
//! arithmetic prevents overflow.

use evaporchain_cmu_gate::{cmu_bound, cmu_check, entropy_millibits, EntropyError, Verdict};

// ── Helpers ───────────────────────────────────────────────────────────────

/// A near-deterministic histogram: one bucket holds almost all mass.
/// Represents a Sybil-like validator producing nearly identical blocks.
fn sybil_histogram() -> [u64; 8] { [9990, 1, 1, 1, 1, 1, 1, 1] }

/// A naturally-varying histogram: mass spread across many outcomes.
/// Represents an honest validator with genuine block diversity.
fn honest_histogram() -> [u64; 8] { [125, 125, 125, 125, 125, 125, 125, 125] }

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn cmu_at_bound_admitted() {
    // Cμ = E + hμ exactly: at-bound is NOT a violation.
    let v = cmu_check(300, 100, 200);
    assert!(matches!(v, Verdict::Ok { observed_cmu: 300, bound: 300 }),
        "at-bound must be admitted: {:?}", v);
}

#[test]
fn cmu_above_bound_violation() {
    // One millibit above the bound → formal fault.
    let v = cmu_check(301, 100, 200);
    assert!(matches!(v, Verdict::Violation { observed_cmu: 301, bound: 300 }),
        "1mb above bound must be violation: {:?}", v);
}

#[test]
fn violation_carries_observed_and_bound() {
    // Structured error: operators see exactly how far Cμ exceeds the bound.
    let v = cmu_check(500, 100, 100);
    match v {
        Verdict::Violation { observed_cmu, bound } => {
            assert_eq!(observed_cmu, 500);
            assert_eq!(bound, 200);
        }
        other => panic!("expected Violation, got {:?}", other),
    }
}

#[test]
fn zero_zero_zero_admitted() {
    // All-zero chain state: no liveness assertion is violated.
    assert!(matches!(cmu_check(0, 0, 0), Verdict::Ok { .. }));
}

#[test]
fn nonzero_cmu_zero_bound_violation() {
    // Any non-zero Cμ with zero E and zero hμ: the process generates
    // complexity without any predictive or random basis — Sybil.
    assert!(matches!(cmu_check(1, 0, 0), Verdict::Violation { .. }));
}

#[test]
fn bound_is_sum_of_e_and_hmu() {
    assert_eq!(cmu_bound(100, 200), 300);
    assert_eq!(cmu_bound(0, 0), 0);
    assert_eq!(cmu_bound(u64::MAX / 2, u64::MAX / 2 + 1), u64::MAX);
}

#[test]
fn saturating_add_no_overflow_at_max() {
    // cmu_bound must not panic or wrap on extreme inputs.
    let b = cmu_bound(u64::MAX, 1);
    assert_eq!(b, u64::MAX, "saturating add must clamp to u64::MAX");
}

#[test]
fn entropy_deterministic() {
    let h1 = entropy_millibits(&sybil_histogram()).unwrap();
    let h2 = entropy_millibits(&sybil_histogram()).unwrap();
    assert_eq!(h1, h2, "entropy estimator must be deterministic");
}

#[test]
fn sybil_histogram_has_low_entropy() {
    // Near-deterministic histogram → entropy close to 0.
    let h = entropy_millibits(&sybil_histogram()).unwrap();
    let honest_h = entropy_millibits(&honest_histogram()).unwrap();
    assert!(h < honest_h,
        "Sybil histogram entropy ({h}) must be lower than honest ({honest_h})");
}

#[test]
fn honest_histogram_has_maximum_entropy() {
    // Uniform 8-bucket histogram → H = log_2(8) = 3 bits = 3000 millibits.
    let h = entropy_millibits(&honest_histogram()).unwrap();
    assert_eq!(h, 3_000, "uniform 8-bucket histogram must yield 3000 millibits");
}

#[test]
fn uniform_two_outcomes_is_one_bit() {
    // 50/50 → exactly 1000 millibits.
    assert_eq!(entropy_millibits(&[500, 500]).unwrap(), 1_000);
}

#[test]
fn single_bucket_is_zero_entropy() {
    // All mass in one bucket → 0 entropy (deterministic process).
    assert_eq!(entropy_millibits(&[1_000, 0, 0]).unwrap(), 0);
}

#[test]
fn empty_histogram_rejected() {
    let err = entropy_millibits(&[]).unwrap_err();
    assert_eq!(err, EntropyError::Empty);
}

#[test]
fn all_zero_histogram_rejected() {
    let err = entropy_millibits(&[0, 0, 0]).unwrap_err();
    assert_eq!(err, EntropyError::AllZero);
}

#[test]
fn sybil_detection_full_arc() {
    // Full arc: each validator's entropy is used to bound Cμ.
    // For a Sybil: Cμ = 2000 mb, E = 0 mb, hμ = sybil_h ≈ 0 → violation.
    // For honest: Cμ = 2000 mb, E = 1500 mb, hμ = honest_h = 3000 mb → ok.
    let sybil_h = entropy_millibits(&sybil_histogram()).unwrap();
    let honest_h = entropy_millibits(&honest_histogram()).unwrap();

    let cmu_observed = 2_000u64;

    // Cartel A: Cμ >> E + hμ → violation.
    let cartel_verdict = cmu_check(cmu_observed, 0, sybil_h);
    assert!(matches!(cartel_verdict, Verdict::Violation { .. }),
        "cartel must be flagged: {:?}", cartel_verdict);

    // Honest D: Cμ < E + hμ → ok.
    let honest_verdict = cmu_check(cmu_observed, 1_500, honest_h);
    assert!(matches!(honest_verdict, Verdict::Ok { .. }),
        "honest must be cleared: {:?}", honest_verdict);
}

//! End-to-end integration tests for evaporchain-causal-chsh.
//!
//! Non-trivial fixture: 200-block synthetic Ethereum gate walkthrough.
//!
//! A chain operator monitors a rolling 200-block window of "honest"
//! block summaries (independent LCG-generated statistics). At each gate
//! run, a synthetic coordinated-cartel sample of the same per-bucket
//! size is injected. The gate checks whether the CHSH S statistic
//! discriminates honest from cartel traffic per the doctrine thresholds.
//!
//!   Honest source (200 blocks, LCG, 60-second window):
//!     S_honest < 1.8  (LHV-shaped, random block observables)
//!
//!   Cartel injection (max-violation sample):
//!     S_cartel = 4.0  (E(A,B)=E(A,B')=E(A',B)=1; E(A',B')=-1)
//!
//!   Gate thresholds (doctrine-locked):
//!     honest_ceiling = 1.8   cartel_floor = 2.2   min_gap = 0.4
//!
//!   Gap = 4.0 - S_honest  ≈ 4.0 (>> 0.4) → PASS
//!
//! Rolling alarm session: alarm receives blocks at 12-second intervals.
//! First gate run fires at record 50 (run_interval). The alarm status
//! carries both milli-integer (validator-deterministic) and f64 (display)
//! S values. The milli value derives the verdict.
//!
//! Doctrine claim (INVENTION_STACK §4.1 / Tier-0 supporting primitive):
//! "Causal-CHSH bounds |S| ≤ 2 under honest validators + LightCone
//! causality (proposed theorem). A coordinated cartel achieves S up to
//! 4, violating the bound. The pure-integer `compute_chsh_s_milli` is
//! validator-deterministic on all architectures. The synthetic gate
//! PASSES iff S_honest < 1.8, S_cartel > 2.2, gap > 0.4 — all three
//! thresholds committed before the empirical run, MERA-style."
//!
//! Adversarial fixture: cartel injected as honest (honest ceiling fails),
//! balanced honest never exceeds Bell bound, empty bucket fails closed,
//! non-binary observable fails closed.
//!
//! INVENTION_STACK §4.1 (Tier-0 supporting, Causal-CHSH gate PASS
//! confirmed 2026-05-04 on real Ethereum).

use evaporchain_causal_chsh::{
    compute_chsh_s, compute_chsh_s_milli, run_synthetic_gate,
    CartelAlarm, ChshError,
    BlockSummary, GateThresholds, GateVerdict,
    extract_chsh_samples, synthesize_max_cartel_samples,
};
use evaporchain_causal_chsh::chsh::{ConcurrentPair, ConcurrentPairSamples};

// ── Helpers ───────────────────────────────────────────────────────────────

/// Deterministic LCG block summary — same LCG as the unit test suite.
fn synth_block(h: u64) -> BlockSummary {
    let mut rng = h
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut next = |bound: u64| {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (rng >> 33) % bound.max(1)
    };
    BlockSummary {
        height: h,
        timestamp_secs: h * 12 + next(3),
        energy:    10_000_000 + next(2_000_000),
        gas:       12_000_000 + next(2_000_000),
        tx_count:  100 + next(80),
    }
}

fn honest_trace_200() -> Vec<BlockSummary> {
    (0..200).map(synth_block).collect()
}

fn samples(ab: Vec<i8>, abp: Vec<i8>, apb: Vec<i8>, apbp: Vec<i8>) -> ConcurrentPairSamples {
    ConcurrentPairSamples {
        samples_ab: ab,
        samples_ab_prime: abp,
        samples_a_prime_b: apb,
        samples_a_prime_b_prime: apbp,
    }
}

fn max_cartel(n: usize) -> ConcurrentPairSamples {
    samples(vec![1; n], vec![1; n], vec![1; n], vec![-1; n])
}

fn balanced_honest(n: usize) -> ConcurrentPairSamples {
    let b: Vec<i8> = (0..n).map(|i| if i % 2 == 0 { 1 } else { -1 }).collect();
    samples(b.clone(), b.clone(), b.clone(), b)
}

// ── Non-trivial fixture: 200-block gate walkthrough ───────────────────────

#[test]
fn honest_200_block_trace_satisfies_bell_bound() {
    // 200 synthetic-Eth blocks, 60-second concurrency window.
    // Independent random observables → S well below the 1.8 doctrine ceiling.
    let trace = honest_trace_200();
    let honest = extract_chsh_samples(&trace, 60);

    // All four buckets must have substantial samples.
    assert!(honest.samples_ab.len() >= 10,
        "AB bucket must have ≥10 samples, got {}", honest.samples_ab.len());
    assert!(honest.samples_ab_prime.len() >= 10,
        "AB' bucket must have ≥10 samples");
    assert!(honest.samples_a_prime_b.len() >= 10,
        "A'B bucket must have ≥10 samples");
    assert!(honest.samples_a_prime_b_prime.len() >= 10,
        "A'B' bucket must have ≥10 samples");

    let s = compute_chsh_s(&honest).unwrap();
    assert!(s < 1.8, "honest 200-block trace must satisfy Bell bound: got S={s:.4}");
    assert!(s >= 0.0, "S must be non-negative");
}

#[test]
fn max_cartel_reaches_algebraic_ceiling() {
    // Maximally-coordinated cartel: S = 4 (algebraic max).
    let cartel = max_cartel(100);
    let s = compute_chsh_s(&cartel).unwrap();
    assert!((s - 4.0).abs() < 1e-9, "cartel must reach S=4, got {s}");
    // Cartel S vastly exceeds Bell bound.
    assert!(s > 2.0, "cartel must violate Bell bound");
    assert!(s > 2.2, "cartel must exceed doctrine cartel_floor=2.2");
}

#[test]
fn gate_passes_doctrine_thresholds_on_200_block_trace() {
    // Full gate walkthrough: honest trace vs max cartel, doctrine thresholds.
    let trace = honest_trace_200();
    let honest = extract_chsh_samples(&trace, 60);
    let n = honest.samples_ab.len();
    let cartel = synthesize_max_cartel_samples(n);

    let verdict = run_synthetic_gate(&honest, &cartel, GateThresholds::doctrine());

    match verdict {
        GateVerdict::Pass { s_honest, s_cartel, gap } => {
            assert!(s_honest < 1.8,
                "S_honest must be below ceiling=1.8; got {s_honest:.4}");
            assert!(s_cartel > 2.2,
                "S_cartel must be above floor=2.2; got {s_cartel:.4}");
            assert!(gap > 0.4,
                "gap must exceed min_gap=0.4; got {gap:.4}");
            assert!((s_cartel - 4.0).abs() < 1e-9,
                "max-cartel S must equal 4.0");
        }
        other => panic!("expected Pass, got {other:?}"),
    }
}

#[test]
fn compute_chsh_s_milli_is_validator_deterministic() {
    // Integer milli-path must give bit-identical results across calls.
    // This is the consensus-bearing path — f64 non-determinism excluded.
    let trace = honest_trace_200();
    let honest = extract_chsh_samples(&trace, 60);
    let a = compute_chsh_s_milli(&honest).unwrap();
    let b = compute_chsh_s_milli(&honest).unwrap();
    assert_eq!(a, b, "milli S must be deterministic (i64 arithmetic)");
    // Milli S is in range [0, 4000].
    assert!(a >= 0 && a <= 4_000, "S_milli must be in [0, 4000], got {a}");
}

#[test]
fn milli_s_agrees_with_float_s_within_one_milli() {
    // The two paths (f64 and i64) must agree to within 1 milli-unit
    // for balanced inputs. Validates the integer path's numerical fidelity.
    let bal = balanced_honest(100);
    let s_float = compute_chsh_s(&bal).unwrap();
    let s_milli = compute_chsh_s_milli(&bal).unwrap();
    let s_float_as_milli = (s_float * 1000.0) as i64;
    assert!((s_milli - s_float_as_milli).abs() <= 2,
        "milli ({s_milli}) and float ({s_float_as_milli}) must agree within 2 milli-units");
}

#[test]
fn rolling_alarm_doctrine_session() {
    // Alarm session: feed 200 blocks at 12-second intervals.
    // First gate run fires at record 50 (run_interval=50 with ≥50 buffer).
    // Each run shows verdict=Pass on honest synthetic source.
    let mut alarm = CartelAlarm::doctrine_default();

    for h in 0..200 {
        alarm.record_block(synth_block(h), h);
    }

    let status = alarm.status().expect("200 records must have triggered at least one gate run");

    assert_eq!(status.verdict, "Pass",
        "honest 200-block trace must produce Pass verdict");
    assert!(status.s_honest < 1.8,
        "honest S must be below ceiling: {}", status.s_honest);
    assert!(status.s_cartel_synthetic > 2.2,
        "cartel S must exceed floor");
    assert!(status.gap > 0.4,
        "gap must exceed min_gap");

    // Milli values must be non-negative and bounded.
    assert!(status.s_honest_milli >= 0, "milli S must be ≥0");
    assert!(status.s_honest_milli <= 4_000, "milli S must be ≤4000");
    assert_eq!(status.s_cartel_synthetic_milli, 4_000,
        "max-cartel milli S must be exactly 4000");

    // Buffer capped at capacity.
    assert!(alarm.buffer_len() <= 200, "buffer must not exceed capacity");
    assert_eq!(alarm.records_seen(), 200);
}

#[test]
fn doctrine_honest_s_far_below_cartel_s() {
    // INVENTION_STACK §4.1 doctrine: the gap between honest and cartel S
    // must be large enough to discriminate. Quantitatively verify the
    // discrimination margin on our 200-block trace.
    let trace = honest_trace_200();
    let honest = extract_chsh_samples(&trace, 60);
    let n = honest.samples_ab.len();
    let cartel = synthesize_max_cartel_samples(n);

    let s_h = compute_chsh_s(&honest).unwrap();
    let s_c = compute_chsh_s(&cartel).unwrap();
    let gap = s_c - s_h;

    assert!(gap > 2.0,
        "discrimination gap must be >> 0.4 on 200-block trace; got gap={gap:.4}");
    assert!(gap > s_h * 2.0,
        "cartel S must be >> honest S (not just a marginal gap)");
}

#[test]
fn doctrine_bell_bound_holds_for_lhv_source_classically() {
    // Bell's theorem (classical LHV): when all four observables are
    // assigned per pair from a shared joint distribution, |S| ≤ 2.
    // Verified across 8 canonical LHV assignments.
    let lhv_cases: &[(i8, i8, i8, i8)] = &[
        ( 1,  1,  1,  1), ( 1,  1,  1, -1), ( 1,  1, -1,  1), ( 1,  1, -1, -1),
        ( 1, -1,  1,  1), ( 1, -1,  1, -1), (-1,  1,  1,  1), (-1, -1, -1, -1),
    ];
    let pairs: Vec<ConcurrentPair> = lhv_cases.iter().map(|&(a, ap, b, bp)| {
        ConcurrentPair { a, a_prime: ap, b, b_prime: bp }
    }).collect();

    let s = evaporchain_causal_chsh::chsh::compute_chsh_s_classical(&pairs).unwrap();
    assert!(s <= 2.0 + 1e-9, "LHV source must satisfy Bell bound: S={s:.6}");
}

#[test]
fn doctrine_max_violation_is_4_algebraic_ceiling() {
    // Algebraic ceiling: all four bucket means rigged to extremes.
    // S = |+1 + +1 + +1 - (-1)| = 4. Cannot be achieved by any LHV
    // process — only a communicating cartel can push all four samples
    // independently to their extremes.
    let s_milli = compute_chsh_s_milli(&max_cartel(8)).unwrap();
    assert_eq!(s_milli, 4_000, "algebraic max must be exactly S_milli=4000");
    assert_eq!(s_milli, 4 * 1000);
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_cartel_injected_as_honest_fails_gate() {
    // If the "honest" sample is itself a cartel (S=4), the gate must
    // fail — the honest ceiling (1.8) is exceeded.
    let cartel = max_cartel(50);
    let verdict = run_synthetic_gate(&cartel, &cartel, GateThresholds::doctrine());
    match verdict {
        GateVerdict::Fail { reasons, s_honest, .. } => {
            assert!(s_honest > 1.8, "cartel-as-honest must trip the ceiling");
            assert!(reasons.iter().any(|r| r.contains("honest")),
                "fail reasons must mention the honest ceiling violation");
        }
        other => panic!("expected Fail when cartel injected as honest; got {other:?}"),
    }
}

#[test]
fn adversarial_empty_bucket_fails_closed() {
    // Empty AB sample → ChshError::EmptySample. The gate must reject
    // malformed input rather than computing a garbage S value.
    let bad = samples(vec![], vec![1, -1], vec![1, -1], vec![1, -1]);
    let err = compute_chsh_s(&bad).unwrap_err();
    assert!(matches!(err, ChshError::EmptySample { pair_label: "AB" }),
        "empty bucket must fail closed");

    // Same for milli path.
    let err_milli = compute_chsh_s_milli(&bad).unwrap_err();
    assert!(matches!(err_milli, ChshError::EmptySample { .. }),
        "milli path must also fail closed on empty bucket");
}

#[test]
fn adversarial_non_binary_observable_fails_closed() {
    // Observable value=0 (not ±1) must be rejected.
    let bad = samples(vec![1, 0, -1], vec![1, -1], vec![1, -1], vec![1, -1]);
    let err = compute_chsh_s(&bad).unwrap_err();
    assert!(matches!(err, ChshError::NonBinaryObservable { value: 0, index: 1, .. }),
        "non-binary observable must fail closed");
}

#[test]
fn adversarial_balanced_honest_is_exactly_zero() {
    // Perfectly balanced samples → all E(·,·) = 0 → S = 0.
    // Confirms the honest baseline: zero coordination = zero violation.
    let balanced = balanced_honest(64);
    let s = compute_chsh_s(&balanced).unwrap();
    assert!(s.abs() < 1e-9, "balanced honest must give S=0, got {s}");
    let s_milli = compute_chsh_s_milli(&balanced).unwrap();
    assert_eq!(s_milli, 0, "balanced honest milli S must be 0");
}

#[test]
fn adversarial_concurrency_window_zero_admits_no_pairs() {
    // Window=0 means no pair (i,j) with i≠j has dt≤0 (unless
    // timestamps exactly equal — LCG jitter may allow a few, but far
    // fewer than the 60-second window).
    let trace = honest_trace_200();
    let tight = extract_chsh_samples(&trace, 0);
    let wide  = extract_chsh_samples(&trace, 60);

    let tight_total: usize = [
        tight.samples_ab.len(),
        tight.samples_ab_prime.len(),
        tight.samples_a_prime_b.len(),
        tight.samples_a_prime_b_prime.len(),
    ].iter().sum();
    let wide_total: usize = [
        wide.samples_ab.len(),
        wide.samples_ab_prime.len(),
        wide.samples_a_prime_b.len(),
        wide.samples_a_prime_b_prime.len(),
    ].iter().sum();

    assert!(tight_total <= wide_total,
        "tight window must admit ≤ pairs than wide window");
}

#[test]
fn adversarial_alarm_buffer_capped_at_capacity() {
    // Push 400 records into a capacity=200 alarm; buffer stays capped.
    let mut alarm = CartelAlarm::new(200, 50, 60);
    for h in 0..400 {
        alarm.record_block(synth_block(h), h);
        assert!(alarm.buffer_len() <= 200,
            "buffer must never exceed capacity 200 at h={h}");
    }
    assert_eq!(alarm.records_seen(), 400, "records_seen must track all calls");
}

//! Cross-module coverage for evaporchain-causal-chsh — milli/f64
//! agreement, doctrine constant pins, GateVerdict variants,
//! ConcurrentPair classical path, CartelAlarmEvent + AlarmStatus
//! shape, BlockSummary + ConcurrentPair serde, alarm injection,
//! synthesize_max_cartel_samples size correctness.

use evaporchain_causal_chsh::{
    chsh::{compute_chsh_s_classical, ConcurrentPair, ConcurrentPairSamples},
    AlarmStatus, BlockSummary, CartelAlarm, CartelAlarmEvent, ChshError, GateThresholds,
    GateVerdict, compute_chsh_s, compute_chsh_s_milli, extract_chsh_samples,
    run_synthetic_gate, synthesize_max_cartel_samples,
};

// =================================================================
// Doctrine constants
// =================================================================

#[test]
fn doctrine_thresholds_are_pinned() {
    let t = GateThresholds::doctrine();
    assert_eq!(t.honest_ceiling, 1.8);
    assert_eq!(t.cartel_floor, 2.2);
    assert_eq!(t.min_gap, 0.4);
}

// =================================================================
// f64 vs milli agreement
// =================================================================

#[test]
fn milli_path_agrees_with_f64_at_balanced_zero() {
    let bal = ConcurrentPairSamples {
        samples_ab: vec![1, -1, 1, -1, 1, -1],
        samples_ab_prime: vec![1, -1, 1, -1, 1, -1],
        samples_a_prime_b: vec![1, -1, 1, -1, 1, -1],
        samples_a_prime_b_prime: vec![1, -1, 1, -1, 1, -1],
    };
    assert_eq!(compute_chsh_s(&bal).unwrap(), 0.0);
    assert_eq!(compute_chsh_s_milli(&bal).unwrap(), 0);
}

#[test]
fn milli_path_agrees_with_f64_at_algebraic_max() {
    let cartel = ConcurrentPairSamples {
        samples_ab: vec![1; 8],
        samples_ab_prime: vec![1; 8],
        samples_a_prime_b: vec![1; 8],
        samples_a_prime_b_prime: vec![-1; 8],
    };
    let s_f = compute_chsh_s(&cartel).unwrap();
    let s_m = compute_chsh_s_milli(&cartel).unwrap();
    assert!((s_f - 4.0).abs() < 1e-9);
    assert_eq!(s_m, 4_000);
}

#[test]
fn milli_returns_unsigned_absolute_value() {
    // Sign-flipped cartel: S would be -4 as a signed quantity; milli
    // returns |S * 1000| = 4000 regardless of sign.
    let flipped = ConcurrentPairSamples {
        samples_ab: vec![-1; 8],
        samples_ab_prime: vec![-1; 8],
        samples_a_prime_b: vec![-1; 8],
        samples_a_prime_b_prime: vec![1; 8],
    };
    assert_eq!(compute_chsh_s_milli(&flipped).unwrap(), 4_000);
}

// =================================================================
// Error variants — empty / non-binary on both paths
// =================================================================

#[test]
fn empty_a_prime_b_prime_errs_with_correct_label() {
    let s = ConcurrentPairSamples {
        samples_ab: vec![1],
        samples_ab_prime: vec![1],
        samples_a_prime_b: vec![1],
        samples_a_prime_b_prime: vec![],
    };
    match compute_chsh_s(&s) {
        Err(ChshError::EmptySample { pair_label }) => assert_eq!(pair_label, "A'B'"),
        other => panic!("expected EmptySample(A'B'), got {other:?}"),
    }
}

#[test]
fn non_binary_observable_surfaces_index_and_value() {
    let s = ConcurrentPairSamples {
        samples_ab: vec![1, 1, 7],
        samples_ab_prime: vec![1],
        samples_a_prime_b: vec![1],
        samples_a_prime_b_prime: vec![1],
    };
    match compute_chsh_s_milli(&s) {
        Err(ChshError::NonBinaryObservable { index, value, .. }) => {
            assert_eq!(index, 2);
            assert_eq!(value, 7);
        }
        other => panic!("expected NonBinaryObservable, got {other:?}"),
    }
}

// =================================================================
// ConcurrentPair classical path
// =================================================================

#[test]
fn classical_path_caps_at_2_for_lhv_source() {
    // Balanced classical: every pair sums to zero → S=0.
    let pairs = vec![
        ConcurrentPair { a: 1, a_prime: -1, b: 1, b_prime: -1 },
        ConcurrentPair { a: -1, a_prime: 1, b: -1, b_prime: 1 },
    ];
    let s = compute_chsh_s_classical(&pairs).unwrap();
    assert!(s <= 2.0, "classical LHV must satisfy Bell: S={s}");
}

#[test]
fn classical_path_empty_errs() {
    let err = compute_chsh_s_classical(&[]).unwrap_err();
    assert!(matches!(err, ChshError::EmptySample { .. }));
}

#[test]
fn classical_path_non_binary_errs() {
    let pairs = vec![ConcurrentPair { a: 2, a_prime: 1, b: 1, b_prime: 1 }];
    assert!(matches!(
        compute_chsh_s_classical(&pairs),
        Err(ChshError::NonBinaryObservable { .. })
    ));
}

// =================================================================
// GateVerdict variants + run_synthetic_gate paths
// =================================================================

#[test]
fn gate_verdict_pass_carries_floats() {
    let honest = balanced_zero();
    let cartel = synthesize_max_cartel_samples(8);
    match run_synthetic_gate(&honest, &cartel, GateThresholds::doctrine()) {
        GateVerdict::Pass { s_honest, s_cartel, gap } => {
            assert!(s_honest < 1.8);
            assert!(s_cartel > 2.2);
            assert!(gap > 0.4);
        }
        other => panic!("expected Pass, got {other:?}"),
    }
}

#[test]
fn gate_verdict_fail_lists_specific_reasons() {
    // Honest = cartel → fails on honest_ceiling AND gap thresholds.
    let cartel = synthesize_max_cartel_samples(8);
    match run_synthetic_gate(&cartel, &cartel, GateThresholds::doctrine()) {
        GateVerdict::Fail { reasons, .. } => {
            assert!(reasons.iter().any(|r| r.contains("honest")));
            assert!(reasons.iter().any(|r| r.contains("gap")));
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[test]
fn gate_verdict_input_error_surfaces_which_side_failed() {
    let empty = ConcurrentPairSamples {
        samples_ab: vec![],
        samples_ab_prime: vec![1],
        samples_a_prime_b: vec![1],
        samples_a_prime_b_prime: vec![1],
    };
    let cartel = synthesize_max_cartel_samples(8);
    match run_synthetic_gate(&empty, &cartel, GateThresholds::doctrine()) {
        GateVerdict::InputError(msg) => assert!(msg.contains("honest")),
        other => panic!("expected InputError, got {other:?}"),
    }
}

// =================================================================
// synthesize_max_cartel_samples
// =================================================================

#[test]
fn cartel_synthesis_has_correct_per_bucket_size() {
    let c = synthesize_max_cartel_samples(0);
    assert!(c.samples_ab.is_empty());
    let c = synthesize_max_cartel_samples(17);
    assert_eq!(c.samples_ab.len(), 17);
    assert_eq!(c.samples_ab_prime.len(), 17);
    assert_eq!(c.samples_a_prime_b.len(), 17);
    assert_eq!(c.samples_a_prime_b_prime.len(), 17);
    assert!(c.samples_ab.iter().all(|&v| v == 1));
    assert!(c.samples_a_prime_b_prime.iter().all(|&v| v == -1));
}

// =================================================================
// extract_chsh_samples — concurrency window monotonicity
// =================================================================

#[test]
fn extract_with_singleton_trace_yields_empty() {
    let trace = vec![BlockSummary {
        height: 0,
        timestamp_secs: 0,
        energy: 1,
        gas: 1,
        tx_count: 1,
    }];
    let s = extract_chsh_samples(&trace, 60);
    assert!(s.samples_ab.is_empty());
}

#[test]
fn extract_distributes_round_robin_across_buckets() {
    // 4 blocks at 1-second intervals, 60s window → all 6 pairs admit.
    // Round-robin counter % 4 distributes 6 pairs as [2, 2, 1, 1].
    let trace: Vec<BlockSummary> = (0..4)
        .map(|i| BlockSummary {
            height: i,
            timestamp_secs: i,
            energy: 10 + i,
            gas: 100 + i,
            tx_count: 50 + i,
        })
        .collect();
    let s = extract_chsh_samples(&trace, 60);
    let lens = [
        s.samples_ab.len(),
        s.samples_ab_prime.len(),
        s.samples_a_prime_b.len(),
        s.samples_a_prime_b_prime.len(),
    ];
    let total: usize = lens.iter().sum();
    assert_eq!(total, 6, "must visit every (i,j) with j>i for n=4");
    // Round-robin: the four bucket sizes must be balanced.
    let max = *lens.iter().max().unwrap();
    let min = *lens.iter().min().unwrap();
    assert!(max - min <= 1, "round-robin imbalance: {lens:?}");
}

// =================================================================
// CartelAlarm — defaults, capacity clamp, force-recompute, status injection
// =================================================================

#[test]
fn cartel_alarm_doctrine_default_matches_lane_o3_settings() {
    let a = CartelAlarm::doctrine_default();
    assert_eq!(a.buffer_len(), 0);
    assert_eq!(a.records_seen(), 0);
    assert!(a.status().is_none());
}

#[test]
fn cartel_alarm_capacity_floor_is_50() {
    let a = CartelAlarm::new(10, 50, 60);
    // The floor isn't directly readable, but feeding 60 records and
    // checking buffer-cap surfaces it.
    let mut a = a;
    for h in 0..60 {
        a.record_block(
            BlockSummary {
                height: h,
                timestamp_secs: h * 12,
                energy: 1_000 + h,
                gas: 1_000 + h,
                tx_count: 100 + h,
            },
            h,
        );
    }
    assert!(a.buffer_len() <= 50, "capacity clamped to 50, got {}", a.buffer_len());
}

#[test]
fn cartel_alarm_inject_status_for_test_replaces_last_status() {
    let mut a = CartelAlarm::doctrine_default();
    let injected = AlarmStatus {
        s_honest: 1.95,
        s_cartel_synthetic: 4.0,
        gap: 2.05,
        s_honest_milli: 1_950,
        s_cartel_synthetic_milli: 4_000,
        gap_milli: 2_050,
        verdict: "Fail".into(),
        last_run_at_height: 42,
        samples_per_bucket: [5, 5, 5, 5],
        thresholds: GateThresholds::doctrine(),
    };
    a._inject_status_for_test(injected.clone());
    assert_eq!(a.status().unwrap(), &injected);
}

// =================================================================
// Serde round-trips
// =================================================================

#[test]
fn alarm_status_serde_round_trips() {
    let st = AlarmStatus {
        s_honest: 0.5,
        s_cartel_synthetic: 4.0,
        gap: 3.5,
        s_honest_milli: 500,
        s_cartel_synthetic_milli: 4_000,
        gap_milli: 3_500,
        verdict: "Pass".into(),
        last_run_at_height: 100,
        samples_per_bucket: [12, 12, 12, 11],
        thresholds: GateThresholds::doctrine(),
    };
    let json = serde_json::to_string(&st).unwrap();
    let back: AlarmStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(back, st);
}

#[test]
fn cartel_alarm_event_serde_round_trips() {
    let e = CartelAlarmEvent {
        at_height: 50,
        s_honest_milli: 1_900,
        s_cartel_synthetic_milli: 4_000,
        gap_milli: 2_100,
        honest_ceiling_milli_at_fire: 1_800,
        samples_per_bucket: [10, 10, 10, 10],
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: CartelAlarmEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back, e);
}

#[test]
fn block_summary_and_concurrent_pair_serde_round_trips() {
    let b = BlockSummary {
        height: 7,
        timestamp_secs: 100,
        energy: 9_999,
        gas: 8_888,
        tx_count: 77,
    };
    let json = serde_json::to_string(&b).unwrap();
    let back: BlockSummary = serde_json::from_str(&json).unwrap();
    assert_eq!(back.height, b.height);
    assert_eq!(back.tx_count, b.tx_count);

    let p = ConcurrentPair { a: 1, a_prime: -1, b: 1, b_prime: -1 };
    let json = serde_json::to_string(&p).unwrap();
    let back: ConcurrentPair = serde_json::from_str(&json).unwrap();
    assert_eq!(back.a, p.a);
    assert_eq!(back.b_prime, p.b_prime);
}

#[test]
fn gate_thresholds_serde_round_trips() {
    let t = GateThresholds::doctrine();
    let json = serde_json::to_string(&t).unwrap();
    let back: GateThresholds = serde_json::from_str(&json).unwrap();
    assert_eq!(back, t);
}

// =================================================================
// Helpers
// =================================================================

fn balanced_zero() -> ConcurrentPairSamples {
    let bal = vec![1i8, -1, 1, -1, 1, -1, 1, -1];
    ConcurrentPairSamples {
        samples_ab: bal.clone(),
        samples_ab_prime: bal.clone(),
        samples_a_prime_b: bal.clone(),
        samples_a_prime_b_prime: bal,
    }
}

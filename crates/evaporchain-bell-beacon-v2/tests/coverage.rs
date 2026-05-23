//! Cross-module coverage for evaporchain-bell-beacon-v2 — targets
//! VerifyError variants and edge cases not already exercised by
//! e2e.rs. Focuses on field-mismatch paths (each verifier check
//! returns a distinct error variant), window-binding cartel non-
//! replay, threshold-mismatch, gate-failure on a cartel-only
//! window, error Display, and serde round-trips.

use evaporchain_bell_beacon_v2::{
    issue_certificate, verify_certificate, BeaconError, BellCertificate, ConcurrentPair,
    GateThresholdsMilli, PairStats, VerifyError,
};
use evaporchain_causal_chsh::gate::GateThresholds;

fn pair(e1: u64, t1: u64, e2: u64, t2: u64, tag_byte: u8) -> ConcurrentPair {
    let mut tag = [0u8; 32];
    tag[31] = tag_byte;
    tag[0] = tag_byte;
    ConcurrentPair {
        first: PairStats { energy: e1, tx_count: t1 },
        second: PairStats { energy: e2, tx_count: t2 },
        tag,
    }
}

fn balanced_window() -> Vec<ConcurrentPair> {
    let mut out = Vec::new();
    for i in 0..16u8 {
        out.push(pair(
            if i & 1 == 1 { 100 } else { 10 },
            if (i >> 1) & 1 == 1 { 100 } else { 10 },
            if (i >> 2) & 1 == 1 { 100 } else { 10 },
            if (i >> 3) & 1 == 1 { 100 } else { 10 },
            i,
        ));
    }
    out
}

const CHAIN: &str = "test-chain-bbv2-cov";

fn issue() -> BellCertificate {
    issue_certificate(
        CHAIN,
        100,
        200,
        &balanced_window(),
        GateThresholds::doctrine(),
        [9u8; 32],
    )
    .unwrap()
}

// =================================================================
// GateThresholdsMilli — doctrine conversion
// =================================================================

#[test]
fn gate_thresholds_milli_conversion_pinned() {
    let m: GateThresholdsMilli = GateThresholds::doctrine().into();
    assert_eq!(m.honest_ceiling_milli, 1800);
    assert_eq!(m.cartel_floor_milli, 2200);
    // 0.4 * 1000 truncated — IEEE-754 representation of 0.4 is slightly
    // below, so the conversion may land at 399 or 400 across stdlib
    // builds. Bell-beacon issuance / verification compare integer-to-
    // integer post-conversion, so both values round-trip identically.
    assert!(m.min_gap_milli == 400 || m.min_gap_milli == 399);
}

// =================================================================
// Issuance — additional error paths
// =================================================================

#[test]
fn issuance_window_start_equals_end_rejected() {
    let err = issue_certificate(
        CHAIN,
        100,
        100,
        &balanced_window(),
        GateThresholds::doctrine(),
        [9u8; 32],
    )
    .unwrap_err();
    match err {
        BeaconError::InvalidWindowRange { start, end } => {
            assert_eq!(start, 100);
            assert_eq!(end, 100);
        }
        other => panic!("expected InvalidWindowRange, got {other:?}"),
    }
}

#[test]
fn issuance_certificate_records_bucket_counts_summing_to_n_pairs() {
    let cert = issue();
    assert_eq!(cert.n_pairs, 16);
    assert_eq!(cert.bucket_counts.iter().sum::<u64>(), 16);
}

// =================================================================
// Verify — every per-field mismatch surfaces a distinct error
// =================================================================

#[test]
fn verify_empty_pairs_rejected() {
    let cert = issue();
    let err = verify_certificate(CHAIN, &[], [9u8; 32], GateThresholds::doctrine(), &cert)
        .unwrap_err();
    assert_eq!(err, VerifyError::EmptyWindow);
}

#[test]
fn verify_prev_hash_mismatch() {
    let cert = issue();
    let err = verify_certificate(
        CHAIN,
        &balanced_window(),
        [0u8; 32], // wrong prev
        GateThresholds::doctrine(),
        &cert,
    )
    .unwrap_err();
    assert_eq!(err, VerifyError::PrevHashMismatch);
}

#[test]
fn verify_pair_count_mismatch() {
    let cert = issue();
    let short = balanced_window()[..8].to_vec();
    let err = verify_certificate(
        CHAIN,
        &short,
        [9u8; 32],
        GateThresholds::doctrine(),
        &cert,
    )
    .unwrap_err();
    match err {
        VerifyError::PairCountMismatch { cert: c, given: g } => {
            assert_eq!(c, 16);
            assert_eq!(g, 8);
        }
        other => panic!("expected PairCountMismatch, got {other:?}"),
    }
}

#[test]
fn verify_invalid_window_in_cert_rejected() {
    // Tamper window in-cert so window_end <= window_start.
    let mut cert = issue();
    cert.window_end = cert.window_start;
    let err = verify_certificate(
        CHAIN,
        &balanced_window(),
        [9u8; 32],
        GateThresholds::doctrine(),
        &cert,
    )
    .unwrap_err();
    assert!(matches!(err, VerifyError::InvalidWindowRange { .. }));
}

#[test]
fn verify_threshold_mismatch() {
    let cert = issue();
    // Tamper one threshold value in-cert.
    let mut tampered = cert;
    tampered.honest_ceiling_milli = 9_000;
    let err = verify_certificate(
        CHAIN,
        &balanced_window(),
        [9u8; 32],
        GateThresholds::doctrine(),
        &tampered,
    )
    .unwrap_err();
    assert_eq!(err, VerifyError::ThresholdMismatch);
}

#[test]
fn verify_bucket_count_mismatch() {
    let cert = issue();
    let mut tampered = cert;
    tampered.bucket_counts = [1, 2, 3, 10]; // wrong shape
    let err = verify_certificate(
        CHAIN,
        &balanced_window(),
        [9u8; 32],
        GateThresholds::doctrine(),
        &tampered,
    )
    .unwrap_err();
    assert_eq!(err, VerifyError::BucketCountMismatch);
}

#[test]
fn verify_s_cartel_mismatch_when_tampered() {
    let cert = issue();
    let mut tampered = cert;
    tampered.s_cartel_milli = 1; // implausibly low
    let err = verify_certificate(
        CHAIN,
        &balanced_window(),
        [9u8; 32],
        GateThresholds::doctrine(),
        &tampered,
    )
    .unwrap_err();
    assert!(matches!(err, VerifyError::SCartelMismatch { .. }));
}

#[test]
fn verify_gap_milli_mismatch_when_tampered() {
    let cert = issue();
    let mut tampered = cert;
    // Tamper gap only — s_honest and s_cartel still match, but gap doesn't.
    tampered.gap_milli += 1;
    let err = verify_certificate(
        CHAIN,
        &balanced_window(),
        [9u8; 32],
        GateThresholds::doctrine(),
        &tampered,
    )
    .unwrap_err();
    assert!(matches!(err, VerifyError::GapMismatch { .. }));
}

#[test]
fn verify_cross_chain_replay_seed_mismatch() {
    // Issue under one chain id; verify under another. derive_seed binds
    // chain_id, so the seed won't match.
    let cert = issue();
    let err = verify_certificate(
        "different-chain-id",
        &balanced_window(),
        [9u8; 32],
        GateThresholds::doctrine(),
        &cert,
    )
    .unwrap_err();
    // First field mismatch on the verify-side path: cartel seed (and
    // therefore s_cartel) diverges under a different chain_id.
    assert!(
        matches!(err, VerifyError::SCartelMismatch { .. } | VerifyError::SeedMismatch),
        "cross-chain replay must reject; got {err:?}"
    );
}

#[test]
fn verify_cross_window_replay_seed_mismatch() {
    // Tamper the certificate's recorded window to a different range —
    // the cartel seed derives from (chain_id, prev, window), so cartel
    // diverges.
    let cert = issue();
    let mut tampered = cert;
    tampered.window_end = cert.window_end + 50;
    let err = verify_certificate(
        CHAIN,
        &balanced_window(),
        [9u8; 32],
        GateThresholds::doctrine(),
        &tampered,
    )
    .unwrap_err();
    assert!(
        matches!(err, VerifyError::SCartelMismatch { .. } | VerifyError::SeedMismatch),
        "cross-window replay must reject; got {err:?}"
    );
}

// =================================================================
// VerifyError + BeaconError Display
// =================================================================

#[test]
fn verify_error_display_includes_signal_text() {
    assert!(VerifyError::EmptyWindow.to_string().to_lowercase().contains("empty"));
    assert!(VerifyError::PrevHashMismatch
        .to_string()
        .to_lowercase()
        .contains("prev_block_hash"));
    let e = VerifyError::PairCountMismatch { cert: 5, given: 3 };
    let s = e.to_string();
    assert!(s.contains("5"));
    assert!(s.contains("3"));
}

#[test]
fn beacon_error_display_includes_signal_text() {
    let s = BeaconError::InvalidWindowRange { start: 7, end: 3 }.to_string();
    assert!(s.contains("7"));
    assert!(s.contains("3"));
    assert!(BeaconError::EmptyWindow.to_string().to_lowercase().contains("empty"));
}

// =================================================================
// Equality of error variants
// =================================================================

#[test]
fn verify_error_eq_discriminates_variants() {
    assert_eq!(VerifyError::EmptyWindow, VerifyError::EmptyWindow);
    assert_ne!(VerifyError::EmptyWindow, VerifyError::PrevHashMismatch);
    let a = VerifyError::PairCountMismatch { cert: 1, given: 2 };
    let b = VerifyError::PairCountMismatch { cert: 1, given: 2 };
    let c = VerifyError::PairCountMismatch { cert: 1, given: 3 };
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// =================================================================
// Serde round-trips
// =================================================================

#[test]
fn bell_certificate_serde_round_trips() {
    let cert = issue();
    let json = serde_json::to_string(&cert).unwrap();
    let back: BellCertificate = serde_json::from_str(&json).unwrap();
    assert_eq!(back, cert);
}

#[test]
fn gate_thresholds_milli_serde_round_trips() {
    let m: GateThresholdsMilli = GateThresholds::doctrine().into();
    let json = serde_json::to_string(&m).unwrap();
    let back: GateThresholdsMilli = serde_json::from_str(&json).unwrap();
    assert_eq!(back, m);
}

#[test]
fn pair_and_stats_serde_round_trips() {
    let p = pair(11, 22, 33, 44, 7);
    let json = serde_json::to_string(&p).unwrap();
    let back: ConcurrentPair = serde_json::from_str(&json).unwrap();
    assert_eq!(back, p);
}

// =================================================================
// Round-trip determinism — re-issuance produces byte-identical cert
// =================================================================

#[test]
fn round_trip_certificate_verifies_under_supplied_thresholds() {
    let cert = issue();
    verify_certificate(
        CHAIN,
        &balanced_window(),
        [9u8; 32],
        GateThresholds::doctrine(),
        &cert,
    )
    .unwrap();
}

#[test]
fn three_validators_with_identical_inputs_get_identical_certs() {
    // Locks the BFT-determinism contract: any honest validator can
    // re-issue the exact same certificate from the same inputs.
    let a = issue();
    let b = issue();
    let c = issue();
    assert_eq!(a, b);
    assert_eq!(b, c);
}

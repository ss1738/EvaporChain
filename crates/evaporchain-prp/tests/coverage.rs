//! Coverage tests for Provable Retention Proofs (PRP) — positive
//! dual of `evaporchain-decay-forget`. Per `INVENTION_STACK.md §4.1
//! #11` (regulator-survival primitive).
//!
//! Existing tests cover the happy path + window boundary + tamper-
//! witness / tamper-committed_energy. This file adds:
//!
//!   - Per-field tampering for the remaining binding inputs
//!     (state_id, activated_epoch, retained_until_epoch).
//!   - DST drift detector — the witness MUST be domain-separated
//!     against `evaporchain-prp`.
//!   - `prove_retention` determinism + state_id distinguishability.
//!   - Boundary cases on floor + committed_energy.
//!   - Serde round-trip.
//!   - `RetentionProofError` Display + Eq.

use evaporchain_prp::{
    prove_retention, verify_retention_proof, RetentionProof, RetentionProofError,
};
use evaporchain_energy_kernel::{ChainLambda, Lambda};

fn lambda_100() -> ChainLambda {
    ChainLambda::new(Lambda::from_epochs(100))
}

// =================================================================
// Per-field tampering
// =================================================================

#[test]
fn tamper_state_id_rejected() {
    let mut p = prove_retention([7u8; 32], 1_000_000, lambda_100(), 0, 1);
    p.state_id[0] ^= 0xFF;
    let err = verify_retention_proof(&p, 0).expect_err("must reject");
    assert!(matches!(err, RetentionProofError::WitnessMismatch { .. }));
}

#[test]
fn tamper_activated_epoch_rejected() {
    let mut p = prove_retention([7u8; 32], 1_000_000, lambda_100(), 0, 1);
    p.activated_epoch = p.activated_epoch.saturating_add(1);
    let err = verify_retention_proof(&p, 0).expect_err("must reject");
    assert!(matches!(err, RetentionProofError::WitnessMismatch { .. }));
}

#[test]
fn tamper_retained_until_epoch_rejected() {
    let mut p = prove_retention([7u8; 32], 1_000_000, lambda_100(), 0, 1);
    p.retained_until_epoch = p.retained_until_epoch.saturating_add(1);
    let err = verify_retention_proof(&p, 0).expect_err("must reject");
    assert!(matches!(err, RetentionProofError::WitnessMismatch { .. }));
}

// =================================================================
// DST drift detector
// =================================================================

#[test]
fn witness_uses_evaporchain_prp_dst() {
    // If a refactor drops the `evaporchain-prp` DST, this test fires.
    let state = [0xBBu8; 32];
    let committed: u64 = 1_000_000;
    let p = prove_retention(state, committed, lambda_100(), 5, 1);

    let mut raw = blake3::Hasher::new();
    raw.update(&state);
    raw.update(&p.activated_epoch.to_le_bytes());
    raw.update(&p.committed_energy.to_le_bytes());
    raw.update(&p.retained_until_epoch.to_le_bytes());
    let no_dst: [u8; 32] = *raw.finalize().as_bytes();

    assert_ne!(
        p.witness, no_dst,
        "witness must include the evaporchain-prp DST prefix"
    );
}

// =================================================================
// prove_retention properties
// =================================================================

#[test]
fn prove_retention_is_deterministic() {
    let a = prove_retention([3u8; 32], 1_000, lambda_100(), 0, 1);
    let b = prove_retention([3u8; 32], 1_000, lambda_100(), 0, 1);
    assert_eq!(a, b);
}

#[test]
fn prove_retention_distinct_state_ids_produce_distinct_witnesses() {
    let a = prove_retention([1u8; 32], 1_000, lambda_100(), 0, 1);
    let b = prove_retention([2u8; 32], 1_000, lambda_100(), 0, 1);
    assert_ne!(a.witness, b.witness);
    assert_eq!(
        a.retained_until_epoch, b.retained_until_epoch,
        "same inputs except state_id → same retention window, but different witness"
    );
}

#[test]
fn prove_retention_floor_at_or_above_committed_yields_zero_retention() {
    // floor >= committed → energy_at_epoch(_, _, 0) == committed,
    // which is NOT > floor at t=0. Binary search lands on lo=0 ⇒
    // retained_until == activated (no retention beyond start).
    let p_eq    = prove_retention([0u8; 32], 1_000, lambda_100(), 100, 1_000);
    let p_above = prove_retention([0u8; 32], 1_000, lambda_100(), 100, 5_000);
    assert_eq!(p_eq.retained_until_epoch, 100);
    assert_eq!(p_above.retained_until_epoch, 100);
}

#[test]
fn prove_retention_floor_zero_extends_to_search_horizon() {
    // floor=0 means `remaining > 0` always until energy hits 0
    // (~64 halvings). The search horizon is half_life * 64.
    let p = prove_retention([0u8; 32], 1_000, lambda_100(), 0, 0);
    // half_life=100, so the search reaches up to ~6400 epochs.
    // Decay is exponential — after ~6400 epochs energy is effectively 0
    // (1000 / 2^64 == 0 in integer arithmetic).
    assert!(p.retained_until_epoch >= 100,
        "floor=0 must yield retention beyond one half-life, got {}",
        p.retained_until_epoch);
}

#[test]
fn prove_retention_committed_zero_yields_zero_retention() {
    // committed=0 means energy is already 0 at activation;
    // 0 > any non-negative floor is false ⇒ retained_until == activated.
    let p = prove_retention([0u8; 32], 0, lambda_100(), 42, 0);
    assert_eq!(p.retained_until_epoch, 42);
}

// =================================================================
// verify_retention_proof boundary
// =================================================================

#[test]
fn verify_at_exact_retained_until_succeeds() {
    let p = prove_retention([7u8; 32], 1_000_000, lambda_100(), 0, 1);
    // Boundary IS inclusive: query == retained_until must verify.
    verify_retention_proof(&p, p.retained_until_epoch).expect("inclusive boundary");
}

#[test]
fn verify_one_epoch_past_retained_until_rejected_with_payload() {
    let p = prove_retention([7u8; 32], 1_000, lambda_100(), 0, 100);
    let beyond = p.retained_until_epoch + 1;
    match verify_retention_proof(&p, beyond) {
        Err(RetentionProofError::QueryAfterRetention { query, retained_until }) => {
            assert_eq!(query, beyond);
            assert_eq!(retained_until, p.retained_until_epoch);
        }
        other => panic!("expected QueryAfterRetention, got {other:?}"),
    }
}

#[test]
fn verify_at_activation_epoch_succeeds() {
    let p = prove_retention([7u8; 32], 1_000_000, lambda_100(), 50, 1);
    verify_retention_proof(&p, 50).expect("activation epoch must verify");
}

// =================================================================
// Serde
// =================================================================

#[test]
fn proof_serde_round_trips_and_still_verifies() {
    let p = prove_retention([0xCAu8; 32], 2_048, lambda_100(), 5, 16);
    let json = serde_json::to_string(&p).expect("serialize");
    let back: RetentionProof = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(p, back);
    verify_retention_proof(&back, p.activated_epoch).expect("must verify after round-trip");
}

// =================================================================
// RetentionProofError ergonomics
// =================================================================

#[test]
fn retention_proof_error_displays_both_variants() {
    let qar = RetentionProofError::QueryAfterRetention { query: 100, retained_until: 50 }
        .to_string();
    let wm = RetentionProofError::WitnessMismatch {
        derived: [0u8; 32], claimed: [1u8; 32],
    }.to_string();
    assert!(qar.contains("100") && qar.contains("50"), "got: {qar}");
    assert!(wm.contains("mismatch"), "got: {wm}");
}

#[test]
fn retention_proof_error_eq_discriminates_variants_and_payloads() {
    // RetentionProofError derives PartialEq + Eq but NOT Clone — pin
    // the equality semantics we DO have.
    let qar_a = RetentionProofError::QueryAfterRetention { query: 1, retained_until: 2 };
    let qar_a2 = RetentionProofError::QueryAfterRetention { query: 1, retained_until: 2 };
    let qar_b = RetentionProofError::QueryAfterRetention { query: 1, retained_until: 3 };
    let wm = RetentionProofError::WitnessMismatch { derived: [0u8; 32], claimed: [0u8; 32] };
    assert_eq!(qar_a, qar_a2);
    assert_ne!(qar_a, qar_b);
    assert_ne!(qar_a, wm);
}

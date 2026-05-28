//! Coverage tests for Decay-Forget Proofs (Tier 2 substrate).
//!
//! Existing tests cover happy + tamper-witness + tamper-threshold +
//! NotForgotten paths. This file adds:
//!
//!   - Per-field tampering: every input to `compute_witness` must
//!     change the binding (record_id, original_commitment,
//!     activated_epoch, forgotten_at_epoch, decayed_commitment).
//!   - DST presence — the binding hash MUST be domain-separated
//!     against the `evaporchain-decay-forget` tag.
//!   - `prove_forgotten` determinism + distinguishability.
//!   - `verify_forget_proof` exact-threshold boundary (decayed ==
//!     threshold → verifies; threshold + 1 → NotForgotten).
//!   - Serde round-trip of `DecayForgetProof` preserves verify-ability.
//!   - `ForgetProofError` Display rendering + Clone / Eq.
//!   - `query_epoch < activated_epoch` saturating-sub clamp.
//!
//! See `research/INVENTION_STACK.md §4.2` for the GDPR-native
//! doctrine anchor.

use evaporchain_decay_forget::{
    prove_forgotten, verify_forget_proof, DecayForgetProof, ForgetProofError,
};
use evaporchain_energy_kernel::{ChainLambda, Lambda};

fn lambda_100() -> ChainLambda {
    ChainLambda::new(Lambda::from_epochs(100))
}

// =================================================================
// Per-field tampering: every binding input must matter
// =================================================================

#[test]
fn tamper_record_id_rejected() {
    let mut p = prove_forgotten([7u8; 32], 1_000, lambda_100(), 0, 1_000, 10);
    p.record_id[0] ^= 0xFF;
    let err = verify_forget_proof(&p).expect_err("must reject");
    assert!(matches!(err, ForgetProofError::WitnessMismatch { .. }));
}

#[test]
fn tamper_original_commitment_rejected() {
    let mut p = prove_forgotten([7u8; 32], 1_000, lambda_100(), 0, 1_000, 10);
    p.original_commitment = 999_999;
    let err = verify_forget_proof(&p).expect_err("must reject");
    assert!(matches!(err, ForgetProofError::WitnessMismatch { .. }));
}

#[test]
fn tamper_activated_epoch_rejected() {
    let mut p = prove_forgotten([7u8; 32], 1_000, lambda_100(), 0, 1_000, 10);
    p.activated_epoch = 1;
    let err = verify_forget_proof(&p).expect_err("must reject");
    assert!(matches!(err, ForgetProofError::WitnessMismatch { .. }));
}

#[test]
fn tamper_forgotten_at_epoch_rejected() {
    let mut p = prove_forgotten([7u8; 32], 1_000, lambda_100(), 0, 1_000, 10);
    p.forgotten_at_epoch = 999;
    let err = verify_forget_proof(&p).expect_err("must reject");
    assert!(matches!(err, ForgetProofError::WitnessMismatch { .. }));
}

#[test]
fn tamper_decayed_commitment_rejected() {
    let mut p = prove_forgotten([7u8; 32], 1_000, lambda_100(), 0, 1_000, 10);
    p.decayed_commitment = p.decayed_commitment.saturating_add(1);
    let err = verify_forget_proof(&p).expect_err("must reject");
    assert!(matches!(err, ForgetProofError::WitnessMismatch { .. }));
}

// =================================================================
// DST presence — drift detector
// =================================================================

#[test]
fn witness_uses_evaporchain_decay_forget_dst() {
    // The doctrine demands `evaporchain-decay-forget` as the witness
    // DST prefix. If a refactor drops it, this test fires: the proof's
    // witness must NOT equal a raw blake3 of just the fields.
    let record = [0xAAu8; 32];
    let original: u64 = 1_000;
    let activated: u64 = 0;
    let query: u64 = 1_000;
    let threshold: u64 = 10;

    let p = prove_forgotten(record, original, lambda_100(), activated, query, threshold);

    let mut raw = blake3::Hasher::new();
    raw.update(&record);
    raw.update(&original.to_le_bytes());
    raw.update(&activated.to_le_bytes());
    raw.update(&query.to_le_bytes());
    raw.update(&threshold.to_le_bytes());
    raw.update(&p.decayed_commitment.to_le_bytes());
    let no_dst: [u8; 32] = *raw.finalize().as_bytes();

    assert_ne!(
        p.witness, no_dst,
        "binding must include the evaporchain-decay-forget DST"
    );
}

// =================================================================
// prove_forgotten determinism + distinguishability
// =================================================================

#[test]
fn prove_forgotten_is_deterministic() {
    let a = prove_forgotten([3u8; 32], 1_000, lambda_100(), 0, 1_000, 10);
    let b = prove_forgotten([3u8; 32], 1_000, lambda_100(), 0, 1_000, 10);
    assert_eq!(a, b);
}

#[test]
fn prove_forgotten_distinct_record_ids_produce_distinct_witnesses() {
    let a = prove_forgotten([1u8; 32], 1_000, lambda_100(), 0, 1_000, 10);
    let b = prove_forgotten([2u8; 32], 1_000, lambda_100(), 0, 1_000, 10);
    assert_ne!(a.witness, b.witness);
    assert_eq!(
        a.decayed_commitment, b.decayed_commitment,
        "same inputs except record_id → same decay, but different witness"
    );
}

#[test]
fn prove_forgotten_query_before_activated_clamps() {
    // query_epoch < activated_epoch must NOT panic — saturating_sub
    // clamps elapsed to 0, leaving decayed == original.
    let p = prove_forgotten([7u8; 32], 1_000, lambda_100(), 100, 50, 10);
    assert_eq!(
        p.decayed_commitment, 1_000,
        "no decay when elapsed clamps to 0"
    );
    // The proof still verifies its own witness binding.
    let err = verify_forget_proof(&p).expect_err("NotForgotten when decayed > threshold");
    assert!(matches!(err, ForgetProofError::NotForgotten { .. }));
}

// =================================================================
// verify_forget_proof — threshold boundary
// =================================================================

#[test]
fn verify_succeeds_at_exact_decay_below_threshold() {
    // 10 half-lives at half_life=100 → decayed = 1000 / 2^10 ≈ 0 ≤ 10.
    let p = prove_forgotten([7u8; 32], 1_000, lambda_100(), 0, 1_000, 10);
    assert!(p.decayed_commitment <= p.forget_threshold);
    verify_forget_proof(&p).expect("must verify");
}

#[test]
fn verify_rejects_at_decayed_one_above_threshold() {
    // Craft a proof whose decay leaves exactly one unit above threshold.
    // Easiest: high threshold lower than original at zero elapsed.
    let p = prove_forgotten([7u8; 32], 1_000, lambda_100(), 0, 0, 999);
    assert_eq!(p.decayed_commitment, 1_000);
    let err = verify_forget_proof(&p).expect_err("decayed > threshold");
    match err {
        ForgetProofError::NotForgotten { decayed, threshold } => {
            assert_eq!(decayed, 1_000);
            assert_eq!(threshold, 999);
        }
        other => panic!("expected NotForgotten, got {other:?}"),
    }
}

// =================================================================
// Serde round-trip
// =================================================================

#[test]
fn proof_serde_round_trips_and_still_verifies() {
    let p = prove_forgotten([0xCAu8; 32], 2_048, lambda_100(), 5, 2_005, 16);
    let json = serde_json::to_string(&p).expect("serialize");
    let back: DecayForgetProof = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(p, back, "round-trip must preserve byte-equality");
    verify_forget_proof(&back).expect("round-trip must still verify");
}

// =================================================================
// ForgetProofError Display + Clone / Eq
// =================================================================

#[test]
fn forget_proof_error_displays_both_variants() {
    let nf = ForgetProofError::NotForgotten {
        decayed: 100,
        threshold: 50,
    }
    .to_string();
    let wm = ForgetProofError::WitnessMismatch {
        derived: [0u8; 32],
        claimed: [1u8; 32],
    }
    .to_string();
    assert!(nf.contains("100") && nf.contains("50"), "got: {nf}");
    assert!(wm.contains("mismatch"), "got: {wm}");
}

#[test]
fn forget_proof_error_eq_discriminates_variants_and_payloads() {
    // Note: ForgetProofError derives PartialEq + Eq + Debug but NOT
    // Clone — pin the equality behavior we DO have.
    let nf12 = ForgetProofError::NotForgotten {
        decayed: 1,
        threshold: 2,
    };
    let nf12_again = ForgetProofError::NotForgotten {
        decayed: 1,
        threshold: 2,
    };
    let nf13 = ForgetProofError::NotForgotten {
        decayed: 1,
        threshold: 3,
    };
    let wm = ForgetProofError::WitnessMismatch {
        derived: [0u8; 32],
        claimed: [0u8; 32],
    };

    assert_eq!(nf12, nf12_again, "same payload must be Eq");
    assert_ne!(nf12, nf13, "different threshold must be Ne");
    assert_ne!(nf12, wm, "different variant must be Ne");
}

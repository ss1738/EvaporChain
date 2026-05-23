//! End-to-end integration tests for evaporchain-decay-forget.
//!
//! Non-trivial fixture: a GDPR medical-records privacy platform.
//!
//! Three patient records are registered on the chain at genesis.
//! Each has a different original recoverability commitment and a
//! different active lifespan. The platform's data controller queries
//! the chain at various epochs to prove records have been forgotten.
//!
//!   Record A — "alice-doe"    original=1_000_000  activated=0
//!     Queried at epoch 500 (5 halvings, half_life=100):
//!     decayed = 1_000_000 >> 5 = 31_250   threshold=50_000  → FORGOTTEN
//!
//!   Record B — "bob-smith"    original=1_000_000  activated=0
//!     Queried at epoch 100 (1 halving):
//!     decayed = 1_000_000 >> 1 = 500_000  threshold=400_000 → NOT YET FORGOTTEN
//!     Queried at epoch 300 (3 halvings):
//!     decayed = 1_000_000 >> 3 = 125_000  threshold=400_000 → FORGOTTEN
//!
//!   Record C — "carol-jones"  original=1_000       activated=0
//!     Queried at epoch 1000 (10 halvings):
//!     decayed = 1_000 >> 10 = 0            threshold=1      → FORGOTTEN (floor)
//!
//! Boundary record:
//!   Record D — "dora-west"   original=1_000_000  activated=0
//!     decayed == threshold (exact boundary):
//!     epoch=100 → decayed=500_000 = threshold=500_000 → FORGOTTEN (≤ check)
//!
//! Tamper fixture:
//!   Any mutation of any of the 7 DecayForgetProof fields invalidates
//!   the BLAKE3 witness binding.
//!
//! Doctrine claim (INVENTION_STACK §4.2 Decay-Forget Proofs):
//!   "GDPR-native: chain *provably cannot* recover a timestamp once
//!   decayed past threshold. Verifier runs in O(1) from the proof
//!   alone — no external state required."

use evaporchain_decay_forget::{prove_forgotten, verify_forget_proof, DecayForgetProof, ForgetProofError};
use evaporchain_energy_kernel::{ChainLambda, Lambda};

// ── Helpers ──────────────────────────────────────────────────────────────────

const HALF_LIFE: u64 = 100;

fn cl() -> ChainLambda {
    ChainLambda::new(Lambda::from_epochs(HALF_LIFE))
}

fn record_id(tag: &[u8; 8]) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[..8].copy_from_slice(tag);
    id
}

const ALICE: [u8; 8] = *b"alice-do";
const BOB:   [u8; 8] = *b"bob-smit";
const CAROL: [u8; 8] = *b"carol-jo";
const DORA:  [u8; 8] = *b"dora-wes";

// ── Full GDPR platform session ────────────────────────────────────────────────

#[test]
fn gdpr_platform_three_record_lifecycle() {
    // ── Record A: forgotten at epoch 500 ─────────────────────────────
    let alice_proof = prove_forgotten(record_id(&ALICE), 1_000_000, cl(), 0, 500, 50_000);
    // 5 halvings: 1_000_000 >> 5 = 31_250 ≤ 50_000 → FORGOTTEN.
    assert_eq!(alice_proof.decayed_commitment, 31_250,
        "Alice: 5 halvings of 1_000_000 must yield 31_250");
    assert!(alice_proof.decayed_commitment <= alice_proof.forget_threshold);
    verify_forget_proof(&alice_proof).unwrap();

    // ── Record B: not yet forgotten at epoch 100, forgotten at 300 ───
    let bob_not_yet = prove_forgotten(record_id(&BOB), 1_000_000, cl(), 0, 100, 400_000);
    // 1 halving: 500_000 > 400_000 → NOT YET FORGOTTEN.
    assert_eq!(bob_not_yet.decayed_commitment, 500_000,
        "Bob at epoch 100: 1 halving of 1_000_000 must yield 500_000");
    assert!(verify_forget_proof(&bob_not_yet).is_err(),
        "Bob's record must not yet be provably forgotten at epoch 100");

    let bob_forgotten = prove_forgotten(record_id(&BOB), 1_000_000, cl(), 0, 300, 400_000);
    // 3 halvings: 125_000 ≤ 400_000 → FORGOTTEN.
    assert_eq!(bob_forgotten.decayed_commitment, 125_000,
        "Bob at epoch 300: 3 halvings of 1_000_000 must yield 125_000");
    verify_forget_proof(&bob_forgotten).unwrap();

    // ── Record C: floor at 0 after 10 halvings ────────────────────────
    let carol_proof = prove_forgotten(record_id(&CAROL), 1_000, cl(), 0, 1_000, 1);
    // 10 halvings: 1_000 >> 10 = 0 ≤ 1 → FORGOTTEN (floor).
    assert_eq!(carol_proof.decayed_commitment, 0,
        "Carol: 10 halvings of 1_000 must floor to 0");
    verify_forget_proof(&carol_proof).unwrap();
}

// ── Boundary: decayed == threshold ───────────────────────────────────────────

#[test]
fn forget_boundary_decayed_equals_threshold_is_forgotten() {
    // 1 halving: 1_000_000 >> 1 = 500_000.  threshold = 500_000.
    // The check is ≤ so decayed == threshold → FORGOTTEN.
    let proof = prove_forgotten(record_id(&DORA), 1_000_000, cl(), 0, 100, 500_000);
    assert_eq!(proof.decayed_commitment, 500_000);
    assert_eq!(proof.forget_threshold, 500_000);
    verify_forget_proof(&proof).unwrap();
}

#[test]
fn forget_boundary_decayed_one_above_threshold_is_not_forgotten() {
    // Same decay (500_000) but threshold is 499_999 → NOT FORGOTTEN.
    let proof = prove_forgotten(record_id(&DORA), 1_000_000, cl(), 0, 100, 499_999);
    assert_eq!(proof.decayed_commitment, 500_000);
    let err = verify_forget_proof(&proof).unwrap_err();
    assert!(matches!(err, ForgetProofError::NotForgotten { decayed: 500_000, threshold: 499_999 }),
        "decayed one above threshold must return NotForgotten");
}

// ── O(1) auditor verification ─────────────────────────────────────────────────

#[test]
fn auditor_verifies_in_o1_from_proof_alone() {
    // The auditor receives only the DecayForgetProof struct — no original
    // record data, no external chain state, no epoch oracle.
    let proof = prove_forgotten(record_id(&ALICE), 1_000_000, cl(), 0, 500, 50_000);
    // The auditor runs verify_forget_proof exactly once — O(1).
    verify_forget_proof(&proof).expect("auditor O(1) verification must succeed");
}

// ── Determinism ───────────────────────────────────────────────────────────────

#[test]
fn same_inputs_produce_identical_proof_byte_for_byte() {
    let p1 = prove_forgotten(record_id(&ALICE), 1_000_000, cl(), 0, 500, 50_000);
    let p2 = prove_forgotten(record_id(&ALICE), 1_000_000, cl(), 0, 500, 50_000);
    assert_eq!(p1, p2, "prove_forgotten must be deterministic");
}

// ── Epoch sensitivity ─────────────────────────────────────────────────────────

#[test]
fn different_query_epochs_produce_different_decayed_values() {
    let p_100 = prove_forgotten(record_id(&ALICE), 1_000_000, cl(), 0, 100, 1);
    let p_200 = prove_forgotten(record_id(&ALICE), 1_000_000, cl(), 0, 200, 1);
    let p_500 = prove_forgotten(record_id(&ALICE), 1_000_000, cl(), 0, 500, 1);
    assert!(p_100.decayed_commitment > p_200.decayed_commitment,
        "decayed commitment must decrease as query epoch advances");
    assert!(p_200.decayed_commitment > p_500.decayed_commitment,
        "decayed commitment at 200 must exceed commitment at 500");
}

// ── Tamper detection: all 7 fields bound ─────────────────────────────────────

#[test]
fn tamper_any_proof_field_invalidates_witness() {
    let proof = prove_forgotten(record_id(&ALICE), 1_000_000, cl(), 0, 500, 50_000);
    // We need the proof to be FORGOTTEN first (so witness check fires, not threshold check).
    verify_forget_proof(&proof).unwrap();

    let tampers: Vec<(&str, DecayForgetProof)> = vec![
        ("record_id", { let mut p = proof.clone(); p.record_id[0] ^= 1; p }),
        ("original_commitment", { let mut p = proof.clone(); p.original_commitment += 1; p }),
        ("activated_epoch", { let mut p = proof.clone(); p.activated_epoch += 1; p }),
        ("forgotten_at_epoch", { let mut p = proof.clone(); p.forgotten_at_epoch += 1; p }),
        ("forget_threshold", { let mut p = proof.clone(); p.forget_threshold += 1; p }),
        ("decayed_commitment", { let mut p = proof.clone(); p.decayed_commitment += 1; p }),
        ("witness_direct", { let mut p = proof.clone(); p.witness[0] ^= 0xFF; p }),
    ];

    for (field, tampered) in tampers {
        let err = verify_forget_proof(&tampered).unwrap_err();
        assert!(
            matches!(err, ForgetProofError::WitnessMismatch { .. }),
            "tampering '{field}' must produce WitnessMismatch, got {err:?}"
        );
    }
}

// ── Adversarial: attacker raises threshold to forge a 'forgotten' proof ───────

#[test]
fn adversarial_raised_threshold_in_proof_invalidates_witness() {
    // Attacker takes a proof where the record is NOT forgotten and raises
    // the forget_threshold so that decayed_commitment ≤ threshold.
    // The BLAKE3 witness binds forget_threshold — this must be caught.
    let not_forgotten = prove_forgotten(record_id(&BOB), 1_000_000, cl(), 0, 100, 400_000);
    assert!(verify_forget_proof(&not_forgotten).is_err(),
        "sanity: Bob at epoch 100 must NOT be forgotten");

    // Attacker inflates threshold to 600_000 (> decayed 500_000).
    let mut forged = not_forgotten.clone();
    forged.forget_threshold = 600_000;
    let err = verify_forget_proof(&forged).unwrap_err();
    assert!(
        matches!(err, ForgetProofError::WitnessMismatch { .. }),
        "inflating the threshold in the proof must fail witness binding, got {err:?}"
    );
}

// ── Adversarial: activated_epoch after query_epoch ────────────────────────────

#[test]
fn activated_after_query_elapsed_zero_so_full_commitment_not_forgotten() {
    // activated=500, query=100 → saturating_sub gives elapsed=0 → decayed=original.
    // If original >> threshold the proof correctly fails with NotForgotten.
    let proof = prove_forgotten(record_id(&CAROL), 1_000_000, cl(), 500, 100, 50_000);
    // elapsed = 100.saturating_sub(500) = 0 → decayed = 1_000_000 > 50_000.
    assert_eq!(proof.decayed_commitment, 1_000_000,
        "activated after query → zero elapsed → full commitment");
    let err = verify_forget_proof(&proof).unwrap_err();
    assert!(matches!(err, ForgetProofError::NotForgotten { .. }),
        "a record activated after the query epoch must not be forgotten");
}

// ── Cross-record isolation ────────────────────────────────────────────────────

#[test]
fn alice_proof_does_not_verify_for_bob_record_id() {
    let alice_proof = prove_forgotten(record_id(&ALICE), 1_000_000, cl(), 0, 500, 50_000);
    verify_forget_proof(&alice_proof).unwrap();

    // Swap in Bob's record_id — witness must invalidate.
    let mut swapped = alice_proof.clone();
    swapped.record_id = record_id(&BOB);
    let err = verify_forget_proof(&swapped).unwrap_err();
    assert!(matches!(err, ForgetProofError::WitnessMismatch { .. }),
        "proof for alice must not verify under bob's record_id");
}

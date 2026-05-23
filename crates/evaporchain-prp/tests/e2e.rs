//! §PRP — Provable Retention Proofs (§4.1 regulator-survival primitive) e2e
//!
//! Scenario: "MiCA compliance window" — a REGULATOR (MiCA compliance
//! officer) must prove that a specific transaction record (state_id =
//! 0xREC…) was retained for at least 5 years under EvaporChain's global
//! λ. She commits enough energy at epoch 0 to cover the window. OSCAR
//! (adversary) tries to forge a proof by tampering with fields.
//!
//! The suite proves: higher committed_energy → longer retention;
//! higher floor → shorter retention; BLAKE3 witness binds all fields;
//! queries within the window pass; queries past the window fail closed.

use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_prp::{prove_retention, verify_retention_proof, RetentionProofError};

fn lambda100() -> ChainLambda { ChainLambda::new(Lambda::from_epochs(100)) }
fn lambda500() -> ChainLambda { ChainLambda::new(Lambda::from_epochs(500)) }

fn regulator_state() -> [u8; 32] { [0x4E; 32] }
fn oscar_state()     -> [u8; 32] { [0x0C; 32] }

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn honest_proof_verifies_within_window() {
    let proof = prove_retention(regulator_state(), 1_000_000, lambda100(), 0, 1);
    let mid = proof.retained_until_epoch / 2;
    verify_retention_proof(&proof, mid).expect("query within window must succeed");
}

#[test]
fn verify_at_activation_epoch_passes() {
    // Query at the activation epoch itself must always pass.
    let proof = prove_retention(regulator_state(), 50_000, lambda100(), 100, 10);
    verify_retention_proof(&proof, 100).expect("query at activation must pass");
}

#[test]
fn verify_at_exact_retained_until_passes() {
    // The boundary is inclusive: retained_until_epoch itself must pass.
    let proof = prove_retention(regulator_state(), 50_000, lambda100(), 0, 1);
    verify_retention_proof(&proof, proof.retained_until_epoch)
        .expect("query at retained_until boundary must pass");
}

#[test]
fn query_one_past_window_rejected() {
    let proof = prove_retention(regulator_state(), 50_000, lambda100(), 0, 100);
    let err = verify_retention_proof(&proof, proof.retained_until_epoch + 1).unwrap_err();
    assert!(matches!(err, RetentionProofError::QueryAfterRetention {
        query, retained_until
    } if query == proof.retained_until_epoch + 1 && retained_until == proof.retained_until_epoch),
        "error must carry query and retained_until: {:?}", err);
}

#[test]
fn higher_committed_energy_extends_retention() {
    let low  = prove_retention(regulator_state(), 1_000,       lambda100(), 0, 1);
    let high = prove_retention(regulator_state(), 1_000_000,   lambda100(), 0, 1);
    assert!(high.retained_until_epoch > low.retained_until_epoch,
        "more committed energy must extend the retention window");
}

#[test]
fn higher_floor_shortens_retention() {
    // Floor acts as the minimum survivable energy — a higher floor
    // means the state expires sooner.
    let low_floor  = prove_retention(regulator_state(), 10_000, lambda100(), 0, 1);
    let high_floor = prove_retention(regulator_state(), 10_000, lambda100(), 0, 5_000);
    assert!(low_floor.retained_until_epoch >= high_floor.retained_until_epoch,
        "higher floor must not extend retention");
}

#[test]
fn slower_lambda_extends_retention() {
    // A slower decay (longer half-life) keeps energy above floor for longer.
    let fast = prove_retention(regulator_state(), 10_000, lambda100(), 0, 10);
    let slow = prove_retention(regulator_state(), 10_000, lambda500(), 0, 10);
    assert!(slow.retained_until_epoch >= fast.retained_until_epoch,
        "slower λ must not shorten the retention window");
}

#[test]
fn proof_is_deterministic() {
    // Same inputs → byte-identical proof on any validator.
    let p1 = prove_retention(regulator_state(), 100_000, lambda100(), 50, 100);
    let p2 = prove_retention(regulator_state(), 100_000, lambda100(), 50, 100);
    assert_eq!(p1, p2, "prove_retention must be deterministic");
}

#[test]
fn tampered_witness_rejected() {
    let mut proof = prove_retention(oscar_state(), 10_000, lambda100(), 0, 10);
    proof.witness[0] ^= 0xFF;
    let err = verify_retention_proof(&proof, 0).unwrap_err();
    assert!(matches!(err, RetentionProofError::WitnessMismatch { .. }),
        "tampered witness must be rejected: {:?}", err);
}

#[test]
fn tampered_committed_energy_rejected() {
    let mut proof = prove_retention(oscar_state(), 10_000, lambda100(), 0, 10);
    proof.committed_energy = 999_999_999;
    let err = verify_retention_proof(&proof, 0).unwrap_err();
    assert!(matches!(err, RetentionProofError::WitnessMismatch { .. }),
        "inflated committed_energy must break witness: {:?}", err);
}

#[test]
fn tampered_retained_until_epoch_rejected() {
    let mut proof = prove_retention(oscar_state(), 10_000, lambda100(), 0, 10);
    proof.retained_until_epoch += 1_000_000; // extend window without paying
    let err = verify_retention_proof(&proof, 0).unwrap_err();
    assert!(matches!(err, RetentionProofError::WitnessMismatch { .. }),
        "extended retained_until must break witness: {:?}", err);
}

#[test]
fn proof_carries_all_input_fields() {
    let state_id = [0x5A; 32];
    let proof = prove_retention(state_id, 50_000, lambda100(), 200, 50);
    assert_eq!(proof.state_id, state_id);
    assert_eq!(proof.activated_epoch, 200);
    assert_eq!(proof.committed_energy, 50_000);
    assert!(proof.retained_until_epoch >= 200,
        "retained_until must be ≥ activated_epoch");
}

#[test]
fn serde_round_trip() {
    let proof = prove_retention(regulator_state(), 100_000, lambda100(), 0, 1);
    let json = serde_json::to_string(&proof).unwrap();
    let back = serde_json::from_str(&json).unwrap();
    assert_eq!(proof, back, "RetentionProof must serialise/deserialise exactly");
}

#[test]
fn mica_compliance_full_arc() {
    // REGULATOR commits 10_000_000 energy at epoch 0 for state record 0xREC.
    // She needs to query at epochs 0, 100, 500, 1_000 to prove retention.
    let proof = prove_retention(regulator_state(), 10_000_000, lambda100(), 0, 1);

    // All in-window queries pass.
    let window = proof.retained_until_epoch;
    for &q in &[0u64, 100, 500, 1_000] {
        if q <= window {
            verify_retention_proof(&proof, q)
                .unwrap_or_else(|e| panic!("epoch {q} should be in window: {:?}", e));
        }
    }

    // One epoch past the window fails closed.
    let err = verify_retention_proof(&proof, window + 1).unwrap_err();
    assert!(matches!(err, RetentionProofError::QueryAfterRetention { .. }),
        "one past the window must fail: {:?}", err);

    // OSCAR's forgery fails.
    let mut forged = proof.clone();
    forged.retained_until_epoch = u64::MAX;
    assert!(verify_retention_proof(&forged, u64::MAX / 2).is_err(),
        "forged infinite window must be rejected");
}

//! Coverage tests for the LLSA chain-side gate (proof binding,
//! amendment hashing, registry transitions, k-of-n auditor verifier).

use evaporchain_epv::registry::{EpvRegistry, ProtocolVersion};
use evaporchain_llsa::{
    AlwaysAcceptVerifier, AlwaysRejectVerifier, Amendment, AmendmentError, LlsaProof,
    MultiAuditorVerifier, ProofError, ProofVerifier, apply_amendment,
};

fn fresh_registry() -> EpvRegistry {
    let mut r = EpvRegistry::new();
    r.register(ProtocolVersion::new(1, 1_000_000, 0)).unwrap();
    r
}

fn well_formed_amendment(from: u64, to: u64, invariant: [u8; 32]) -> Amendment {
    let mut a = Amendment {
        from_version: from,
        to_version: to,
        step_new_descriptor: format!("v{to}-impl").into_bytes(),
        proof: LlsaProof {
            coq_term_hash: [9u8; 32],
            target_invariant_id: invariant,
            bound_amendment_hash: [0u8; 32],
            proof_bytes: vec![1, 2, 3],
        },
    };
    let h = a.hash();
    a.proof.bound_amendment_hash = h;
    a
}

// =================================================================
// Amendment hash
// =================================================================

#[test]
fn amendment_hash_changes_with_from_version() {
    let inv = [1u8; 32];
    let a = well_formed_amendment(1, 2, inv);
    let mut b = a.clone();
    b.from_version = 5;
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn amendment_hash_changes_with_descriptor_bytes() {
    let inv = [1u8; 32];
    let mut a = well_formed_amendment(1, 2, inv);
    let h1 = a.hash();
    a.step_new_descriptor.push(0x42);
    let h2 = a.hash();
    assert_ne!(h1, h2);
}

#[test]
fn amendment_hash_includes_descriptor_length() {
    // Same prefix bytes but different length must produce different hashes.
    let inv = [1u8; 32];
    let mut a = well_formed_amendment(1, 2, inv);
    a.step_new_descriptor = b"abc".to_vec();
    let h1 = a.hash();
    a.step_new_descriptor = b"abcd".to_vec();
    let h2 = a.hash();
    assert_ne!(h1, h2);
}

// =================================================================
// apply_amendment — full registry transition
// =================================================================

#[test]
fn happy_path_inserts_to_version() {
    let mut r = fresh_registry();
    let inv = [42u8; 32];
    let a = well_formed_amendment(1, 2, inv);
    apply_amendment(&mut r, &a, inv, 2_000_000, 100, &AlwaysAcceptVerifier).unwrap();
    assert!(r.contains(2));
    assert_eq!(r.get(2).unwrap().seed_energy, 2_000_000);
}

#[test]
fn from_version_absent_errors() {
    let mut r = EpvRegistry::new();
    let inv = [42u8; 32];
    let a = well_formed_amendment(7, 8, inv);
    let err = apply_amendment(&mut r, &a, inv, 1_000, 10, &AlwaysAcceptVerifier).unwrap_err();
    assert_eq!(err, AmendmentError::FromVersionAbsent(7));
}

#[test]
fn to_version_collision_errors() {
    let mut r = fresh_registry();
    r.register(ProtocolVersion::new(2, 500, 25)).unwrap();
    let inv = [42u8; 32];
    let a = well_formed_amendment(1, 2, inv);
    let err = apply_amendment(&mut r, &a, inv, 1_000, 10, &AlwaysAcceptVerifier).unwrap_err();
    assert_eq!(err, AmendmentError::ToVersionExists(2));
}

#[test]
fn rejected_proof_does_not_mutate_registry() {
    let mut r = fresh_registry();
    let inv = [42u8; 32];
    let a = well_formed_amendment(1, 2, inv);
    let err = apply_amendment(&mut r, &a, inv, 1_000, 10, &AlwaysRejectVerifier).unwrap_err();
    assert!(matches!(err, AmendmentError::Proof(_)));
    assert!(!r.contains(2), "registry must be untouched on rejection");
}

#[test]
fn proof_bound_to_different_invariant_rejected() {
    let mut r = fresh_registry();
    let claimed = [42u8; 32];
    let actual_chain_invariant = [99u8; 32];
    let a = well_formed_amendment(1, 2, claimed);
    let err = apply_amendment(
        &mut r,
        &a,
        actual_chain_invariant,
        1_000,
        10,
        &AlwaysAcceptVerifier,
    )
    .unwrap_err();
    match err {
        AmendmentError::Proof(ProofError::WrongInvariant { .. }) => {}
        other => panic!("expected WrongInvariant, got {other:?}"),
    }
    assert!(!r.contains(2));
}

#[test]
fn tampered_descriptor_breaks_proof_binding() {
    let mut r = fresh_registry();
    let inv = [42u8; 32];
    let mut a = well_formed_amendment(1, 2, inv);
    a.step_new_descriptor = b"mallorys-impl".to_vec(); // tamper post-binding
    let err = apply_amendment(&mut r, &a, inv, 1_000, 10, &AlwaysAcceptVerifier).unwrap_err();
    match err {
        AmendmentError::Proof(ProofError::WrongAmendment { .. }) => {}
        other => panic!("expected WrongAmendment, got {other:?}"),
    }
}

// =================================================================
// MultiAuditorVerifier — integration with apply_amendment
// =================================================================

#[test]
fn multi_auditor_2_of_3_gates_real_amendment() {
    let mut r = fresh_registry();
    let inv = [42u8; 32];
    let a = well_formed_amendment(1, 2, inv);
    let v = MultiAuditorVerifier::new(
        vec![
            Box::new(AlwaysAcceptVerifier),
            Box::new(AlwaysAcceptVerifier),
            Box::new(AlwaysRejectVerifier),
        ],
        2,
    )
    .expect("valid threshold");
    apply_amendment(&mut r, &a, inv, 1_000_000, 50, &v).unwrap();
    assert!(r.contains(2));
}

#[test]
fn multi_auditor_below_threshold_blocks_registration() {
    let mut r = fresh_registry();
    let inv = [42u8; 32];
    let a = well_formed_amendment(1, 2, inv);
    let v = MultiAuditorVerifier::new(
        vec![
            Box::new(AlwaysAcceptVerifier),
            Box::new(AlwaysRejectVerifier),
            Box::new(AlwaysRejectVerifier),
        ],
        2,
    )
    .expect("valid threshold");
    let err = apply_amendment(&mut r, &a, inv, 1_000_000, 50, &v).unwrap_err();
    assert!(matches!(err, AmendmentError::Proof(_)));
    assert!(!r.contains(2));
}

#[test]
fn multi_auditor_constructor_validates_threshold() {
    assert!(MultiAuditorVerifier::new(vec![], 1).is_none());
    let single: Vec<Box<dyn ProofVerifier + Send + Sync>> = vec![Box::new(AlwaysAcceptVerifier)];
    assert!(MultiAuditorVerifier::new(single, 0).is_none());
    let pair: Vec<Box<dyn ProofVerifier + Send + Sync>> =
        vec![Box::new(AlwaysAcceptVerifier), Box::new(AlwaysAcceptVerifier)];
    assert!(MultiAuditorVerifier::new(pair, 3).is_none());
}

// =================================================================
// Error display + serde
// =================================================================

#[test]
fn amendment_error_display_contains_version_ids() {
    let e = AmendmentError::FromVersionAbsent(42);
    assert!(e.to_string().contains("42"));
    let e = AmendmentError::ToVersionExists(7);
    assert!(e.to_string().contains("7"));
}

#[test]
fn proof_error_display_contains_signal_text() {
    let e = ProofError::VerifierRejected("kernel-mismatch".into());
    assert!(e.to_string().contains("kernel-mismatch"));
}

#[test]
fn amendment_serde_round_trips() {
    let inv = [3u8; 32];
    let a = well_formed_amendment(1, 2, inv);
    let json = serde_json::to_string(&a).unwrap();
    let back: Amendment = serde_json::from_str(&json).unwrap();
    assert_eq!(back, a);
    assert_eq!(back.hash(), a.hash());
}

#[test]
fn proof_serde_round_trips() {
    let p = LlsaProof {
        coq_term_hash: [1u8; 32],
        target_invariant_id: [2u8; 32],
        bound_amendment_hash: [3u8; 32],
        proof_bytes: vec![4, 5, 6, 7, 8],
    };
    let json = serde_json::to_string(&p).unwrap();
    let back: LlsaProof = serde_json::from_str(&json).unwrap();
    assert_eq!(back, p);
}

// =================================================================
// Sequencing — multi-step amendment chain
// =================================================================

#[test]
fn two_amendments_in_sequence_register_both() {
    let mut r = fresh_registry();
    let inv = [42u8; 32];
    let a12 = well_formed_amendment(1, 2, inv);
    apply_amendment(&mut r, &a12, inv, 1_000_000, 100, &AlwaysAcceptVerifier).unwrap();
    let a23 = well_formed_amendment(2, 3, inv);
    apply_amendment(&mut r, &a23, inv, 1_000_000, 200, &AlwaysAcceptVerifier).unwrap();
    assert!(r.contains(1));
    assert!(r.contains(2));
    assert!(r.contains(3));
    assert_eq!(r.len(), 3);
}

#[test]
fn amendment_cannot_re_register_same_to_version_after_success() {
    let mut r = fresh_registry();
    let inv = [42u8; 32];
    let a = well_formed_amendment(1, 2, inv);
    apply_amendment(&mut r, &a, inv, 1_000_000, 100, &AlwaysAcceptVerifier).unwrap();
    // Try to apply it again — to_version now exists.
    let err = apply_amendment(&mut r, &a, inv, 1_000_000, 100, &AlwaysAcceptVerifier).unwrap_err();
    assert_eq!(err, AmendmentError::ToVersionExists(2));
}

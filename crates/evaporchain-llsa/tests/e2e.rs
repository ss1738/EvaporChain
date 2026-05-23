//! End-to-end integration tests for evaporchain-llsa.
//!
//! Non-trivial fixture: 3-auditor chain upgrade gate.
//!
//! A protocol amendment from v0 → v1 carries an LLSA proof. The
//! MultiAuditorVerifier requires k-of-n acceptance before the amendment
//! is applied to the EpvRegistry.
//!
//!   Fixture 1 (k=2, n=3 — threshold met):
//!     2 of 3 auditors accept → amendment applied; v1 registered.
//!
//!   Fixture 2 (k=2, n=3 — threshold missed):
//!     Only 1 of 3 auditors accepts → ProofError::VerifierRejected.
//!
//!   Fixture 3 (from_version absent):
//!     Amendment from a version not yet registered → AmendmentError.
//!
//!   Fixture 4 (to_version collision):
//!     Duplicate amendment targeting an already-registered version
//!     is rejected.
//!
//!   Fixture 5 (sequential v0→v1→v2 chain):
//!     Two successful amendments build the version chain.
//!
//! Doctrine claim (INVENTION_STACK §A1.2 T4 / §A1.3):
//!   "Lambda-Locked Self-Amendment — protocol upgrades require a
//!    mechanically-checked invariant-preservation proof."
//!
//! INVENTION_STACK.md §A1.2 T4 LLSA.

use evaporchain_epv::{EpvRegistry, ProtocolVersion};
use evaporchain_llsa::amendment::{Amendment, AmendmentError};
use evaporchain_llsa::apply::apply_amendment;
use evaporchain_llsa::proof::{AlwaysAcceptVerifier, AlwaysRejectVerifier, LlsaProof,
                               MultiAuditorVerifier, ProofError};

const V0: u32 = 0;
const V1: u32 = 1;
const V2: u32 = 2;
const INVARIANT: [u8; 32] = [0xCD; 32];

fn genesis_registry() -> EpvRegistry {
    let mut r = EpvRegistry::new();
    r.register(ProtocolVersion::new(V0, 1_000_000, 0)).unwrap();
    r
}

/// Build a proof correctly bound to the supplied amendment and invariant.
/// `AlwaysAcceptVerifier` checks these bindings before accepting.
fn bound_proof(amendment: &Amendment) -> LlsaProof {
    LlsaProof {
        coq_term_hash: [0xABu8; 32],
        target_invariant_id: INVARIANT,
        bound_amendment_hash: amendment.hash(),
        proof_bytes: vec![1, 2, 3],
    }
}

fn v0_to_v1() -> Amendment {
    let descriptor = b"increment-opcode-set".to_vec();
    let mut a = Amendment {
        from_version: V0,
        to_version: V1,
        step_new_descriptor: descriptor,
        proof: LlsaProof {
            coq_term_hash: [0u8; 32],
            target_invariant_id: [0u8; 32],
            bound_amendment_hash: [0u8; 32],
            proof_bytes: vec![],
        },
    };
    a.proof = bound_proof(&a);
    a
}

// ── Fixture 1: k=2, n=3, 2 accept → applied ──────────────────────────────

#[test]
fn three_auditor_threshold_met_applies_amendment() {
    let mut reg = genesis_registry();
    let amendment = v0_to_v1();

    let verifier = MultiAuditorVerifier::new(
        vec![
            Box::new(AlwaysAcceptVerifier),
            Box::new(AlwaysAcceptVerifier),
            Box::new(AlwaysRejectVerifier),
        ],
        2,
    )
    .expect("k=2, n=3 is valid");

    apply_amendment(&mut reg, &amendment, INVARIANT, 500_000, 1, &verifier)
        .expect("2-of-3 threshold met — amendment must apply");

    assert!(reg.contains(V1), "v1 must be registered after successful amendment");
    assert!(reg.contains(V0), "v0 genesis entry preserved");
    assert_eq!(reg.len(), 2);
}

// ── Fixture 2: k=2, n=3, 1 accept → rejected ─────────────────────────────

#[test]
fn three_auditor_threshold_missed_rejects_amendment() {
    let mut reg = genesis_registry();
    let amendment = v0_to_v1();

    let verifier = MultiAuditorVerifier::new(
        vec![
            Box::new(AlwaysAcceptVerifier),
            Box::new(AlwaysRejectVerifier),
            Box::new(AlwaysRejectVerifier),
        ],
        2,
    )
    .expect("k=2, n=3 is valid");

    let err = apply_amendment(&mut reg, &amendment, INVARIANT, 500_000, 1, &verifier)
        .expect_err("only 1-of-3 accept — must fail");

    assert!(
        matches!(err, AmendmentError::Proof(ProofError::VerifierRejected(_))),
        "expected Proof(VerifierRejected), got {err:?}"
    );
    assert!(!reg.contains(V1), "v1 must NOT be registered after rejection");
}

// ── Fixture 3: from_version absent ────────────────────────────────────────

#[test]
fn amendment_from_unregistered_version_rejected() {
    let mut reg = genesis_registry();
    // Amendment from V2 which was never registered.
    let descriptor = b"upgrade-from-v2".to_vec();
    let mut bad = Amendment {
        from_version: V2,
        to_version: V1,
        step_new_descriptor: descriptor,
        proof: LlsaProof {
            coq_term_hash: [0u8; 32],
            target_invariant_id: INVARIANT,
            bound_amendment_hash: [0u8; 32],
            proof_bytes: vec![],
        },
    };
    bad.proof.bound_amendment_hash = bad.hash();

    let err = apply_amendment(&mut reg, &bad, INVARIANT, 0, 0, &AlwaysAcceptVerifier)
        .expect_err("from_version absent — must fail");

    assert!(
        matches!(err, AmendmentError::FromVersionAbsent(V2)),
        "expected FromVersionAbsent(V2), got {err:?}"
    );
}

// ── Fixture 4: to_version collision ───────────────────────────────────────

#[test]
fn duplicate_to_version_rejected() {
    let mut reg = genesis_registry();
    // Register V1 directly first.
    reg.register(ProtocolVersion::new(V1, 200_000, 0)).unwrap();

    let dup = v0_to_v1(); // from V0 → V1, but V1 already exists

    let err = apply_amendment(&mut reg, &dup, INVARIANT, 0, 0, &AlwaysAcceptVerifier)
        .expect_err("to_version collision — must fail");

    assert!(
        matches!(err, AmendmentError::ToVersionExists(V1)),
        "expected ToVersionExists(V1), got {err:?}"
    );
}

// ── Fixture 5: sequential v0 → v1 → v2 ──────────────────────────────────

#[test]
fn sequential_amendments_build_version_chain() {
    let mut reg = genesis_registry();

    apply_amendment(&mut reg, &v0_to_v1(), INVARIANT, 500_000, 1, &AlwaysAcceptVerifier)
        .unwrap();

    let descriptor = b"second-upgrade".to_vec();
    let mut v1v2 = Amendment {
        from_version: V1,
        to_version: V2,
        step_new_descriptor: descriptor,
        proof: LlsaProof {
            coq_term_hash: [0u8; 32],
            target_invariant_id: INVARIANT,
            bound_amendment_hash: [0u8; 32],
            proof_bytes: vec![],
        },
    };
    v1v2.proof.bound_amendment_hash = v1v2.hash();

    apply_amendment(&mut reg, &v1v2, INVARIANT, 400_000, 2, &AlwaysAcceptVerifier)
        .unwrap();

    assert!(reg.contains(V0));
    assert!(reg.contains(V1));
    assert!(reg.contains(V2));
    assert_eq!(reg.len(), 3);
}

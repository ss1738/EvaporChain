//! End-to-end LLSA fixture: a full proposed-amendment lifecycle.
//!
//! Drives `apply_amendment` through the trust-boundary contract:
//!
//! 1. Well-formed amendment + matching invariant + matching hash binding
//!    → registry advances.
//! 2. Same proof bound to a *different* amendment → rejected
//!    (`WrongAmendment`).
//! 3. Proof targeting the wrong invariant → rejected (`WrongInvariant`).
//! 4. Always-reject verifier exercises the kernel-rejection path.
//! 5. Replay of the same amendment against an already-registered
//!    to_version → rejected (`ToVersionExists`).
//!
//! Doctrine: `INVENTION_STACK.md` §A1.2 T4 — protocol upgrades require
//! a kernel-checked `forall s, Inv(s) → Inv(step_new(s))` term.

use evaporchain_epv::{EpvRegistry, ProtocolVersion};
use evaporchain_llsa::amendment::{Amendment, AmendmentError};
use evaporchain_llsa::apply::apply_amendment;
use evaporchain_llsa::proof::{
    AlwaysAcceptVerifier, AlwaysRejectVerifier, InvariantId, LlsaProof, ProofError,
};

fn invariant_id() -> InvariantId {
    let mut h = blake3::Hasher::new();
    h.update(b"e2e-invariant");
    *h.finalize().as_bytes()
}

fn make_amendment(from: u32, to: u32, invariant: InvariantId) -> Amendment {
    let a = Amendment {
        from_version: from,
        to_version: to,
        step_new_descriptor: format!("v{}-impl", to).into_bytes(),
        proof: LlsaProof {
            coq_term_hash: [0xaa; 32],
            target_invariant_id: invariant,
            bound_amendment_hash: [0u8; 32], // patched below
            proof_bytes: b"opaque-coq-cert".to_vec(),
        },
    };
    let h = a.hash();
    let mut a = a;
    a.proof.bound_amendment_hash = h;
    a
}

fn fresh_registry() -> EpvRegistry {
    let mut r = EpvRegistry::new();
    r.register(ProtocolVersion::new(1, 1_000_000, 0)).unwrap();
    r
}

#[test]
fn e2e_full_lifecycle_v1_to_v2_to_v3() {
    let inv = invariant_id();
    let mut registry = fresh_registry();
    let v = AlwaysAcceptVerifier;

    // v1 → v2.
    let a12 = make_amendment(1, 2, inv);
    apply_amendment(&mut registry, &a12, inv, 500_000, 100, &v).unwrap();
    assert!(registry.contains(2));

    // v2 → v3 from the same chain.
    let a23 = make_amendment(2, 3, inv);
    apply_amendment(&mut registry, &a23, inv, 250_000, 200, &v).unwrap();
    assert!(registry.contains(3));
}

#[test]
fn e2e_proof_bound_to_other_amendment_rejected() {
    let inv = invariant_id();
    let mut registry = fresh_registry();
    let v = AlwaysAcceptVerifier;

    // Two distinct amendments — different to_version → different hashes.
    let mut a_target = make_amendment(1, 2, inv);
    let a_other = make_amendment(1, 99, inv);
    // Re-bind a_target's proof to a_other's hash → mismatch.
    a_target.proof.bound_amendment_hash = a_other.hash();

    let err = apply_amendment(&mut registry, &a_target, inv, 500_000, 100, &v).unwrap_err();
    match err {
        AmendmentError::Proof(ProofError::WrongAmendment { .. }) => {}
        other => panic!("expected WrongAmendment, got {:?}", other),
    }
    assert!(!registry.contains(2));
}

#[test]
fn e2e_wrong_invariant_rejected() {
    let inv = invariant_id();
    let mut wrong_inv = inv;
    wrong_inv[0] ^= 0xff;
    let mut registry = fresh_registry();
    let v = AlwaysAcceptVerifier;

    // Proof targets `wrong_inv` but chain expects `inv`.
    let a = make_amendment(1, 2, wrong_inv);
    let err = apply_amendment(&mut registry, &a, inv, 500_000, 100, &v).unwrap_err();
    match err {
        AmendmentError::Proof(ProofError::WrongInvariant { .. }) => {}
        other => panic!("expected WrongInvariant, got {:?}", other),
    }
}

#[test]
fn e2e_kernel_rejection_path() {
    let inv = invariant_id();
    let mut registry = fresh_registry();
    let v = AlwaysRejectVerifier;
    let a = make_amendment(1, 2, inv);

    let err = apply_amendment(&mut registry, &a, inv, 500_000, 100, &v).unwrap_err();
    assert!(matches!(err, AmendmentError::Proof(ProofError::VerifierRejected(_))));
    assert!(!registry.contains(2));
}

#[test]
fn e2e_replay_after_registration_rejected() {
    let inv = invariant_id();
    let mut registry = fresh_registry();
    let v = AlwaysAcceptVerifier;

    let a = make_amendment(1, 2, inv);
    apply_amendment(&mut registry, &a, inv, 500_000, 100, &v).unwrap();
    // Replay → to_version already exists.
    let err = apply_amendment(&mut registry, &a, inv, 500_000, 100, &v).unwrap_err();
    assert!(matches!(err, AmendmentError::ToVersionExists(2)));
}

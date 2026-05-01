//! `LlsaProof` artefact + `ProofVerifier` trait.
//!
//! The on-chain proof artefact is opaque (Coq output bytes); the
//! `ProofVerifier` trait is the trust-boundary surface — production
//! supplies a Coq-bound impl, tests use the no-op or always-reject
//! impls in [`tests`] below.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 32-byte invariant identifier — typically `blake3(canonical_invariant_text)`.
pub type InvariantId = [u8; 32];

/// 32-byte amendment hash — `blake3(from_version || to_version ||
/// step_new_descriptor)`. Binds the proof to the specific amendment
/// it claims to validate.
pub type AmendmentHash = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlsaProof {
    /// Hash of the Coq term (the kernel's witness that the term
    /// type-checks against the expected goal). Opaque.
    pub coq_term_hash: [u8; 32],
    /// Which invariant `Inv` the proof claims to preserve.
    pub target_invariant_id: InvariantId,
    /// Which amendment this proof is bound to.
    pub bound_amendment_hash: AmendmentHash,
    /// Opaque proof bytes (the Coq kernel's certificate). Production
    /// verifiers re-derive these against the supplied Coq term.
    pub proof_bytes: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProofError {
    #[error("proof targets invariant {got:?} but the chain expects {expected:?}")]
    WrongInvariant {
        got: InvariantId,
        expected: InvariantId,
    },
    #[error("proof bound to amendment {got:?} but supplied for amendment {expected:?}")]
    WrongAmendment {
        got: AmendmentHash,
        expected: AmendmentHash,
    },
    #[error("verifier rejected proof: {0}")]
    VerifierRejected(String),
}

/// Trust-boundary surface. Production supplies a Coq-bound impl
/// (extracted-to-Rust MetaCoq kernel); tests use the no-op or always-
/// reject impls in this module's tests.
pub trait ProofVerifier {
    /// Verify the proof artefact. Returns `Ok(())` iff the proof's
    /// kernel certificate is valid AND the proof binds to the
    /// supplied (invariant, amendment) pair.
    fn verify(
        &self,
        proof: &LlsaProof,
        expected_invariant: InvariantId,
        expected_amendment: AmendmentHash,
    ) -> Result<(), ProofError>;
}

/// No-op verifier — accepts every proof whose binding matches.
/// **For testing only.** Production must use a Coq-bound impl.
#[derive(Default)]
pub struct AlwaysAcceptVerifier;

impl ProofVerifier for AlwaysAcceptVerifier {
    fn verify(
        &self,
        proof: &LlsaProof,
        expected_invariant: InvariantId,
        expected_amendment: AmendmentHash,
    ) -> Result<(), ProofError> {
        if proof.target_invariant_id != expected_invariant {
            return Err(ProofError::WrongInvariant {
                got: proof.target_invariant_id,
                expected: expected_invariant,
            });
        }
        if proof.bound_amendment_hash != expected_amendment {
            return Err(ProofError::WrongAmendment {
                got: proof.bound_amendment_hash,
                expected: expected_amendment,
            });
        }
        Ok(())
    }
}

/// Always-reject verifier. **For testing only** — exercises the
/// rejection path without needing a Coq install.
#[derive(Default)]
pub struct AlwaysRejectVerifier;

impl ProofVerifier for AlwaysRejectVerifier {
    fn verify(
        &self,
        _proof: &LlsaProof,
        _expected_invariant: InvariantId,
        _expected_amendment: AmendmentHash,
    ) -> Result<(), ProofError> {
        Err(ProofError::VerifierRejected(
            "AlwaysRejectVerifier".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proof(inv: InvariantId, amend: AmendmentHash) -> LlsaProof {
        LlsaProof {
            coq_term_hash: [0u8; 32],
            target_invariant_id: inv,
            bound_amendment_hash: amend,
            proof_bytes: vec![],
        }
    }

    #[test]
    fn accepts_matching_invariant_and_amendment() {
        let v = AlwaysAcceptVerifier;
        let p = proof([1u8; 32], [2u8; 32]);
        assert!(v.verify(&p, [1u8; 32], [2u8; 32]).is_ok());
    }

    #[test]
    fn rejects_wrong_invariant() {
        let v = AlwaysAcceptVerifier;
        let p = proof([1u8; 32], [2u8; 32]);
        let err = v.verify(&p, [9u8; 32], [2u8; 32]).unwrap_err();
        assert!(matches!(err, ProofError::WrongInvariant { .. }));
    }

    #[test]
    fn rejects_wrong_amendment() {
        let v = AlwaysAcceptVerifier;
        let p = proof([1u8; 32], [2u8; 32]);
        let err = v.verify(&p, [1u8; 32], [9u8; 32]).unwrap_err();
        assert!(matches!(err, ProofError::WrongAmendment { .. }));
    }

    #[test]
    fn always_reject_rejects() {
        let v = AlwaysRejectVerifier;
        let p = proof([1u8; 32], [2u8; 32]);
        let err = v.verify(&p, [1u8; 32], [2u8; 32]).unwrap_err();
        assert!(matches!(err, ProofError::VerifierRejected(_)));
    }
}

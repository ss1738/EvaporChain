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

/// Layer 7 (LLSA descope) of `DOCTRINE_PUNCH_LIST.md` — k-of-n
/// multi-auditor threshold verifier. Wraps multiple
/// `ProofVerifier` implementations; accepts iff at least `k` of
/// the inner verifiers accept.
///
/// **Use case:** the chain's production verifier is gated on a
/// Coq build (manual M2). Until that lands, the descope path uses
/// k-of-n auditor signatures: each auditor independently runs
/// their preferred verifier, and the chain accepts the proof iff
/// `k` auditors agree. This is the substrate the chain's auditor-
/// multisig wraps; the `ProofVerifier` impls plugged in here are
/// typically auditor-specific signature checkers, not the
/// per-proof Coq kernel.
///
/// The threshold contract: `0 < k <= n` where `n = verifiers.len()`.
/// `k = 1` is "any verifier accepts"; `k = n` is "every verifier
/// accepts" (the unanimous case). Construction with `k = 0` or
/// `k > n` is rejected at the caller.
pub struct MultiAuditorVerifier {
    verifiers: Vec<Box<dyn ProofVerifier + Send + Sync>>,
    threshold: usize,
}

impl MultiAuditorVerifier {
    /// Construct a k-of-n verifier. `threshold` is `k`; the inner
    /// `verifiers` Vec defines `n`. Returns `None` if the threshold
    /// is 0 or exceeds the number of verifiers (caller must
    /// validate before reaching consensus).
    pub fn new(
        verifiers: Vec<Box<dyn ProofVerifier + Send + Sync>>,
        threshold: usize,
    ) -> Option<Self> {
        if threshold == 0 || threshold > verifiers.len() {
            return None;
        }
        Some(Self {
            verifiers,
            threshold,
        })
    }

    /// `k` — the threshold.
    pub fn threshold(&self) -> usize {
        self.threshold
    }

    /// `n` — the number of verifiers.
    pub fn n(&self) -> usize {
        self.verifiers.len()
    }
}

impl ProofVerifier for MultiAuditorVerifier {
    fn verify(
        &self,
        proof: &LlsaProof,
        expected_invariant: InvariantId,
        expected_amendment: AmendmentHash,
    ) -> Result<(), ProofError> {
        let mut accepts = 0usize;
        let mut last_rejection: Option<ProofError> = None;
        for v in &self.verifiers {
            match v.verify(proof, expected_invariant, expected_amendment) {
                Ok(()) => {
                    accepts += 1;
                    if accepts >= self.threshold {
                        return Ok(());
                    }
                }
                Err(e) => {
                    last_rejection = Some(e);
                }
            }
        }
        Err(ProofError::VerifierRejected(format!(
            "k-of-n threshold not met: {} accepted, threshold={}, n={} \
             (last rejection: {:?})",
            accepts,
            self.threshold,
            self.verifiers.len(),
            last_rejection
        )))
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

    /// Layer 7 — `MultiAuditorVerifier::new` rejects degenerate
    /// thresholds.
    #[test]
    fn multi_auditor_constructor_rejects_invalid_thresholds() {
        let vs: Vec<Box<dyn ProofVerifier + Send + Sync>> =
            vec![Box::new(AlwaysAcceptVerifier), Box::new(AlwaysAcceptVerifier)];
        // threshold = 0 → None
        assert!(MultiAuditorVerifier::new(vs, 0).is_none());
        let vs: Vec<Box<dyn ProofVerifier + Send + Sync>> =
            vec![Box::new(AlwaysAcceptVerifier)];
        // threshold > n → None
        assert!(MultiAuditorVerifier::new(vs, 2).is_none());
    }

    /// Layer 7 — k=1 unanimous accept (any verifier accepting is
    /// enough). Locks the OR-semantic.
    #[test]
    fn multi_auditor_one_of_three_accepts() {
        let v = MultiAuditorVerifier::new(
            vec![
                Box::new(AlwaysRejectVerifier),
                Box::new(AlwaysAcceptVerifier),
                Box::new(AlwaysRejectVerifier),
            ],
            1,
        )
        .expect("valid threshold");
        let p = proof([1u8; 32], [2u8; 32]);
        assert!(v.verify(&p, [1u8; 32], [2u8; 32]).is_ok());
    }

    /// Layer 7 — k=2 of 3, 2 accept → ok.
    #[test]
    fn multi_auditor_two_of_three_meets_threshold() {
        let v = MultiAuditorVerifier::new(
            vec![
                Box::new(AlwaysAcceptVerifier),
                Box::new(AlwaysRejectVerifier),
                Box::new(AlwaysAcceptVerifier),
            ],
            2,
        )
        .expect("valid threshold");
        let p = proof([1u8; 32], [2u8; 32]);
        assert!(v.verify(&p, [1u8; 32], [2u8; 32]).is_ok());
    }

    /// Layer 7 — k=2 of 3, only 1 accepts → rejected.
    #[test]
    fn multi_auditor_below_threshold_rejected() {
        let v = MultiAuditorVerifier::new(
            vec![
                Box::new(AlwaysAcceptVerifier),
                Box::new(AlwaysRejectVerifier),
                Box::new(AlwaysRejectVerifier),
            ],
            2,
        )
        .expect("valid threshold");
        let p = proof([1u8; 32], [2u8; 32]);
        let err = v.verify(&p, [1u8; 32], [2u8; 32]).unwrap_err();
        assert!(matches!(err, ProofError::VerifierRejected(_)));
    }

    /// Layer 7 — k=n unanimous case. All accept → ok; one rejects → fail.
    #[test]
    fn multi_auditor_unanimous_threshold() {
        let v = MultiAuditorVerifier::new(
            vec![Box::new(AlwaysAcceptVerifier), Box::new(AlwaysAcceptVerifier)],
            2,
        )
        .expect("valid threshold");
        let p = proof([1u8; 32], [2u8; 32]);
        assert!(v.verify(&p, [1u8; 32], [2u8; 32]).is_ok());

        let v_with_reject = MultiAuditorVerifier::new(
            vec![Box::new(AlwaysAcceptVerifier), Box::new(AlwaysRejectVerifier)],
            2,
        )
        .expect("valid threshold");
        assert!(v_with_reject.verify(&p, [1u8; 32], [2u8; 32]).is_err());
    }

    /// Layer 7 — `threshold()` and `n()` accessors return the
    /// constructed values.
    #[test]
    fn multi_auditor_accessors() {
        let v = MultiAuditorVerifier::new(
            vec![
                Box::new(AlwaysAcceptVerifier),
                Box::new(AlwaysAcceptVerifier),
                Box::new(AlwaysAcceptVerifier),
            ],
            2,
        )
        .expect("valid");
        assert_eq!(v.threshold(), 2);
        assert_eq!(v.n(), 3);
    }
}

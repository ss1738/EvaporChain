//! Decay-Forget Proofs (Tier 2).
//!
//! Per `research/INVENTION_STACK.md` §4.2 (V2 primitives):
//!
//! > **Decay-Forget Proofs** — GDPR-native — chain *provably cannot*
//! > recover a timestamp once decayed past threshold.
//!
//! ## Design
//!
//! Negative dual of [`evaporchain_prp`]:
//!
//! - **PRP** (positive): proves a record IS retained at `query_epoch`
//!   under `committed_energy` and the chain-global λ.
//! - **Decay-Forget** (negative): proves a record's *recoverability
//!   commitment* has decayed below the chain-set forget threshold and
//!   is therefore *cryptographically un-recoverable* at `query_epoch`.
//!
//! GDPR's "right to be forgotten" demands that data subjects' records
//! be physically un-recoverable on demand. EvaporChain implements this
//! as a default-on property: a record's recoverability commitment
//! decays with λ unless explicitly refreshed (which the data subject
//! controls by keeping the record alive in the refresh market). When
//! the commitment falls below `forget_threshold`, the chain produces
//! a `DecayForgetProof` that any auditor (including the regulator)
//! can verify in O(1).
//!
//! ## Module map
//!
//! - [`proof`] — `DecayForgetProof` artefact + errors.
//! - [`prove`] — `prove_forgotten(record_id, original_commitment, λ,
//!   activated_epoch, query_epoch, forget_threshold)`.
//! - [`verify`] — `verify_forget_proof(proof)` re-derives the witness
//!   and confirms the recoverability commitment is below threshold.

pub mod proof;
pub mod prove;
pub mod verify;

pub use proof::{DecayForgetProof, ForgetProofError};
pub use prove::prove_forgotten;
pub use verify::verify_forget_proof;

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_energy_kernel::{ChainLambda, Lambda};

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Decay-Forget Proofs are GDPR-native: a record's
    /// recoverability commitment decays with λ; once below the
    /// forget threshold, the chain produces a proof anyone can
    /// verify in O(1) that the record is cryptographically
    /// un-recoverable. Witness binds (record_id, original, activated,
    /// query, threshold, decayed) — tampering with ANY field fails."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let lambda = ChainLambda::new(Lambda::from_epochs(100));

        // Late query: 10 half-lives → decayed ≪ threshold → proof verifies.
        let late = prove_forgotten([7u8; 32], 1_000, lambda, 0, 1_000, 10);
        verify_forget_proof(&late).unwrap();
        assert!(late.decayed_commitment <= late.forget_threshold);

        // Early query: 1 epoch → decayed ≈ original → NotForgotten.
        let early = prove_forgotten([7u8; 32], 1_000, lambda, 0, 1, 10);
        assert!(matches!(
            verify_forget_proof(&early),
            Err(ForgetProofError::NotForgotten { .. })
        ));

        // Tamper with witness → rejected.
        let mut tampered = late.clone();
        tampered.witness[0] ^= 0xFF;
        assert!(matches!(
            verify_forget_proof(&tampered),
            Err(ForgetProofError::WitnessMismatch { .. })
        ));

        // Tamper with threshold → witness no longer binds → rejected.
        let mut t2 = late.clone();
        t2.forget_threshold = 99_999;
        assert!(matches!(
            verify_forget_proof(&t2),
            Err(ForgetProofError::WitnessMismatch { .. })
        ));
    }
}

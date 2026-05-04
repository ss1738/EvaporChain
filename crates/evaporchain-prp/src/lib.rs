//! Provable Retention Proofs (PRP).
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 11:
//!
//! > **Provable Retention Proofs** — Positive-finality dual of #10
//! > [Evaporated-Fork Certificates]. Provable retention as a first-
//! > class operation. Regulator-survival primitive.
//!
//! ## Why this exists
//!
//! §3.2 of the doctrine names regulators as the chain's real
//! adversary. Regulators (MiCA, FATF, GDPR) sometimes mandate
//! *selective* permanent retention (e.g. of certain transaction
//! categories) — at odds with the chain's anti-feature manifesto
//! "no permanent data storage at the protocol level".
//!
//! PRP threads the needle: state is retained iff *enough energy was
//! committed to keep it alive*. A regulator who wants permanence
//! pays the energy cost; the chain produces a *proof* that the state
//! was alive at any queried epoch under the global λ.
//!
//! ## Substrate
//!
//! - [`proof`] — `RetentionProof { state_id, retained_until_epoch,
//!   committed_energy, witness }`.
//! - [`prove`] — `prove_retention(state_id, committed_energy, λ,
//!   activated_epoch)` computes the latest epoch at which the state
//!   was still above the chain-set retention floor.
//! - [`verify`] — `verify_retention_proof(proof, λ, query_epoch,
//!   floor)` returns true iff `query_epoch ≤ retained_until_epoch`
//!   AND the proof's witness re-derives correctly.

pub mod proof;
pub mod prove;
pub mod verify;

pub use proof::{RetentionProof, RetentionProofError};
pub use prove::prove_retention;
pub use verify::verify_retention_proof;

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_energy_kernel::{ChainLambda, Lambda};

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Provable Retention Proofs are the positive-
    /// finality dual of Decay-Forget proofs: a regulator who needs
    /// permanent retention pays the energy cost; the chain emits a
    /// proof that state was retained at any epoch within the window.
    /// Higher committed_energy → longer retention; higher floor →
    /// shorter retention. Tampering with any field breaks the
    /// witness; queries past the window fail closed."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let lambda = ChainLambda::new(Lambda::from_epochs(100));

        // Higher committed_energy → strictly longer retention.
        let p_low = prove_retention([0u8; 32], 1_000, lambda, 0, 10);
        let p_high = prove_retention([0u8; 32], 1_000_000, lambda, 0, 10);
        assert!(p_high.retained_until_epoch > p_low.retained_until_epoch);

        // Higher floor → shorter retention.
        let p_low_floor = prove_retention([1u8; 32], 1_000, lambda, 0, 1);
        let p_high_floor = prove_retention([1u8; 32], 1_000, lambda, 0, 500);
        assert!(p_low_floor.retained_until_epoch >= p_high_floor.retained_until_epoch);

        // Honest verification within the window succeeds.
        let mid = p_high.retained_until_epoch / 2;
        verify_retention_proof(&p_high, mid).unwrap();

        // Query past the retention window fails closed.
        assert!(matches!(
            verify_retention_proof(&p_high, p_high.retained_until_epoch + 1),
            Err(RetentionProofError::QueryAfterRetention { .. })
        ));

        // Tamper with witness → WitnessMismatch.
        let mut tampered = p_high.clone();
        tampered.witness[0] ^= 0xFF;
        assert!(matches!(
            verify_retention_proof(&tampered, mid),
            Err(RetentionProofError::WitnessMismatch { .. })
        ));

        // Tamper with committed_energy → witness no longer binds.
        let mut t2 = p_high.clone();
        t2.committed_energy = 999_999_999;
        assert!(matches!(
            verify_retention_proof(&t2, mid),
            Err(RetentionProofError::WitnessMismatch { .. })
        ));
    }
}

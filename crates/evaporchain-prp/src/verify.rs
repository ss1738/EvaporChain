//! `verify_retention_proof` — re-derives the witness and confirms
//! the queried epoch is within the retained window.

use crate::proof::{RetentionProof, RetentionProofError};
use crate::prove::compute_witness;

pub fn verify_retention_proof(proof: &RetentionProof, query_epoch: u64) -> Result<(), RetentionProofError> {
    // Re-derive witness binding.
    let derived = compute_witness(
        proof.state_id,
        proof.activated_epoch,
        proof.committed_energy,
        proof.retained_until_epoch,
    );
    if derived != proof.witness {
        return Err(RetentionProofError::WitnessMismatch {
            derived,
            claimed: proof.witness,
        });
    }
    // Query epoch must be within the retained window.
    if query_epoch > proof.retained_until_epoch {
        return Err(RetentionProofError::QueryAfterRetention {
            query: query_epoch,
            retained_until: proof.retained_until_epoch,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prove::prove_retention;
    use evaporchain_energy_kernel::{ChainLambda, Lambda};

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn happy_path_at_activation() {
        let p = prove_retention([7u8; 32], 1_000_000, lambda(), 50, 1);
        verify_retention_proof(&p, 50).unwrap();
    }

    #[test]
    fn happy_path_within_window() {
        let p = prove_retention([7u8; 32], 1_000_000, lambda(), 0, 1);
        // Pick a query epoch comfortably inside the window.
        verify_retention_proof(&p, p.retained_until_epoch / 2).unwrap();
    }

    #[test]
    fn query_after_window_rejected() {
        let p = prove_retention([7u8; 32], 1000, lambda(), 0, 100);
        let err = verify_retention_proof(&p, p.retained_until_epoch + 1).unwrap_err();
        assert!(matches!(err, RetentionProofError::QueryAfterRetention { .. }));
    }

    #[test]
    fn tampered_witness_rejected() {
        let mut p = prove_retention([7u8; 32], 1000, lambda(), 0, 100);
        p.witness[0] ^= 0xFF;
        let err = verify_retention_proof(&p, 0).unwrap_err();
        assert!(matches!(err, RetentionProofError::WitnessMismatch { .. }));
    }

    #[test]
    fn tampered_committed_energy_rejected() {
        let mut p = prove_retention([7u8; 32], 1000, lambda(), 0, 100);
        // Increase committed_energy without updating witness — re-derivation differs.
        p.committed_energy = 999_999_999;
        let err = verify_retention_proof(&p, 0).unwrap_err();
        assert!(matches!(err, RetentionProofError::WitnessMismatch { .. }));
    }
}

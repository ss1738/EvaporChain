//! `verify_forget_proof` — re-derives witness and checks the
//! decayed commitment is at-or-below `forget_threshold`.

use crate::proof::{DecayForgetProof, ForgetProofError};
use crate::prove::compute_witness;

pub fn verify_forget_proof(proof: &DecayForgetProof) -> Result<(), ForgetProofError> {
    // Witness binding.
    let derived = compute_witness(
        proof.record_id,
        proof.original_commitment,
        proof.activated_epoch,
        proof.forgotten_at_epoch,
        proof.forget_threshold,
        proof.decayed_commitment,
    );
    if derived != proof.witness {
        return Err(ForgetProofError::WitnessMismatch {
            derived,
            claimed: proof.witness,
        });
    }
    if proof.decayed_commitment > proof.forget_threshold {
        return Err(ForgetProofError::NotForgotten {
            decayed: proof.decayed_commitment,
            threshold: proof.forget_threshold,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prove::prove_forgotten;
    use evaporchain_energy_kernel::{ChainLambda, Lambda};

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn happy_path_late_query_verifies() {
        let p = prove_forgotten([7u8; 32], 1000, lambda(), 0, 1000, 10);
        verify_forget_proof(&p).unwrap();
    }

    #[test]
    fn early_query_not_forgotten() {
        let p = prove_forgotten([7u8; 32], 1000, lambda(), 0, 1, 10);
        let err = verify_forget_proof(&p).unwrap_err();
        assert!(matches!(err, ForgetProofError::NotForgotten { .. }));
    }

    #[test]
    fn tampered_witness_rejected() {
        let mut p = prove_forgotten([7u8; 32], 1000, lambda(), 0, 1000, 10);
        p.witness[0] ^= 0xFF;
        let err = verify_forget_proof(&p).unwrap_err();
        assert!(matches!(err, ForgetProofError::WitnessMismatch { .. }));
    }

    #[test]
    fn tampered_threshold_rejected() {
        let mut p = prove_forgotten([7u8; 32], 1000, lambda(), 0, 1000, 10);
        p.forget_threshold = 99_999;
        let err = verify_forget_proof(&p).unwrap_err();
        assert!(matches!(err, ForgetProofError::WitnessMismatch { .. }));
    }
}

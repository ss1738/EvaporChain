//! `verify_folded` — substrate verifier.
//!
//! Real Nova/HyperNova verification (the R1CS satisfiability check)
//! is gated on the arkworks integration in `evaporchain-proving` and
//! plugs in here as a separate commit. The substrate exposes the
//! verifier's *interface* and checks the two non-cryptographic
//! components:
//!
//! 1. `acc_hash` matches a chain-supplied expected value.
//! 2. `total_energy_remaining` is at or above a chain-supplied
//!    threshold (the chain knows what aggregate energy *should* have
//!    survived `step_count` folds at the global λ; the prover must
//!    not have over-reported decay).

use thiserror::Error;

use crate::folded::FoldedInstance;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("acc_hash {got:?} does not match expected {expected:?}")]
    AccHashMismatch { got: [u8; 32], expected: [u8; 32] },
    #[error("total_energy_remaining {got} below the chain-supplied minimum {min}")]
    EnergyBelowMinimum { got: u128, min: u128 },
}

pub fn verify_folded(
    instance: &FoldedInstance,
    expected_acc_hash: [u8; 32],
    min_remaining_energy: u128,
) -> Result<(), VerifyError> {
    if instance.acc_hash != expected_acc_hash {
        return Err(VerifyError::AccHashMismatch {
            got: instance.acc_hash,
            expected: expected_acc_hash,
        });
    }
    if instance.total_energy_remaining < min_remaining_energy {
        return Err(VerifyError::EnergyBelowMinimum {
            got: instance.total_energy_remaining,
            min: min_remaining_energy,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::fold;
    use crate::witness::StepWitness;
    use evaporchain_energy_kernel::{ChainLambda, Lambda};

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    fn folded() -> FoldedInstance {
        let s = StepWitness::new([7u8; 32], 1000, 0);
        fold(FoldedInstance::identity(), s, lambda()).unwrap()
    }

    #[test]
    fn happy_path() {
        let inst = folded();
        verify_folded(&inst, inst.acc_hash, 0).unwrap();
    }

    #[test]
    fn wrong_hash_rejected() {
        let inst = folded();
        let mut bad = inst.acc_hash;
        bad[0] ^= 0xFF;
        let err = verify_folded(&inst, bad, 0).unwrap_err();
        assert!(matches!(err, VerifyError::AccHashMismatch { .. }));
    }

    #[test]
    fn energy_below_minimum_rejected() {
        let inst = folded();
        let err = verify_folded(&inst, inst.acc_hash, 5000).unwrap_err();
        assert!(matches!(err, VerifyError::EnergyBelowMinimum { .. }));
    }
}

//! `verify_evaporated_cert` — O(1) light-client check.

use crate::cert::{CertError, EvaporatedForkCert};
use crate::prove::compute_witness;

pub fn verify_evaporated_cert(cert: &EvaporatedForkCert) -> Result<(), CertError> {
    // 1. Witness binding.
    let derived = compute_witness(
        cert.fork_root,
        cert.evaluated_at_epoch,
        cert.total_seed_energy,
        cert.decayed_energy,
        cert.threshold,
    );
    if derived != cert.witness {
        return Err(CertError::WitnessMismatch {
            derived,
            claimed: cert.witness,
        });
    }
    // 2. Decayed energy strictly below threshold.
    if cert.decayed_energy >= cert.threshold {
        return Err(CertError::NotEvaporated {
            decayed: cert.decayed_energy,
            threshold: cert.threshold,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cert::ForkBlock;
    use crate::prove::prove_fork_evaporated;
    use evaporchain_energy_kernel::{ChainLambda, Lambda};

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn happy_path_verifies() {
        // Single block, one halving → decayed=500. Threshold=600 → evaporated.
        let blocks = [ForkBlock {
            seed_energy: 1000,
            observed_epoch: 0,
        }];
        let c = prove_fork_evaporated([7u8; 32], &blocks, lambda(), 100, 600);
        verify_evaporated_cert(&c).unwrap();
    }

    #[test]
    fn rejected_when_not_evaporated() {
        // decayed=500 but threshold=400 → fork still has enough → cert not valid.
        let blocks = [ForkBlock {
            seed_energy: 1000,
            observed_epoch: 0,
        }];
        let c = prove_fork_evaporated([7u8; 32], &blocks, lambda(), 100, 400);
        let err = verify_evaporated_cert(&c).unwrap_err();
        assert!(matches!(err, CertError::NotEvaporated { .. }));
    }

    #[test]
    fn tampered_threshold_rejected() {
        let blocks = [ForkBlock {
            seed_energy: 1000,
            observed_epoch: 0,
        }];
        let mut c = prove_fork_evaporated([7u8; 32], &blocks, lambda(), 100, 600);
        // Change threshold without recomputing witness.
        c.threshold = 1_000_000;
        let err = verify_evaporated_cert(&c).unwrap_err();
        assert!(matches!(err, CertError::WitnessMismatch { .. }));
    }

    #[test]
    fn tampered_decayed_rejected() {
        let blocks = [ForkBlock {
            seed_energy: 1000,
            observed_epoch: 0,
        }];
        let mut c = prove_fork_evaporated([7u8; 32], &blocks, lambda(), 100, 600);
        c.decayed_energy = 1; // pretend the fork decayed even more
        let err = verify_evaporated_cert(&c).unwrap_err();
        assert!(matches!(err, CertError::WitnessMismatch { .. }));
    }
}

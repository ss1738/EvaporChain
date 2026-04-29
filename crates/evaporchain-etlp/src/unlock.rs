//! `can_unlock` — chain-side gate.

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};

use crate::capsule::Capsule;
use crate::witness::{EnergyWitness, WitnessError};

/// True iff:
/// - witness binding matches the capsule, AND
/// - witness's λ-decayed remaining energy at `current_epoch` is at
///   or above `capsule.energy_threshold`.
pub fn can_unlock(
    capsule: &Capsule,
    witness: &EnergyWitness,
    chain_lambda: ChainLambda,
    current_epoch: u64,
) -> Result<bool, WitnessError> {
    // Binding check.
    let expected = EnergyWitness::compute_binding(
        capsule.seal_epoch,
        capsule.energy_threshold,
        witness.committed_energy,
        witness.observed_epoch,
    );
    if expected != witness.binding {
        return Err(WitnessError::BindingMismatch);
    }
    // Decayed-energy check.
    let elapsed = current_epoch.saturating_sub(witness.observed_epoch);
    let remaining = energy_at_epoch(witness.committed_energy, chain_lambda.half_life(), elapsed);
    Ok(remaining >= capsule.energy_threshold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    fn capsule() -> Capsule {
        Capsule::new(0, 500, vec![0xAA; 16]).unwrap()
    }

    fn witness(committed: u64, observed: u64, capsule: &Capsule) -> EnergyWitness {
        let binding = EnergyWitness::compute_binding(
            capsule.seal_epoch,
            capsule.energy_threshold,
            committed,
            observed,
        );
        EnergyWitness { committed_energy: committed, observed_epoch: observed, binding }
    }

    #[test]
    fn enough_committed_unlocks() {
        let c = capsule();
        let w = witness(1000, 0, &c);
        assert!(can_unlock(&c, &w, lambda(), 0).unwrap());
    }

    #[test]
    fn decayed_below_threshold_does_not_unlock() {
        let c = capsule();
        let w = witness(1000, 0, &c);
        // After 200 epochs at half_life=100 → 1000 → 250, below threshold 500.
        assert!(!can_unlock(&c, &w, lambda(), 200).unwrap());
    }

    #[test]
    fn binding_mismatch_rejected() {
        let c = capsule();
        let mut w = witness(1000, 0, &c);
        w.binding[0] ^= 0xFF;
        let err = can_unlock(&c, &w, lambda(), 0).unwrap_err();
        assert_eq!(err, WitnessError::BindingMismatch);
    }
}

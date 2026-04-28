//! `fold` — accumulate one `StepWitness` into a `FoldedInstance`.
//!
//! Substrate composition:
//!
//! 1. Decay every previously-folded energy from `prev.latest_epoch` to
//!    `step.observed_epoch` via the chain-global λ.
//! 2. Add the step's own energy.
//! 3. Update `acc_hash = blake3("lambda-fold" || prev.acc_hash ||
//!    step.state_hash)`.
//! 4. `step_count += 1`; `latest_epoch = step.observed_epoch`.
//!
//! `step.observed_epoch` must be ≥ `prev.latest_epoch` (no out-of-
//! order folding); otherwise we error.

use thiserror::Error;

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};

use crate::folded::FoldedInstance;
use crate::witness::StepWitness;

const FOLD_TAG: &[u8] = b"lambda-fold";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FoldError {
    #[error(
        "step observed_epoch {step} is before prev latest_epoch {prev} — \
         folds must be monotone in time"
    )]
    OutOfOrder { step: u64, prev: u64 },
}

pub fn fold(
    prev: FoldedInstance,
    step: StepWitness,
    chain_lambda: ChainLambda,
) -> Result<FoldedInstance, FoldError> {
    if step.observed_epoch < prev.latest_epoch {
        return Err(FoldError::OutOfOrder {
            step: step.observed_epoch,
            prev: prev.latest_epoch,
        });
    }
    let elapsed = step.observed_epoch - prev.latest_epoch;
    // Decay the previously-folded energy forward to the step's epoch.
    let decayed_prev = if elapsed == 0 || prev.total_energy_remaining == 0 {
        prev.total_energy_remaining
    } else {
        // u128 → u64 for the decay primitive, saturating.
        let prev_u64 = prev.total_energy_remaining.min(u64::MAX as u128) as u64;
        let decayed = energy_at_epoch(prev_u64, chain_lambda.half_life(), elapsed);
        decayed as u128
    };
    let new_total = decayed_prev.saturating_add(step.step_energy as u128);

    // Hash chain.
    let mut h = blake3::Hasher::new();
    h.update(FOLD_TAG);
    h.update(&prev.acc_hash);
    h.update(&step.state_hash);
    h.update(&step.step_energy.to_le_bytes());
    h.update(&step.observed_epoch.to_le_bytes());
    let acc_hash = *h.finalize().as_bytes();

    Ok(FoldedInstance {
        acc_hash,
        total_energy_remaining: new_total,
        step_count: prev.step_count.saturating_add(1),
        latest_epoch: step.observed_epoch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn first_fold_from_identity() {
        let i = FoldedInstance::identity();
        let step = StepWitness::new([1u8; 32], 1000, 5);
        let out = fold(i, step, lambda()).unwrap();
        assert_eq!(out.step_count, 1);
        assert_eq!(out.latest_epoch, 5);
        assert_eq!(out.total_energy_remaining, 1000);
        assert_ne!(out.acc_hash, [0u8; 32]);
    }

    #[test]
    fn out_of_order_fold_rejected() {
        let i = FoldedInstance {
            acc_hash: [0u8; 32],
            total_energy_remaining: 0,
            step_count: 1,
            latest_epoch: 50,
        };
        let earlier = StepWitness::new([1u8; 32], 100, 40);
        let err = fold(i, earlier, lambda()).unwrap_err();
        assert_eq!(err, FoldError::OutOfOrder { step: 40, prev: 50 });
    }

    #[test]
    fn second_fold_decays_prev_energy() {
        let i = FoldedInstance::identity();
        let step1 = StepWitness::new([1u8; 32], 1000, 0);
        let after_1 = fold(i, step1, lambda()).unwrap();
        let step2 = StepWitness::new([2u8; 32], 1000, 100);
        let after_2 = fold(after_1, step2, lambda()).unwrap();
        // After 100 epochs at half_life=100 → previous 1000 halved to 500.
        // Plus the new step's 1000 → total ≈ 1500.
        assert_eq!(after_2.total_energy_remaining, 1500);
        assert_eq!(after_2.step_count, 2);
        assert_eq!(after_2.latest_epoch, 100);
    }

    #[test]
    fn determinism_under_same_inputs() {
        let i = FoldedInstance::identity();
        let s = StepWitness::new([7u8; 32], 500, 3);
        let a = fold(i, s, lambda()).unwrap();
        let b = fold(i, s, lambda()).unwrap();
        assert_eq!(a, b);
    }
}

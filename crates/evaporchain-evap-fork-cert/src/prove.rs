//! `prove_fork_evaporated` — aggregate the fork's λ-decayed energy
//! at `evaluated_at_epoch` and bind the resulting certificate.

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};

use crate::cert::{EvaporatedForkCert, ForkBlock};

const WITNESS_TAG: &[u8] = b"evaporchain-evap-fork-cert";

pub fn prove_fork_evaporated(
    fork_root: [u8; 32],
    blocks: &[ForkBlock],
    chain_lambda: ChainLambda,
    evaluated_at_epoch: u64,
    threshold: u128,
) -> EvaporatedForkCert {
    let mut total_seed: u128 = 0;
    let mut decayed: u128 = 0;
    for b in blocks {
        total_seed = total_seed.saturating_add(b.seed_energy as u128);
        if evaluated_at_epoch < b.observed_epoch {
            // Block hasn't been observed yet at the eval epoch — skip.
            continue;
        }
        let elapsed = evaluated_at_epoch - b.observed_epoch;
        let r = energy_at_epoch(b.seed_energy, chain_lambda.half_life(), elapsed) as u128;
        decayed = decayed.saturating_add(r);
    }
    let witness = compute_witness(
        fork_root,
        evaluated_at_epoch,
        total_seed,
        decayed,
        threshold,
    );
    EvaporatedForkCert {
        fork_root,
        evaluated_at_epoch,
        total_seed_energy: total_seed,
        decayed_energy: decayed,
        threshold,
        witness,
    }
}

pub(crate) fn compute_witness(
    fork_root: [u8; 32],
    evaluated_at_epoch: u64,
    total_seed: u128,
    decayed: u128,
    threshold: u128,
) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(WITNESS_TAG);
    h.update(&fork_root);
    h.update(&evaluated_at_epoch.to_le_bytes());
    h.update(&total_seed.to_le_bytes());
    h.update(&decayed.to_le_bytes());
    h.update(&threshold.to_le_bytes());
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn empty_fork_decayed_zero() {
        let c = prove_fork_evaporated([0u8; 32], &[], lambda(), 1000, 100);
        assert_eq!(c.decayed_energy, 0);
        assert_eq!(c.total_seed_energy, 0);
    }

    #[test]
    fn single_block_decay_at_one_halflife() {
        let blocks = [ForkBlock {
            seed_energy: 1000,
            observed_epoch: 0,
        }];
        let c = prove_fork_evaporated([1u8; 32], &blocks, lambda(), 100, 1);
        assert_eq!(c.total_seed_energy, 1000);
        assert_eq!(c.decayed_energy, 500); // one halving
    }

    #[test]
    fn two_blocks_decay_aggregates() {
        let blocks = [
            ForkBlock {
                seed_energy: 1000,
                observed_epoch: 0,
            },
            ForkBlock {
                seed_energy: 500,
                observed_epoch: 50,
            },
        ];
        let c = prove_fork_evaporated([0u8; 32], &blocks, lambda(), 100, 1);
        // Block 0: elapsed=100, decayed = 500.
        // Block 1: elapsed=50, half-decay halfway → ~375 (linear interp).
        assert!(c.decayed_energy > 500);
        assert!(c.decayed_energy < 1000);
    }

    /// T1.20 — block whose observed_epoch is past the eval_epoch
    /// is skipped (line 23 — the future-block continue branch).
    ///  still counts it but  doesn't.
    #[test]
    fn t1_20_future_observed_block_skipped() {
        let blocks = [
            ForkBlock {
                seed_energy: 1000,
                observed_epoch: 0,
            },
            ForkBlock {
                seed_energy: 500,
                observed_epoch: 999, // far in the future of eval_epoch=100
            },
        ];
        let c = prove_fork_evaporated([1u8; 32], &blocks, lambda(), 100, 1);
        // Both blocks contribute to total_seed.
        assert_eq!(c.total_seed_energy, 1500);
        // Only the first block contributes to decayed (second is skipped).
        // First block: seed=1000, elapsed=100 → half-life 100 → 500.
        assert_eq!(c.decayed_energy, 500);
    }
}

//! V2 prover — reuses V1 decay aggregation, adds anchor binding.

use blake3::Hasher;
use evaporchain_energy_kernel::ChainLambda;
use evaporchain_evap_fork_cert::{prove_fork_evaporated, ForkBlock};

use crate::cert::{CertV2Error, EvaporatedForkCertV2};

const WITNESS_DOMAIN: &[u8] = b"evaporchain:evap-fork-cert:v2:witness\0";

/// Build a V2 certificate. Returns `Err(CausalityViolation)` if
/// `seed_anchor_epoch > evaluated_at_epoch` (a future seed cannot
/// witness a past evaporation claim).
pub fn prove_fork_evaporated_v2(
    fork_root: [u8; 32],
    blocks: &[ForkBlock],
    chain_lambda: ChainLambda,
    evaluated_at_epoch: u64,
    threshold: u128,
    bell_seed_anchor: [u8; 32],
    seed_anchor_epoch: u64,
) -> Result<EvaporatedForkCertV2, CertV2Error> {
    if seed_anchor_epoch > evaluated_at_epoch {
        return Err(CertV2Error::CausalityViolation {
            anchor: seed_anchor_epoch,
            eval: evaluated_at_epoch,
        });
    }

    // Reuse V1 decay aggregation; ignore V1's witness (V2 derives its own).
    let v1 = prove_fork_evaporated(
        fork_root,
        blocks,
        chain_lambda,
        evaluated_at_epoch,
        threshold,
    );

    let witness = compute_witness_v2(
        fork_root,
        evaluated_at_epoch,
        v1.total_seed_energy,
        v1.decayed_energy,
        threshold,
        &bell_seed_anchor,
        seed_anchor_epoch,
    );

    Ok(EvaporatedForkCertV2 {
        fork_root,
        evaluated_at_epoch,
        total_seed_energy: v1.total_seed_energy,
        decayed_energy: v1.decayed_energy,
        threshold,
        bell_seed_anchor,
        seed_anchor_epoch,
        witness,
    })
}

pub(crate) fn compute_witness_v2(
    fork_root: [u8; 32],
    evaluated_at_epoch: u64,
    total_seed: u128,
    decayed: u128,
    threshold: u128,
    bell_seed_anchor: &[u8; 32],
    seed_anchor_epoch: u64,
) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(WITNESS_DOMAIN);
    h.update(&fork_root);
    h.update(&evaluated_at_epoch.to_le_bytes());
    h.update(&total_seed.to_le_bytes());
    h.update(&decayed.to_le_bytes());
    h.update(&threshold.to_le_bytes());
    h.update(bell_seed_anchor);
    h.update(&seed_anchor_epoch.to_le_bytes());
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    fn one_block_at_one_halflife() -> Vec<ForkBlock> {
        vec![ForkBlock {
            seed_energy: 1000,
            observed_epoch: 0,
        }]
    }

    #[test]
    fn rejects_causality_violation() {
        let err = prove_fork_evaporated_v2(
            [1u8; 32],
            &one_block_at_one_halflife(),
            lambda(),
            100, // eval epoch
            1000,
            [9u8; 32],
            200, // anchor epoch > eval epoch — causality violation
        )
        .unwrap_err();
        assert_eq!(
            err,
            CertV2Error::CausalityViolation {
                anchor: 200,
                eval: 100,
            }
        );
    }

    #[test]
    fn accepts_anchor_at_or_before_eval_epoch() {
        for anchor_epoch in [0u64, 50, 100] {
            let r = prove_fork_evaporated_v2(
                [1u8; 32],
                &one_block_at_one_halflife(),
                lambda(),
                100,
                1000,
                [9u8; 32],
                anchor_epoch,
            );
            assert!(r.is_ok(), "anchor_epoch={anchor_epoch} should accept");
        }
    }

    #[test]
    fn includes_decayed_energy_from_v1() {
        let cert = prove_fork_evaporated_v2(
            [1u8; 32],
            &one_block_at_one_halflife(),
            lambda(),
            100,
            1,
            [9u8; 32],
            0,
        )
        .unwrap();
        // V1 says one halving: 1000 → 500.
        assert_eq!(cert.decayed_energy, 500);
        assert_eq!(cert.total_seed_energy, 1000);
    }

    #[test]
    fn different_anchor_yields_different_witness() {
        let blocks = one_block_at_one_halflife();
        let a =
            prove_fork_evaporated_v2([1u8; 32], &blocks, lambda(), 100, 1, [1u8; 32], 0).unwrap();
        let b =
            prove_fork_evaporated_v2([1u8; 32], &blocks, lambda(), 100, 1, [2u8; 32], 0).unwrap();
        assert_ne!(a.witness, b.witness);
    }

    #[test]
    fn different_anchor_epoch_yields_different_witness() {
        let blocks = one_block_at_one_halflife();
        let a =
            prove_fork_evaporated_v2([1u8; 32], &blocks, lambda(), 100, 1, [9u8; 32], 0).unwrap();
        let b =
            prove_fork_evaporated_v2([1u8; 32], &blocks, lambda(), 100, 1, [9u8; 32], 50).unwrap();
        assert_ne!(a.witness, b.witness);
    }

    #[test]
    fn determinism_same_inputs_same_witness() {
        let blocks = one_block_at_one_halflife();
        let a =
            prove_fork_evaporated_v2([1u8; 32], &blocks, lambda(), 100, 1, [9u8; 32], 50).unwrap();
        let b =
            prove_fork_evaporated_v2([1u8; 32], &blocks, lambda(), 100, 1, [9u8; 32], 50).unwrap();
        assert_eq!(a, b);
    }
}

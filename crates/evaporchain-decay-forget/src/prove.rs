//! `prove_forgotten` — compute the decayed commitment at the queried
//! epoch and bind a `DecayForgetProof`.

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};
use evaporchain_types::Energy;

use crate::proof::{DecayForgetProof, RecordId};

const WITNESS_TAG: &[u8] = b"evaporchain-decay-forget";

pub fn prove_forgotten(
    record_id: RecordId,
    original_commitment: Energy,
    chain_lambda: ChainLambda,
    activated_epoch: u64,
    query_epoch: u64,
    forget_threshold: Energy,
) -> DecayForgetProof {
    let elapsed = query_epoch.saturating_sub(activated_epoch);
    let decayed = energy_at_epoch(original_commitment, chain_lambda.half_life(), elapsed);
    let witness = compute_witness(
        record_id,
        original_commitment,
        activated_epoch,
        query_epoch,
        forget_threshold,
        decayed,
    );
    DecayForgetProof {
        record_id,
        original_commitment,
        activated_epoch,
        forgotten_at_epoch: query_epoch,
        forget_threshold,
        decayed_commitment: decayed,
        witness,
    }
}

pub(crate) fn compute_witness(
    record_id: RecordId,
    original_commitment: Energy,
    activated_epoch: u64,
    query_epoch: u64,
    forget_threshold: Energy,
    decayed: Energy,
) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(WITNESS_TAG);
    h.update(&record_id);
    h.update(&original_commitment.to_le_bytes());
    h.update(&activated_epoch.to_le_bytes());
    h.update(&query_epoch.to_le_bytes());
    h.update(&forget_threshold.to_le_bytes());
    h.update(&decayed.to_le_bytes());
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
    fn proof_carries_input_fields() {
        let p = prove_forgotten([1u8; 32], 1000, lambda(), 0, 1000, 10);
        assert_eq!(p.record_id, [1u8; 32]);
        assert_eq!(p.original_commitment, 1000);
        assert_eq!(p.activated_epoch, 0);
        assert_eq!(p.forgotten_at_epoch, 1000);
        assert_eq!(p.forget_threshold, 10);
    }

    #[test]
    fn early_query_decayed_close_to_original() {
        let p = prove_forgotten([0u8; 32], 1000, lambda(), 0, 1, 10);
        // 1 epoch of decay at half_life=100 → still close to 1000.
        assert!(p.decayed_commitment > 990);
    }

    #[test]
    fn late_query_decayed_below_threshold() {
        let p = prove_forgotten([0u8; 32], 1000, lambda(), 0, 1000, 10);
        // 10 halvings → effectively 0.
        assert!(p.decayed_commitment <= 10);
    }
}

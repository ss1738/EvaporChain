//! `prove_retention` — compute the latest epoch at which the
//! committed energy is still above the chain-set retention floor.

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};
use evaporchain_types::Energy;

use crate::proof::{RetentionProof, StateId};

const WITNESS_TAG: &[u8] = b"evaporchain-prp";

/// Compute and bind a `RetentionProof`.
///
/// The latest retention epoch is the smallest `t` such that
/// `energy_at_epoch(committed_energy, λ, t - activated_epoch) <= floor`,
/// minus one. Substrate finds it by binary search over the epoch
/// axis, bounded by 64 halvings (= effective decay-to-zero).
pub fn prove_retention(
    state_id: StateId,
    committed_energy: Energy,
    chain_lambda: ChainLambda,
    activated_epoch: u64,
    floor: Energy,
) -> RetentionProof {
    // Find the largest `elapsed` such that
    // energy_at_epoch(committed, λ, elapsed) > floor.
    let half_life = chain_lambda.half_life();
    let max_search_epochs: u64 = if half_life == 0 {
        0
    } else {
        // 64 halvings is the effective decay-to-zero in the Rust impl.
        half_life.saturating_mul(64)
    };
    let mut lo: u64 = 0;
    let mut hi: u64 = max_search_epochs;
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        let remaining = energy_at_epoch(committed_energy, half_life, mid);
        if remaining > floor {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let retained_until_epoch = activated_epoch.saturating_add(lo);

    let witness = compute_witness(state_id, activated_epoch, committed_energy, retained_until_epoch);

    RetentionProof {
        state_id,
        activated_epoch,
        committed_energy,
        retained_until_epoch,
        witness,
    }
}

/// `blake3("evaporchain-prp" || state_id || activated_epoch ||
/// committed_energy || retained_until_epoch)`.
pub(crate) fn compute_witness(
    state_id: StateId,
    activated_epoch: u64,
    committed_energy: Energy,
    retained_until_epoch: u64,
) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(WITNESS_TAG);
    h.update(&state_id);
    h.update(&activated_epoch.to_le_bytes());
    h.update(&committed_energy.to_le_bytes());
    h.update(&retained_until_epoch.to_le_bytes());
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
        let p = prove_retention([1u8; 32], 1000, lambda(), 50, 100);
        assert_eq!(p.state_id, [1u8; 32]);
        assert_eq!(p.activated_epoch, 50);
        assert_eq!(p.committed_energy, 1000);
        assert!(p.retained_until_epoch >= 50);
    }

    #[test]
    fn higher_committed_energy_extends_retention() {
        let p_low = prove_retention([0u8; 32], 1000, lambda(), 0, 10);
        let p_high = prove_retention([0u8; 32], 1_000_000, lambda(), 0, 10);
        assert!(p_high.retained_until_epoch > p_low.retained_until_epoch);
    }

    #[test]
    fn higher_floor_shortens_retention() {
        let p_low_floor = prove_retention([0u8; 32], 1000, lambda(), 0, 1);
        let p_high_floor = prove_retention([0u8; 32], 1000, lambda(), 0, 500);
        assert!(p_low_floor.retained_until_epoch >= p_high_floor.retained_until_epoch);
    }
}

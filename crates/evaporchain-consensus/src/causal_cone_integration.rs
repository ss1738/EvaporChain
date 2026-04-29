//! Causal-Cone integration — wires `evaporchain-causal-cone` into the
//! consensus layer.
//!
//! Per INVENTION_STACK.md §A1.3 (Shalizi-Crutchfield Optimal Prediction
//! Theorem) each validator can attach a constant-size `CausalConeSummary`
//! to its prevote/precommit messages in place of a full ancestor header chain.
//! Light clients reconstruct sufficient statistics from this summary alone.
//!
//! # Conservation note
//!
//! `canonical_cone_hash` is deterministic: two validators at the same head and
//! epoch must produce identical hashes.  `summaries_agree` checks this to
//! detect DAG-state divergence before precommit without exchanging full header
//! sets.

use evaporchain_causal_cone::{summarize_cone, CausalConeSummary};
use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_light_cone::{BlockId, LightCone};
use tracing::warn;

/// Build the Shalizi-Crutchfield Causal-Cone summary for `head` at `epoch`.
///
/// Returns `None` if `head` is absent from `lc` or summary construction fails
/// (best-effort — consensus proceeds without the summary).
pub fn validator_cone_summary(
    lc: &LightCone,
    head: BlockId,
    chain_lambda_half_life: u64,
    epoch: u64,
) -> Option<CausalConeSummary> {
    let cl = ChainLambda::new(Lambda::from_epochs(chain_lambda_half_life.max(1)));
    match summarize_cone(head, lc, cl, epoch) {
        Ok(s) => Some(s),
        Err(e) => {
            warn!(
                head = hex::encode(head),
                err = %e,
                "causal-cone summary failed (best-effort)"
            );
            None
        }
    }
}

/// True iff two summaries share the same `canonical_cone_hash`.
///
/// Used by consensus to detect DAG-state divergence between validators
/// before precommit — mismatched hashes indicate a fork that has not yet
/// converged.
pub fn summaries_agree(a: &CausalConeSummary, b: &CausalConeSummary) -> bool {
    a.canonical_cone_hash == b.canonical_cone_hash
}

/// Log summary fields at DEBUG level for observability.  Called when attaching
/// a summary to a prevote/precommit message.
pub fn log_cone_summary(summary: &CausalConeSummary, validator_id: u64) {
    tracing::debug!(
        validator_id,
        head = hex::encode(summary.head_id),
        ancestor_count = summary.ancestor_count,
        total_remaining_energy = summary.total_remaining_energy,
        oldest_epoch = summary.oldest_observed_epoch,
        latest_epoch = summary.latest_observed_epoch,
        cone_hash = hex::encode(summary.canonical_cone_hash),
        "causal-cone summary attached to vote"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_light_cone::Block;

    fn id(b: u8) -> BlockId {
        [b; 32]
    }

    fn linear_lc() -> LightCone {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(1)], 800, 2)).unwrap();
        lc
    }

    #[test]
    fn absent_head_returns_none() {
        let lc = linear_lc();
        assert!(validator_cone_summary(&lc, id(99), 4096, 5).is_none());
    }

    #[test]
    fn present_head_returns_some() {
        let lc = linear_lc();
        let s = validator_cone_summary(&lc, id(2), 4096, 5);
        assert!(s.is_some());
    }

    #[test]
    fn same_head_same_epoch_deterministic_hash() {
        let lc = linear_lc();
        let a = validator_cone_summary(&lc, id(2), 4096, 5).unwrap();
        let b = validator_cone_summary(&lc, id(2), 4096, 5).unwrap();
        assert!(summaries_agree(&a, &b));
    }

    #[test]
    fn different_heads_different_hash() {
        let lc = linear_lc();
        let a = validator_cone_summary(&lc, id(1), 4096, 5).unwrap();
        let b = validator_cone_summary(&lc, id(2), 4096, 5).unwrap();
        // id(2) has strictly more ancestors than id(1) — cones differ.
        assert!(!summaries_agree(&a, &b));
    }

    #[test]
    fn genesis_summary_has_zero_ancestors() {
        let lc = linear_lc();
        let s = validator_cone_summary(&lc, id(0), 4096, 0).unwrap();
        assert_eq!(s.ancestor_count, 0);
    }

    #[test]
    fn summary_ancestor_count_grows_with_chain() {
        let lc = linear_lc();
        let s0 = validator_cone_summary(&lc, id(0), 4096, 5).unwrap();
        let s2 = validator_cone_summary(&lc, id(2), 4096, 5).unwrap();
        assert!(s2.ancestor_count > s0.ancestor_count);
    }
}

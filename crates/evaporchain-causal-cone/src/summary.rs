//! `CausalConeSummary` — the constant-size light-cone sufficient
//! statistic. Built by `summarize_cone(head, lc, λ, t)`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};
use evaporchain_light_cone::{causal_past, BlockId, LightCone};

use crate::canonical::canonical_cone_hash;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CausalConeSummary {
    pub head_id: BlockId,
    /// Number of distinct ancestors (excluding the head itself).
    pub ancestor_count: u64,
    /// Sum of *λ-decayed-to-`t`* remaining energies of every ancestor.
    pub total_remaining_energy: u128,
    /// Earliest `observed_epoch` across the head + ancestors.
    pub oldest_observed_epoch: u64,
    /// Latest `observed_epoch` across the head + ancestors.
    pub latest_observed_epoch: u64,
    /// blake3 hash of the canonical (sorted) ancestor id set.
    pub canonical_cone_hash: [u8; 32],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SummaryError {
    #[error("head block {0:?} is absent from the LightCone")]
    AbsentHead(BlockId),
}

/// Build the constant-size sufficient statistic for the causal past
/// of `head`, observed at chain time `t` under the chain-global λ.
pub fn summarize_cone(
    head: BlockId,
    lc: &LightCone,
    chain_lambda: ChainLambda,
    t: u64,
) -> Result<CausalConeSummary, SummaryError> {
    let head_block = lc.get(&head).ok_or(SummaryError::AbsentHead(head))?;
    let ancestors = causal_past(lc, head);
    let mut oldest = head_block.observed_epoch;
    let mut latest = head_block.observed_epoch;
    let mut total_remaining: u128 = 0;
    for ancestor_id in &ancestors {
        if let Some(b) = lc.get(ancestor_id) {
            oldest = oldest.min(b.observed_epoch);
            latest = latest.max(b.observed_epoch);
            let elapsed = t.saturating_sub(b.observed_epoch);
            let r = energy_at_epoch(b.energy, chain_lambda.half_life(), elapsed) as u128;
            total_remaining = total_remaining.saturating_add(r);
        }
    }
    Ok(CausalConeSummary {
        head_id: head,
        ancestor_count: ancestors.len() as u64,
        total_remaining_energy: total_remaining,
        oldest_observed_epoch: oldest,
        latest_observed_epoch: latest,
        canonical_cone_hash: canonical_cone_hash(&ancestors),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;
    use evaporchain_light_cone::Block;

    fn id(b: u8) -> BlockId {
        [b; 32]
    }

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    fn line() -> LightCone {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 700, 5)).unwrap();
        lc.insert(Block::new(id(2), vec![id(1)], 400, 10)).unwrap();
        lc
    }

    #[test]
    fn absent_head_errs() {
        let lc = LightCone::new();
        assert!(matches!(
            summarize_cone(id(99), &lc, lambda(), 0).unwrap_err(),
            SummaryError::AbsentHead(_)
        ));
    }

    #[test]
    fn genesis_has_empty_cone() {
        let lc = line();
        let s = summarize_cone(id(0), &lc, lambda(), 0).unwrap();
        assert_eq!(s.head_id, id(0));
        assert_eq!(s.ancestor_count, 0);
        assert_eq!(s.total_remaining_energy, 0);
        assert_eq!(s.oldest_observed_epoch, 0);
        assert_eq!(s.latest_observed_epoch, 0);
    }

    #[test]
    fn line_head_has_full_cone() {
        let lc = line();
        let s = summarize_cone(id(2), &lc, lambda(), 10).unwrap();
        assert_eq!(s.ancestor_count, 2);
        // Ancestors: id(0) at obs=0 (decayed 10 epochs from 1000),
        //            id(1) at obs=5 (decayed 5 epochs from 700).
        // Just assert > 0 and that the head's own observed epoch is at
        // both ends of the [oldest, latest] interval.
        assert!(s.total_remaining_energy > 0);
        assert_eq!(s.oldest_observed_epoch, 0);
        assert_eq!(s.latest_observed_epoch, 10);
    }

    #[test]
    fn equal_cones_yield_equal_summaries() {
        let lc = line();
        let s1 = summarize_cone(id(2), &lc, lambda(), 10).unwrap();
        let s2 = summarize_cone(id(2), &lc, lambda(), 10).unwrap();
        assert_eq!(s1, s2);
    }

    #[test]
    fn summary_size_is_bounded() {
        // The Optimal Prediction Theorem promise: the summary is a
        // *constant* size regardless of cone depth. This test just
        // documents the size.
        let bytes = std::mem::size_of::<CausalConeSummary>();
        // 32 (head) + 8 (count) + 16 (u128 energy) + 8 + 8 + 32 + padding
        // ≤ 128 bytes in any sane Rust layout.
        assert!(bytes <= 128, "summary must stay constant-size, got {bytes}");
    }
}

//! Energy-threshold gate for the proposer.
//!
//! Per §4.1 #2 the producer extends a maximal antichain *whose total
//! energy clears a threshold*. This module implements just the
//! sum-and-compare against a chain-set threshold; the policy of which
//! antichain to produce is the proposer's job.

use evaporchain_energy_kernel::energy_at_epoch;
use evaporchain_energy_kernel::ChainLambda;
use evaporchain_light_cone::LightCone;
use evaporchain_types::Energy;

use crate::antichain::Antichain;

/// Sum the *λ-decayed* remaining energy of every member of `a` at
/// epoch `t`, then compare against `threshold`. The decay is applied
/// per-member from each block's `observed_epoch` to `t` (using the
/// chain-global λ).
pub fn total_energy_meets_threshold(
    a: &Antichain,
    lc: &LightCone,
    chain_lambda: ChainLambda,
    t: u64,
    threshold: Energy,
) -> bool {
    let total: u128 = a
        .members()
        .iter()
        .filter_map(|id| lc.get(id))
        .map(|b| {
            if t < b.observed_epoch {
                0
            } else {
                let elapsed = t - b.observed_epoch;
                energy_at_epoch(b.energy, chain_lambda.half_life(), elapsed) as u128
            }
        })
        .sum();
    total >= threshold as u128
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;
    use evaporchain_light_cone::{Block, BlockId};

    fn id(b: u8) -> BlockId {
        [b; 32]
    }

    fn lc_with_two_blocks() -> LightCone {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        // genesis-sibling: another block at the same level (= concurrent).
        lc.insert(Block::new(id(1), vec![], 800, 0)).unwrap();
        lc
    }

    #[test]
    fn empty_antichain_below_any_positive_threshold() {
        let lc = LightCone::new();
        let cl = ChainLambda::new(Lambda::from_epochs(100));
        let a = Antichain::empty();
        assert!(!total_energy_meets_threshold(&a, &lc, cl, 0, 1));
        assert!(total_energy_meets_threshold(&a, &lc, cl, 0, 0));
    }

    #[test]
    fn pair_clears_threshold_at_t_zero() {
        let lc = lc_with_two_blocks();
        let cl = ChainLambda::new(Lambda::from_epochs(100));
        let a =
            Antichain::from_set([id(0), id(1)].into_iter().collect(), &lc).unwrap();
        // Total at t=0: 1000 + 800 = 1800.
        assert!(total_energy_meets_threshold(&a, &lc, cl, 0, 1800));
        assert!(!total_energy_meets_threshold(&a, &lc, cl, 0, 1801));
    }

    #[test]
    fn decay_lowers_total_over_time() {
        let lc = lc_with_two_blocks();
        let cl = ChainLambda::new(Lambda::from_epochs(100));
        let a =
            Antichain::from_set([id(0), id(1)].into_iter().collect(), &lc).unwrap();
        // After one half-life the pair is ~half its seed total ≈ 900.
        assert!(!total_energy_meets_threshold(&a, &lc, cl, 100, 1500));
        assert!(total_energy_meets_threshold(&a, &lc, cl, 100, 800));
    }
}

//! Antichain-Mempool integration — wires `evaporchain-antichain-mempool`
//! into the consensus layer.
//!
//! Per `research/INVENTION_STACK.md` §4.1 #2 the proposer extends a maximal
//! antichain whose total energy clears a governance-set threshold before
//! broadcasting a block.
//!
//! # Flow
//!
//! Block proposal → `build_proposal_antichain`: collect DAG tips (frontier)
//! → seed `Antichain::empty()` → `extend_to_maximal` (greedy, tips sorted by
//! descending energy) → check `antichain_energy_gate`.
//!
//! All operations are best-effort and purely observational for the initial
//! integration — a zero threshold means the gate always passes, allowing
//! genesis and early-chain bootstrapping without pre-funded energy.

use evaporchain_antichain_mempool::{
    extend_to_maximal, is_maximal_antichain, total_energy_meets_threshold, Antichain,
};
use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_light_cone::{BlockId, LightCone};
use evaporchain_types::Energy;
use tracing::debug;

/// Default energy threshold for block proposals: 0 = always pass (genesis
/// and bootstrap mode). Governance can raise this via `governance_params`.
pub const DEFAULT_ANTICHAIN_THRESHOLD: Energy = 0;

/// Compute the DAG frontier: block IDs that have no children in `lc`.
///
/// A "tip" is any block not referenced as a parent by any other block.
/// These form the natural candidates for a maximal antichain proposal.
pub fn dag_tips(lc: &LightCone) -> Vec<BlockId> {
    let is_parent: std::collections::BTreeSet<BlockId> = lc
        .ids()
        .flat_map(|id| {
            lc.get(&id)
                .map(|b| b.parents.clone())
                .unwrap_or_default()
        })
        .collect();
    lc.ids().filter(|id| !is_parent.contains(id)).collect()
}

/// Build the maximal antichain from the LightCone frontier (tips).
///
/// Tips are sorted by descending energy so `extend_to_maximal` greedily
/// selects the highest-value concurrent set first.  Returns
/// `Antichain::empty()` if the DAG is empty or antichain construction fails.
pub fn build_proposal_antichain(lc: &LightCone) -> Antichain {
    let mut tips = dag_tips(lc);
    // Sort by descending energy — greedy picks highest-value concurrent set.
    tips.sort_unstable_by(|a, b| {
        let ea = lc.get(a).map(|blk| blk.energy).unwrap_or(0);
        let eb = lc.get(b).map(|blk| blk.energy).unwrap_or(0);
        eb.cmp(&ea)
    });
    extend_to_maximal(&Antichain::empty(), lc, tips)
        .unwrap_or_else(|_| Antichain::empty())
}

/// True iff `antichain`'s total λ-decayed energy at `epoch` clears `threshold`.
///
/// A zero threshold always returns `true` — used at genesis and in
/// governance-unconfigured mode.
pub fn antichain_energy_gate(
    antichain: &Antichain,
    lc: &LightCone,
    chain_lambda_half_life: u64,
    epoch: u64,
    threshold: Energy,
) -> bool {
    if threshold == 0 {
        return true;
    }
    let cl = ChainLambda::new(Lambda::from_epochs(chain_lambda_half_life.max(1)));
    total_energy_meets_threshold(antichain, lc, cl, epoch, threshold)
}

/// Log the proposal antichain's state. Called by `create_proposal` before
/// broadcasting — purely observational; does not gate block production.
pub fn log_proposal_antichain(lc: &LightCone, epoch: u64, chain_lambda_half_life: u64, energy_threshold: Energy) {
    let antichain = build_proposal_antichain(lc);
    let maximal = is_maximal_antichain(&antichain, lc);
    let gate = antichain_energy_gate(&antichain, lc, chain_lambda_half_life, epoch, energy_threshold);
    debug!(
        epoch,
        antichain_size = antichain.len(),
        is_maximal = maximal,
        energy_gate_passed = gate,
        dag_size = lc.len(),
        "proposal antichain"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_light_cone::Block;

    fn id(b: u8) -> BlockId {
        [b; 32]
    }

    fn diamond_lc() -> LightCone {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 800, 1)).unwrap();
        lc
    }

    #[test]
    fn empty_dag_gives_empty_antichain() {
        let lc = LightCone::new();
        let a = build_proposal_antichain(&lc);
        assert!(a.is_empty());
    }

    #[test]
    fn diamond_tips_are_id1_and_id2() {
        let lc = diamond_lc();
        let tips = dag_tips(&lc);
        // id(0) is a parent → not a tip; id(1) and id(2) have no children.
        assert!(tips.contains(&id(1)));
        assert!(tips.contains(&id(2)));
        assert!(!tips.contains(&id(0)));
    }

    #[test]
    fn build_antichain_is_maximal_on_diamond() {
        let lc = diamond_lc();
        let a = build_proposal_antichain(&lc);
        assert_eq!(a.len(), 2);
        assert!(is_maximal_antichain(&a, &lc));
    }

    #[test]
    fn zero_threshold_always_gates_pass() {
        let lc = LightCone::new();
        let a = build_proposal_antichain(&lc);
        assert!(antichain_energy_gate(&a, &lc, 4096, 0, 0));
    }

    #[test]
    fn gate_passes_when_total_energy_sufficient() {
        let lc = diamond_lc();
        let a = build_proposal_antichain(&lc);
        // Tips have energy 900 + 800 = 1700 at epoch 1 with very long half-life.
        assert!(antichain_energy_gate(&a, &lc, 1_000_000, 1, 1_000));
    }

    #[test]
    fn gate_fails_when_threshold_too_high() {
        let lc = diamond_lc();
        let a = build_proposal_antichain(&lc);
        assert!(!antichain_energy_gate(&a, &lc, 1_000_000, 1, 100_000));
    }

    #[test]
    fn single_genesis_block_is_tip_and_antichain() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 500, 0)).unwrap();
        let tips = dag_tips(&lc);
        assert_eq!(tips, vec![id(0)]);
        let a = build_proposal_antichain(&lc);
        assert_eq!(a.len(), 1);
        assert!(is_maximal_antichain(&a, &lc));
    }
}

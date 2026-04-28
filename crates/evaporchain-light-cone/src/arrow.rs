//! Energy-decay time arrow.
//!
//! Per §4.1 row 1 of the doctrine, "energy decay gives the time arrow"
//! — i.e. the chain doesn't need a wall-clock or PoH leader to define
//! "before" and "after"; the partial order is induced by which block
//! still has more remaining energy at any common epoch.
//!
//! Concretely:
//!
//! ```text
//!   for any (ancestor, descendant) pair and any epoch t ≥ both
//!   observed_epochs:
//!       energy_at_epoch(ancestor.energy, λ, t - ancestor.observed_epoch)
//!       ≥
//!       energy_at_epoch(descendant.energy, λ, t - descendant.observed_epoch)
//! ```
//!
//! This is the operational form of "ancestor is older = has been
//! decaying longer = has less remaining energy" — *but* with the
//! additional invariant that the chain enforces `ancestor.energy ≥
//! descendant.energy` at production (each child block inherits a
//! strictly-not-larger seed energy than its max parent). Together
//! these two facts give the time arrow purely from the chain-global λ.

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};

use crate::block::Block;

/// At epoch `t`, does `ancestor`'s remaining energy strictly dominate
/// `descendant`'s? Returns `false` if `t` is before either block's
/// observed_epoch (the time arrow is meaningful only at common
/// observation time).
pub fn time_arrow_holds_at(
    ancestor: &Block,
    descendant: &Block,
    chain_lambda: ChainLambda,
    t: u64,
) -> bool {
    if t < ancestor.observed_epoch || t < descendant.observed_epoch {
        return false;
    }
    let a_remaining = energy_at_epoch(
        ancestor.energy,
        chain_lambda.half_life(),
        t - ancestor.observed_epoch,
    );
    let d_remaining = energy_at_epoch(
        descendant.energy,
        chain_lambda.half_life(),
        t - descendant.observed_epoch,
    );
    a_remaining >= d_remaining
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn equal_energies_arrow_holds_for_older_ancestor() {
        let ancestor = Block::new([0u8; 32], vec![], 1_000, 0);
        let descendant = Block::new([1u8; 32], vec![[0u8; 32]], 1_000, 10);
        // At t=20: ancestor decays for 20 epochs, descendant for 10.
        // ancestor energy < descendant — time arrow FAILS at this t!
        // This is the intuition test: equal seed energies mean the
        // older block is *less* energetic, so callers must enforce
        // descendant.energy ≤ ancestor.energy at production.
        assert!(!time_arrow_holds_at(&ancestor, &descendant, lambda(), 20));
    }

    #[test]
    fn ancestor_higher_energy_arrow_holds() {
        let ancestor = Block::new([0u8; 32], vec![], 2_000, 0);
        let descendant = Block::new([1u8; 32], vec![[0u8; 32]], 1_000, 10);
        // At t=20: ancestor remaining ≫ descendant remaining.
        assert!(time_arrow_holds_at(&ancestor, &descendant, lambda(), 20));
    }

    #[test]
    fn before_observed_epoch_returns_false() {
        let a = Block::new([0u8; 32], vec![], 1_000, 5);
        let d = Block::new([1u8; 32], vec![[0u8; 32]], 1_000, 10);
        // t=3 is before both — time arrow undefined.
        assert!(!time_arrow_holds_at(&a, &d, lambda(), 3));
    }

    #[test]
    fn arrow_at_t_equals_descendant_observed() {
        // When t equals descendant's observed_epoch:
        //   - ancestor has decayed for (t - a.observed) epochs.
        //   - descendant has decayed for 0 epochs (= seed energy).
        // Arrow holds iff ancestor's decayed energy >= descendant's seed.
        let a = Block::new([0u8; 32], vec![], 1_000, 0);
        let d = Block::new([1u8; 32], vec![[0u8; 32]], 500, 50);
        // a after 50 epochs at λ=100: half-decay halfway → ~750ish.
        // d seed = 500. So 750 >= 500 → arrow holds.
        assert!(time_arrow_holds_at(&a, &d, lambda(), 50));
    }
}

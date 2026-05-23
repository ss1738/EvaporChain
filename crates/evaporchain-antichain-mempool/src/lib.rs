//! Evap-Antichain Mempool — the mempool *is* the partial order.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 2:
//!
//! > **Evap-Antichain Mempool** — Mempool *is* the partial order;
//! > producer extends maximal antichains whose total energy clears a
//! > threshold.
//!
//! ## What an antichain is
//!
//! Given a partial order on blocks (the [`evaporchain_light_cone::LightCone`]
//! DAG), an *antichain* is a set of blocks no two of which are
//! comparable — i.e. every pair is *concurrent*. A *maximal* antichain
//! cannot be extended by adding another block without breaking
//! mutual-concurrency.
//!
//! ## Why this is the right mempool shape
//!
//! Two pending payloads that have no causal relation can be packed
//! together into the same proposal without re-ordering risk: their
//! aggregate energy is the sum of the individual energies and the
//! producer can include them in either temporal order. The mempool's
//! job is to surface a *maximal* antichain whose total energy clears
//! the chain-set threshold (the "block enough work has accrued" test).
//!
//! ## Module map
//!
//! - [`antichain`] — `Antichain` newtype + invariant check.
//! - [`maximal`] — `is_maximal_antichain` and `extend_to_maximal`
//!   (greedy by descending energy).
//! - [`threshold`] — `total_energy_meets_threshold` for the proposer
//!   inclusion gate.

pub mod antichain;
pub mod maximal;
pub mod threshold;

pub use antichain::{Antichain, AntichainError};
pub use maximal::{extend_to_maximal, is_maximal_antichain};
pub use threshold::total_energy_meets_threshold;

#[cfg(test)]
mod press_claim_tests {
    //! The press claim lives as a test (INVENTION_STACK §4.1 row 2):
    //! "Evap-Antichain Mempool — Mempool *is* the partial order;
    //! producer extends maximal antichains whose total energy clears a
    //! threshold."
    //!
    //! Three invariants that MUST hold at the runtime level:
    //!
    //! 1. **Antichain invariant** — `from_set` rejects any set containing
    //!    a pair of causally-ordered (comparable) blocks. Only concurrent
    //!    blocks may coexist in an antichain.
    //! 2. **Maximal completion** — `extend_to_maximal` adds every
    //!    concurrent candidate; the result cannot be extended further
    //!    (`is_maximal_antichain` returns true).
    //! 3. **Energy-threshold gate** — the proposer gate fires only when
    //!    the decayed total energy of the antichain meets or exceeds the
    //!    chain-set threshold.

    use std::collections::BTreeSet;

    use evaporchain_energy_kernel::{ChainLambda, Lambda};
    use evaporchain_light_cone::{Block, BlockId, LightCone};

    use crate::{
        extend_to_maximal, is_maximal_antichain, total_energy_meets_threshold, Antichain,
        AntichainError,
    };

    fn id(b: u8) -> BlockId { [b; 32] }

    /// Diamond DAG: genesis → {1, 2} → 3.  Blocks 1 and 2 are concurrent.
    fn diamond() -> LightCone {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1_000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 800, 2)).unwrap();
        lc
    }

    fn cl() -> ChainLambda { ChainLambda::new(Lambda::from_epochs(100)) }

    // ── 1. Antichain invariant ────────────────────────────────────────

    #[test]
    fn concurrent_pair_accepted_comparable_pair_rejected() {
        let lc = diamond();
        // Concurrent: id(1) ∥ id(2) — neither precedes the other.
        Antichain::from_set([id(1), id(2)].into_iter().collect::<BTreeSet<_>>(), &lc).unwrap();
        // Comparable: id(0) < id(1).
        let err = Antichain::from_set(
            [id(0), id(1)].into_iter().collect::<BTreeSet<_>>(), &lc,
        ).unwrap_err();
        assert!(matches!(err, AntichainError::Comparable { .. }));
    }

    // ── 2. Maximal completion ─────────────────────────────────────────

    #[test]
    fn extend_from_singleton_produces_maximal_antichain() {
        let lc = diamond();
        let seed = Antichain::from_set([id(1)].into_iter().collect::<BTreeSet<_>>(), &lc).unwrap();
        let maximal = extend_to_maximal(&seed, &lc, lc.ids()).unwrap();
        assert!(is_maximal_antichain(&maximal, &lc),
            "extended antichain must be maximal — no concurrent candidate was skipped");
    }

    // ── 3. Energy-threshold gate ──────────────────────────────────────

    #[test]
    fn antichain_clears_threshold_at_sum_energy() {
        let lc = diamond();
        let a = Antichain::from_set([id(1), id(2)].into_iter().collect::<BTreeSet<_>>(), &lc).unwrap();
        // Total at epoch 1 (issued at epoch 1, elapsed=0): 900 + 900 = 1800.
        assert!(total_energy_meets_threshold(&a, &lc, cl(), 1, 1800),
            "antichain must clear a threshold equal to its exact total energy");
        assert!(!total_energy_meets_threshold(&a, &lc, cl(), 1, 1801),
            "antichain must not clear a threshold one above its total energy");
    }
}

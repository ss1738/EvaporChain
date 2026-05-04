//! Phasing Nullifier Tree (PNT) — Tier 2.
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Phasing Nullifier Tree (PNT)** — Bounded nullifier sets — kills
//! > monotone privacy-chain growth (Tornado/Aztec/Zcash all suffer
//! > this).
//!
//! ## The problem PNT solves
//!
//! Existing privacy chains accumulate nullifiers forever — the set
//! only grows, never shrinks. Tornado Cash, Aztec, Zcash all carry
//! this monotone-growth liability: state size scales with cumulative
//! activity, not active activity.
//!
//! PNT splits the nullifier history into *phases*. At any time the
//! "live" set is the union of the last `K` phases (a chain-set
//! window). When a phase ages out, its nullifiers drop from the
//! double-spend check. Privacy proofs reference the phase they were
//! constructed against; the chain rejects a proof whose phase is no
//! longer in the live window.
//!
//! ## Substrate
//!
//! - [`tree`] — `PhasedNullifierTree` with current_phase + sliding
//!   window of `live_phases` (configurable depth).
//! - `insert_nullifier(n, phase)` records.
//! - `is_spent_in_window(n)` is the chain-side double-spend check —
//!   returns true iff `n` is in any of the live phases.
//! - `advance_phase()` rotates: drops the oldest phase, opens a fresh
//!   one. Bounded state by construction.

pub mod tree;

pub use tree::{Nullifier, PhasedNullifierTree, PntError};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Phasing Nullifier Tree kills the monotone-growth
    /// liability of Tornado/Aztec/Zcash: live nullifier set is the
    /// union of the last `window_depth` phases. Aged-out nullifiers
    /// are forgotten and can be re-inserted. Double-spend within the
    /// window is rejected; zero depth is rejected at construction."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let mut t = PhasedNullifierTree::new(2).unwrap();
        let n1: Nullifier = [1u8; 32];

        // Insert + double-spend rejection within the window.
        t.insert_nullifier(n1).unwrap();
        assert!(t.is_spent_in_window(&n1));
        assert!(matches!(
            t.insert_nullifier(n1),
            Err(PntError::DoubleSpend { .. })
        ));

        // Phase-rollover boundary: depth=2 keeps the last 2 phases.
        t.advance_phase(); // [phase 0, phase 1]
        assert!(t.is_spent_in_window(&n1));
        t.advance_phase(); // [phase 1, phase 2]; phase 0 dropped
        assert!(!t.is_spent_in_window(&n1), "aged-out nullifier must be forgotten");

        // Now re-inserting the same nullifier succeeds (chain forgot).
        t.insert_nullifier(n1).unwrap();
        assert!(t.is_spent_in_window(&n1));

        // live_count is bounded by the window.
        let mut t2 = PhasedNullifierTree::new(2).unwrap();
        for i in 0u8..5 {
            t2.insert_nullifier([i; 32]).unwrap();
        }
        assert_eq!(t2.live_count(), 5);
        t2.advance_phase();
        for i in 5u8..8 {
            t2.insert_nullifier([i; 32]).unwrap();
        }
        assert_eq!(t2.live_count(), 8);
        t2.advance_phase();
        // Phase 0 (5 entries) is now dropped.
        assert_eq!(t2.live_count(), 3);

        // Zero depth rejected.
        assert!(matches!(
            PhasedNullifierTree::new(0),
            Err(PntError::ZeroDepth)
        ));
    }
}

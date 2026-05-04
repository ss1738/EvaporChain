//! Allen-Decay Opcodes (Tier 2).
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Allen-Decay Opcodes** — 13 interval-relation opcodes in
//! > EvaporScript (Allen 1983); intervals bounded by energy levels.
//!
//! ## What's here
//!
//! - [`interval`] — `Interval { start_energy, end_energy }`. Half-open
//!   `[start, end)`; rejected if `start ≥ end`.
//! - [`relation`] — `AllenRelation` enum, all 13 of:
//!   `Before, Meets, Overlaps, Starts, During, Finishes, Equals,
//!    FinishedBy, Contains, StartedBy, OverlappedBy, MetBy, After`.
//! - [`compute`] — `compute_relation(a, b) -> AllenRelation` pure
//!   function. Symmetric inverse: `inverse(rel)` swaps `a` and `b`.

pub mod compute;
pub mod interval;
pub mod relation;

pub use compute::compute_relation;
pub use interval::{Interval, IntervalError};
pub use relation::AllenRelation;

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Allen-Decay Opcodes ship all 13 of Allen's
    /// interval relations on energy-bounded intervals.
    /// `compute_relation(a,b).inverse() == compute_relation(b,a)`
    /// for every pair (the pair-flip inversion identity); intervals
    /// with `start ≥ end` are rejected at construction."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let a = Interval::new(0, 10).unwrap();
        let b = Interval::new(20, 30).unwrap();
        // a strictly Before b.
        assert_eq!(compute_relation(a, b), AllenRelation::Before);
        // Pair-flip identity.
        assert_eq!(compute_relation(b, a), AllenRelation::After);
        assert_eq!(
            compute_relation(a, b).inverse(),
            compute_relation(b, a),
            "inverse(R(a,b)) must equal R(b,a)"
        );

        // Equals self-inverse.
        let same = Interval::new(5, 15).unwrap();
        assert_eq!(compute_relation(same, same), AllenRelation::Equals);
        assert_eq!(AllenRelation::Equals.inverse(), AllenRelation::Equals);

        // Sweep all 13 relations: double-inverse is identity.
        for r in [
            AllenRelation::Before,
            AllenRelation::Meets,
            AllenRelation::Overlaps,
            AllenRelation::Starts,
            AllenRelation::During,
            AllenRelation::Finishes,
            AllenRelation::Equals,
            AllenRelation::FinishedBy,
            AllenRelation::Contains,
            AllenRelation::StartedBy,
            AllenRelation::OverlappedBy,
            AllenRelation::MetBy,
            AllenRelation::After,
        ] {
            assert_eq!(r.inverse().inverse(), r);
        }

        // start >= end is rejected at construction (no zero-width
        // or inverted intervals slip through).
        assert!(matches!(
            Interval::new(10, 10),
            Err(IntervalError::EmptyOrInverted { .. })
        ));
        assert!(matches!(
            Interval::new(20, 5),
            Err(IntervalError::EmptyOrInverted { .. })
        ));
    }
}

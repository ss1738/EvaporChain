//! GraveGraph V2 — split Dedications.
//!
//! ## V1 vs V2
//!
//! V1 (`evaporchain-grave-graph`) ships 1-to-1 Legacy edges:
//! `from → to` becomes a single Dedication on certified death.
//!
//! V2 ships **split Legacy edges**: a holder declares a Legacy
//! edge with a `SplitSpec` (per-recipient basis-points share,
//! summing to 10_000). On certified source-death, the single
//! Legacy edge inverts into N weighted Dedications, one per
//! recipient. Each survivor curates their share independently;
//! the chain tracks cumulative paid-out shares so the same
//! split slot cannot be double-claimed.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Splits must sum to 10_000 bp** — exactly. Anything else
//!    is rejected at declaration.
//!
//! 2. **Inversion produces N weighted Dedications, one per
//!    recipient.** Each is independently curated by its
//!    surviving recipient.
//!
//! 3. **Total paid-out tracked across split shares.** Once the
//!    sum of paid shares hits 10_000 bp, the legacy is
//!    fully-distributed and no further claims accepted.
//!
//! ## Module map
//!
//! - [`split`] — [`SplitLegacy`] state machine.

pub mod split;

pub use split::{SplitDedication, SplitError, SplitId, SplitLegacy, SplitState};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use split::Curation;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "GraveGraph V2 splits a Legacy edge into N
    /// weighted Dedications on certified source-death. Shares MUST
    /// sum to exactly 10_000 bp at declaration. Survivors curate
    /// independently; once curated, total_share_paid increments and
    /// fully-distributed legacies refuse further mutations."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let source = [0xAAu8; 32];
        let r1 = [0xB1u8; 32];
        let r2 = [0xB2u8; 32];
        let r3 = [0xB3u8; 32];

        // 60% / 30% / 10% sums to 10_000 → declare succeeds.
        let mut leg = SplitLegacy::declare(
            SplitId([1u8; 32]),
            source,
            vec![(r1, 6_000), (r2, 3_000), (r3, 1_000)],
            0,
        )
        .unwrap();
        assert!(matches!(leg.state, SplitState::Pending));

        // Curating before death rejected.
        assert!(matches!(
            leg.curate(r1, Curation::Accepted),
            Err(SplitError::NotInverted)
        ));

        // Source death → inverted.
        leg.certify_source_death(100);
        assert!(matches!(leg.state, SplitState::Inverted { died_at_epoch: 100 }));

        // r1 accepts → counts toward total.
        leg.curate(r1, Curation::Accepted).unwrap();
        assert_eq!(leg.total_share_paid_bp, 6_000);

        // r2 rejects → also counts (slot claimed).
        leg.curate(r2, Curation::Rejected).unwrap();
        assert_eq!(leg.total_share_paid_bp, 9_000);

        // Re-curation rejected (slot already claimed).
        assert!(matches!(
            leg.curate(r1, Curation::Hidden),
            Err(SplitError::AlreadyCurated(_))
        ));

        // Unknown recipient rejected.
        let stranger = [0xCCu8; 32];
        assert!(matches!(
            leg.curate(stranger, Curation::Accepted),
            Err(SplitError::UnknownRecipient(_))
        ));

        // r3 hides → fully distributed.
        leg.curate(r3, Curation::Hidden).unwrap();
        assert!(leg.is_fully_distributed());

        // Construction guards: shares not summing to exactly 10_000.
        assert!(matches!(
            SplitLegacy::declare(
                SplitId([2u8; 32]),
                source,
                vec![(r1, 5_000), (r2, 4_000)],
                0,
            ),
            Err(SplitError::SplitDoesNotSumToFull { sum: 9_000 })
        ));
        // Self-recipient rejected.
        assert!(matches!(
            SplitLegacy::declare(
                SplitId([3u8; 32]),
                source,
                vec![(source, 10_000)],
                0,
            ),
            Err(SplitError::SelfRecipient)
        ));
    }
}

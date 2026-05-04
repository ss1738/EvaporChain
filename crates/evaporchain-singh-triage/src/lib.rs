//! Singh-Triage — wallet-opens-on-inbox UX paradigm.
//!
//! Per `research/INVENTION_STACK.md` §A5.4:
//!
//! > Wallet opens not on a balance — on an **inbox**. "3 items decay
//! > today" with swipe actions: Refresh / Let Die / Archive-to-ghost.
//! > Below the fold: "Tomorrow (7), This Week (24), Healthy (137)."
//!
//! ## What this crate does
//!
//! This is the **on-chain backing logic** for the wallet UX. The
//! wallet itself is a frontend (React Native, web, etc.) that consumes
//! the data structures + classifier defined here. Validators don't
//! agree on UI pixels; they agree on:
//!
//! - per-item `TriageBucket` (Today / Tomorrow / ThisWeek / Healthy
//!   / Decayed) at a given epoch
//! - per-bucket inbox counts
//! - per-action outcome semantics (Refresh / LetDie / Archive)
//!
//! Two structural decisions:
//!
//! 1. **Buckets are pure functions of `(item.energy, item.half_life,
//!    item.last_refreshed, epoch_now)`.** No clock, no off-chain
//!    state, no oracle. Validators compute the same buckets given
//!    the same chain head.
//!
//! 2. **The "decay today" threshold is expressed in epochs, not days
//!    or hours.** A wallet that wants "1 day" buckets multiplies its
//!    chain's epochs-per-day. Cross-chain portable; doesn't bake in
//!    block time.
//!
//! ## Module map
//!
//! - [`item`] — [`TriageItem`] minimal record (id, energy, half_life,
//!   last_refreshed_epoch).
//! - [`bucket`] — [`TriageBucket`] enum + [`classify`] pure function.
//! - [`inbox`] — [`Inbox`] aggregate over a slice of items;
//!   [`bucket_counts`] for the headline numbers.
//! - [`action`] — [`Action`] (Refresh / LetDie / Archive) + outcome
//!   semantics applied to a `TriageItem`.

pub mod action;
pub mod bucket;
pub mod inbox;
pub mod item;

pub use action::{apply_action, Action, ActionError, ActionOutcome};
pub use bucket::{classify, epochs_until_threshold, TriageBucket};
pub use inbox::{bucket_counts, Inbox};
pub use item::{TriageItem, TriageItemError};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh-Triage classifies wallet items into the
    /// inbox buckets {Decayed, Today, Tomorrow, ThisWeek, Healthy}
    /// as a pure function of (item, epoch_now, horizons). Buckets
    /// are monotone in time — items only move toward Decayed,
    /// never back to Healthy. Validators agree on per-item bucket
    /// without consulting any clock or oracle."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let it = TriageItem::new([0u8; 32], 8, 1, 0).unwrap();

        // Until threshold: 8→4→2→1 = 3 hops. Bucket depends on horizons.
        assert_eq!(
            classify(&it, 0, 5, 10, 50),
            TriageBucket::Today,
            "until 3 ≤ today 5 → Today"
        );
        assert_eq!(
            classify(&it, 0, 1, 2, 2),
            TriageBucket::Healthy,
            "week 2 < until 3 → Healthy"
        );

        // Already-dead item is Decayed.
        let dead = TriageItem::new([1u8; 32], 1, 1, 0).unwrap();
        assert_eq!(classify(&dead, 100, 1, 2, 7), TriageBucket::Decayed);

        // Bucket monotonicity: a fresher snapshot of (it) has bucket
        // closer to Healthy than a later snapshot.
        let order = |b: TriageBucket| match b {
            TriageBucket::Healthy => 0,
            TriageBucket::ThisWeek => 1,
            TriageBucket::Tomorrow => 2,
            TriageBucket::Today => 3,
            TriageBucket::Decayed => 4,
        };
        let big = TriageItem::new([2u8; 32], 1_024, 10, 0).unwrap();
        let early = order(classify(&big, 0, 1, 2, 7));
        let later = order(classify(&big, 200, 1, 2, 7));
        assert!(later >= early, "bucket only moves toward Decayed");

        // Pure function: same inputs → same bucket.
        assert_eq!(classify(&it, 0, 5, 10, 50), classify(&it, 0, 5, 10, 50));

        // bucket_counts aggregates an inbox.
        let inbox = bucket_counts(&[it.clone(), dead.clone()], 100, 5, 10, 50);
        assert_eq!(inbox.total(), 2);
    }
}

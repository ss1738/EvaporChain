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

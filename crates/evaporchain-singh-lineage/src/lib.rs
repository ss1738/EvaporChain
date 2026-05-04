//! Singh-Lineage (EvaporWallet-Lineage).
//!
//! Per `research/INVENTION_STACK.md` §A5.4:
//!
//! > Wallet has a second screen called Lineage: family tree showing
//! > primary key + designated successors with dormancy thresholds.
//! > *"If I'm silent 90 days, my daughter's key gains 25% authority;
//! > 180 days, 50%; 365 days, full."* Per-asset posthumous designation.
//! > Real-time visual of your own digital mortality.
//!
//! > **Cultural lineage:** Apple Legacy Contact (closest precedent —
//! > but reactive, not graduated); Google Inactive Account Manager.
//!
//! > **Pitch:** *"crypto solves inheritance."* Highest mainstream-
//! > press potency of the wallet set — FT, NYT personal finance,
//! > Atlantic.
//!
//! ## What's structurally different from existing inheritance UX
//!
//! Apple Legacy Contact and Google Inactive Account Manager are both
//! **reactive**: a successor either has access or they don't, with a
//! single threshold and a one-shot transfer event. Singh-Lineage is
//! **graduated**: authority accrues continuously as the issuer's
//! silence lengthens. Three structural decisions:
//!
//! 1. **Authority is a fraction in [0, 1]**, not a binary flag. At
//!    each `DormancyTier`, a successor's authority increases by the
//!    tier's `authority_share`. Multiple successors can be on the
//!    same lineage with overlapping shares; the chain enforces that
//!    total authority across all successors at any tier sums to ≤ 1.
//!
//! 2. **Dormancy resets on any signed action by the issuer**, not
//!    just heartbeat pings. Validators agree on a single source of
//!    truth: `last_seen_epoch`. The wallet's "still alive" signal
//!    is just any tx the issuer signed.
//!
//! 3. **Per-asset designation, not per-wallet.** The lineage policy
//!    can attach to a specific [`evaporchain_types::ObjectId`] so a
//!    user can leave one NFT to a daughter and another to a sibling
//!    without one-shot account merges. (V1 ships per-lineage; the
//!    type is keyed on a 32-byte handle the wallet/contract layer
//!    chooses to scope to a wallet, an asset, or a bundle.)
//!
//! ## Module map
//!
//! - [`tier`] — [`DormancyTier`] (epochs threshold + authority share);
//!   [`Ladder`] tier list with monotonicity invariants.
//! - [`successor`] — [`Successor`] declaration; one per heir.
//! - [`lineage`] — [`Lineage`] policy (issuer + ladder + successors +
//!   `last_seen_epoch`); `authority_at(epoch_now, successor)` resolves
//!   the live fraction; `touch(epoch_now)` resets dormancy.

pub mod lineage;
pub mod successor;
pub mod tier;

pub use lineage::{Lineage, LineageError, LineageId};
pub use successor::{Successor, SuccessorError};
pub use tier::{DormancyTier, Ladder, LadderError};

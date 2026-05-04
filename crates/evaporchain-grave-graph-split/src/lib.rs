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

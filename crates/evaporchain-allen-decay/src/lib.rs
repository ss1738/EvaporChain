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

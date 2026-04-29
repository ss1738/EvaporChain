//! Decay-Stamped Nullifiers (DSN) — Tier 2.
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Decay-Stamped Nullifiers (DSN)** — Per-window nullifier
//! > accumulators; bounded state for privacy chains.
//!
//! ## Relationship to PNT
//!
//! - [`evaporchain_pnt`] keeps a sliding window of *raw nullifier
//!   sets*: each phase is `BTreeSet<Nullifier>`. State is bounded by
//!   `window_depth × max_per_phase`.
//! - DSN keeps a sliding window of *cryptographic accumulators*: each
//!   window is one `Accumulator` value (32 bytes), into which all of
//!   that window's nullifiers have been folded. State is bounded by
//!   `window_depth × 32 bytes` — independent of how many nullifiers
//!   land in each window.
//!
//! Trade-off: DSN's membership check is *probabilistic* under the
//! accumulator's collision-resistance assumption (substrate uses
//! domain-separated blake3 chaining; production swaps in an RSA
//! accumulator or vector commitment).
//!
//! ## Module map
//!
//! - [`accumulator`] — `Accumulator { value: [u8; 32], count: u64 }`
//!   with `fold(nullifier)` operation.
//! - [`window`] — `DsnWindow` sliding-window of accumulators per
//!   epoch. `fold_nullifier(n, current_epoch)` and
//!   `advance_window()`.

pub mod accumulator;
pub mod window;

pub use accumulator::Accumulator;
pub use window::{DsnError, DsnWindow};

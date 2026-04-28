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

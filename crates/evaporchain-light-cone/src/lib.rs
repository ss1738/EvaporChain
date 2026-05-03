//! Light-Cone Consensus substrate.
//!
//! **Production status (2026-05-03):** read-only observability layer.
//! The chain authority is still Tendermint's linear chain — see the
//! honest comment at `evaporchain-consensus/src/tendermint.rs:3115-3118`.
//! This crate's DAG / antichain / causal-past machinery is correct math,
//! but no consensus-rejecting decision currently consults it. Promoting
//! Light-Cone to authoritative fork-choice (replacing the single
//! `parent_hash` rule) is the Layer 4 work tracked in
//! `DOCTRINE_PUNCH_LIST.md`. Until then, the "Soul of the chain" framing
//! describes the *intent*, not the shipped behaviour.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 1:
//!
//! > **Light-Cone Consensus** — Causal-set partial-order consensus
//! > (Sorkin/Pratt). Energy decay gives the time arrow. *Soul of the
//! > chain.*
//!
//! ## Key idea
//!
//! Blocks form a *partial order* (a DAG of "before/after" relations
//! determined by parent edges), not a total-order chain. Two blocks
//! that have no path between them are *concurrent* — neither precedes
//! the other. The arrow of time comes from the chain-global λ: a
//! block's energy decays as descendants accumulate, so any block in
//! `b`'s causal past has strictly higher remaining energy than `b`
//! at the same epoch.
//!
//! Sorkin causal-set math is a literal mathematical formalism (per
//! §3.1 of the doctrine — "physics-inspired, not invoking quantum
//! gravity"); we model it directly with Rust's data structures.
//!
//! ## Module map
//!
//! - [`block`] — `BlockId` (32-byte hash) + `Block { id, parents,
//!   energy, observed_epoch }`.
//! - [`dag`] — `LightCone` DAG with insert + causal_past + causal_future.
//! - [`concurrency`] — `is_concurrent`, `precedes`, `comparable`.
//! - [`arrow`] — energy-decay-based time arrow check
//!   (`causal_past_has_higher_energy_at`).

pub mod arrow;
pub mod block;
pub mod concurrency;
pub mod dag;

pub use arrow::time_arrow_holds_at;
pub use block::{Block, BlockId};
pub use concurrency::{comparable, is_concurrent, precedes};
pub use dag::{causal_future, causal_past, LightCone, LightConeError};

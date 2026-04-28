//! Linear-Affine-Decay VM (LAD) substrate.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 12:
//!
//! > **Linear-Affine-Decay VM** — Move resources × decay. "Use it or
//! > evaporate." Forces liveness as a type-system property.
//!
//! Source attribution (DP-VM agent, round 2): "Move × Wadler-Girard
//! linear logic × decay."
//!
//! ## What this substrate ships
//!
//! Real Move-style linearity needs *compiler-enforced* substructural
//! types — that lives in a future `evaporchain-script-lad` (the LAD
//! frontend to the existing scripting layer). This crate ships the
//! *runtime* data structures + the operational semantics so the
//! frontend has a target to lower into.
//!
//! Three substructural modes:
//!
//! - **Linear** — must be consumed exactly once.
//! - **Affine** — may be consumed at most once (drop is allowed).
//! - **Decaying** — affine, plus an explicit decay window. Consumes
//!   itself implicitly when `current_epoch ≥ created_at + window`.
//!
//! Liveness as a type-system property: a `Decaying` resource that
//! the script never `use`s gets recorded as *evaporated* on the next
//! epoch tick — automatic GC for stale state, and the chain can
//! refuse to honour a tx that touches an evaporated resource.
//!
//! ## Module map
//!
//! - [`mode`] — `Mode` enum.
//! - [`resource`] — `Resource<T> { value, mode, created_at,
//!   decay_window, consumed }`.
//! - [`ops`] — `use_resource`, `drop_resource`, `tick_decay`.

pub mod mode;
pub mod ops;
pub mod resource;

pub use mode::Mode;
pub use ops::{drop_resource, tick_decay, use_resource, OpError};
pub use resource::Resource;

//! Single-λ substrate for EvaporChain.
//!
//! Per `research/INVENTION_STACK.md`:
//!
//! - **§1.1 Single-λ Principle**: the chain has *one* fundamental constant —
//!   λ (the decay rate). Every layer (consensus, mempool, time, gas, stake,
//!   governance, capabilities, identity, demurrage) reads from this one
//!   number. This crate owns it.
//! - **§1.2 Conservation Invariant**: total energy across
//!   {accounts + stake + refresh_pool + slashed_pool} decreases monotonically
//!   only via λ — never by destruction in any other transition. This crate
//!   defines the conservation domain (`Compartment`), the accumulator that
//!   tracks each compartment, and the verifier that audits transitions.
//!
//! The `energy_at_epoch` decay function itself is owned by
//! `evaporchain-types::energy_at_epoch` (mechanized in
//! `research/coq/EnergyDecayMonotonicity.v`); this crate re-exports it so
//! every chain layer reaches it through the same canonical entry point.

pub mod compartment;
pub mod conservation;
pub mod lambda;
pub mod redirect;
pub mod refresh_pool;

pub use compartment::{Compartment, EnergyAccumulator};
pub use conservation::{ConservationCheck, ConservationViolation};
pub use evaporchain_types::{energy_at_epoch, Energy, HalfLife};
pub use lambda::{ChainLambda, Lambda, DEFAULT_LAMBDA};
pub use redirect::{EnergyRedirect, RedirectKind};
pub use refresh_pool::{RefreshCredit, RefreshPool};

//! Singh-Bennett Asymmetric VM (SBAV).
//!
//! Per `research/INVENTION_STACK.md` §A5.1:
//!
//! > **Formalism:** Bennett 1973 reversible TM; Janus reversible
//! > imperative language (Yokoyama-Glück 2007); Landauer 1961
//! > (irreversibility ⇔ entropy export ⇔ energy cost).
//!
//! > **Decay synergy — STRUCTURAL (philosophically the cleanest in
//! > the entire stack):** every classical opcode is reversible
//! > (zero-energy in the limit); **only `decay(λ)` exports entropy
//! > and is the unique irreversible primitive.** Not analogy —
//! > Landauer literally. State at block t is bit-for-bit recoverable
//! > from t+k *except for the λ-decay trace*, which is the chain's
//! > thermodynamic arrow.
//!
//! > **Pitch:** *"the first computer system where the laws of
//! > thermodynamics dictate which operations cost gas."*
//!
//! ## What this crate ships
//!
//! A first-pass model of an asymmetric VM where:
//!
//! 1. Every reversible opcode has zero gas cost (in the limit) and a
//!    `Reversible` impl that produces an inverse op.
//! 2. The **only** irreversible opcode is `Decay(λ)`. It alone has a
//!    nonzero gas floor — the chain literally encodes Landauer's
//!    `kT ln 2` per bit erased as the unit of irreversible cost.
//! 3. A program's *trace* is recoverable bit-for-bit from any later
//!    state via the inverse trace, **except** for the decay events,
//!    which are the chain's thermodynamic arrow.
//!
//! ## Module map
//!
//! - [`reversible`] — [`Reversible`] trait; reversible-op contract.
//! - [`op`] — [`Op`] opcode enum (the invertible classical ops + the
//!   one irreversible `Decay`).
//! - [`vm`] — [`Sbav`] tiny stack-machine; `apply` and `inverse_apply`.
//! - [`trace`] — [`Trace`] recording of executed ops + a guarantee
//!   that `inverse_apply` produces an identity round-trip on the
//!   reversible subset.

pub mod op;
pub mod reversible;
pub mod trace;
pub mod vm;

pub use op::{Op, OpError};
pub use reversible::{Reversible, ReversibilityError};
pub use trace::{Trace, TraceError};
pub use vm::{Sbav, VmError};

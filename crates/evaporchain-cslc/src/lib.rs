//! Causal-State Light Client (CSLC).
//!
//! Per `research/INVENTION_STACK.md` §A1.2 T3 (Tier-0 theorem-grade):
//!
//! > **Causal-State Light Client (CSLC)** — ε-machine reconstruction
//! > of the energy-filtered tx process.
//! > Shalizi-Crutchfield 2001 (Optimal Prediction Theorem);
//! > Shalizi-Klinkner 2004 (CSSR).
//! >
//! > "Our light clients carry the *unique minimal sufficient
//! > predictive model* of the energy-surviving tx process. Provably
//! > optimal — any model with fewer states cannot be predictively
//! > sufficient."
//!
//! ## What an ε-machine is
//!
//! Given a stationary process over a finite alphabet `Σ`, the
//! *causal states* `{S_i}` are the equivalence classes of pasts
//! under the relation
//!
//! ```text
//!   x_<t ~ y_<t  iff  P(future | x_<t) = P(future | y_<t)
//! ```
//!
//! The ε-machine is the labeled-transition graph on these states
//! whose transition probabilities are `P(s' | s, σ)`. Shalizi-
//! Crutchfield proved that this is the **unique minimal sufficient
//! predictive model** — any other predictor with fewer states cannot
//! match the ε-machine's predictions.
//!
//! ## What this crate ships
//!
//! - [`machine`] — `EpsilonMachine` data structure (states +
//!   transition table + per-state output distributions).
//! - [`reconstruct`] — `reconstruct_unconditional(observations)`
//!   single-state baseline (a memoryless model from a flat
//!   symbol-count vector). Cheapest path; useful when caller has
//!   no access to the underlying symbol stream.
//! - [`cssr`] — full Shalizi-Klinkner 2004 CSSR algorithm via
//!   `reconstruct_cssr(stream, alphabet_size, l_max, alpha)`.
//!   Recovers multi-state ε-machines: fair coin → 1 state,
//!   period-2 → 2 states, golden-mean shift → 2 states with the
//!   uniform-class pmf within ε=0.02 TV-distance of reference at
//!   α=0.001. Phase II determinization currently over-splits on
//!   the canonical even-process (~2× canonical state count); fix
//!   tracked in `DOCTRINE_PUNCH_LIST.md` Layer 2 follow-up.
//! - [`predict`] — `predict_next(machine, current_state)` returns
//!   the per-symbol output distribution.

pub mod cssr;
pub mod machine;
pub mod predict;
pub mod reconstruct;

pub use cssr::{
    reconstruct_cssr, ReconstructError, DEFAULT_ALPHA, DEFAULT_L_MAX, MIN_COUNT_FOR_TEST,
};
pub use machine::{EpsilonMachine, MachineError, StateId};
pub use predict::predict_next;
pub use reconstruct::reconstruct_unconditional;

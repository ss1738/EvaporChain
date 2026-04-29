//! Sentinel — autonomic chain-parameter governance.
//!
//! Per `research/INVENTION_STACK.md` Amendment 2 §A2.5:
//!
//! > **Sentinel** — autonomic parameter governance via decay-weighted
//! > LLSA voting within hard-coded bounds.
//! >
//! > "EvaporChain is the first chain that governs itself the way a
//! > body does — through homeostasis, not legislators."
//!
//! ## What homeostasis means here
//!
//! Each governable parameter has hard-coded `(min, max)` bounds set
//! at genesis. Within those bounds, validators vote continuously on
//! the parameter's value. Vote weights *decay* with the chain's
//! global λ (recent votes dominate; ancient ones evaporate). The
//! sentinel controller continuously moves the parameter toward the
//! decay-weighted-average vote, subject to a per-tick max-step cap.
//!
//! No legislators, no proposals, no time-bounded voting periods —
//! the chain finds its set-point continuously. Like body
//! temperature, like blood pH, like a thermostat. Hence "homeostasis,
//! not legislators."
//!
//! Pairs with `evaporchain-llsa`: any *bounds change* (i.e. moving
//! the `(min, max)` envelope itself) requires a Coq-checked LLSA
//! proof that the new bounds preserve chain invariants.
//!
//! ## Substrate
//!
//! - [`parameter`] — `BoundedParameter { id, current, min, max }`.
//! - [`vote`] — `Vote { validator_id, target, observed_epoch }`.
//! - [`controller`] — `SentinelController::propose_adjustment(param,
//!   votes, λ, current_epoch, max_step)` returns the new parameter
//!   value clamped within bounds and within the per-tick step cap.

pub mod controller;
pub mod parameter;
pub mod vote;

pub use controller::{propose_adjustment, ControllerError};
pub use parameter::{BoundedParameter, ParameterError};
pub use vote::Vote;

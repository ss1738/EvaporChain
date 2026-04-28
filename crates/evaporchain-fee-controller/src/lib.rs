//! Singh-Lyapunov Fee Controller — the whitepaper centerpiece.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 4:
//!
//! > Singh-Lyapunov Fee Controller — First L1 with **provably globally
//! > stable** fee market. Lyapunov `V(E) = ½(E − E*)²` converges
//! > *because* of decay. Antifragile under attack. **Whitepaper
//! > centerpiece.**
//!
//! And per the mechanism-design agent's source attribution: "Lyapunov
//! drift on EIP-1559". The shape is an *integrator with leak*:
//!
//! ```text
//!   E_{n+1} = decay(E_n − E*; λ, 1) + E* + (gas_used − target_gas)
//!   diff_{n+1} = decay(diff_n; λ, 1) + perturbation
//!   V(diff)   = ½ diff²
//! ```
//!
//! The natural λ-decay of `evaporchain-energy-kernel` does the
//! stabilizing work. The controller's only job is to translate
//! `(E − E*)` into a base-fee response — with the perturbation
//! magnitude clipped so each empty block is guaranteed-monotone in `V`.
//!
//! This crate ships the substrate (state machine + Lyapunov function +
//! step + drift measurement). The closed-form equilibrium extension
//! (CFM, Crooks-Singh) and the Sanov-Slashing companion live in their
//! own crates so each primitive is independently auditable.
//!
//! ## Module map
//!
//! - [`params`] — `FeeControllerParams` (target_energy, target_gas, λ,
//!   gain).
//! - [`state`] — `FeeState` (current energy + current base fee).
//! - [`lyapunov`] — `lyapunov_value(diff)` and signed-diff helpers.
//! - [`controller`] — `FeeController::step` applies one block update;
//!   returns `(FeeState, Drift)` so callers can audit the Lyapunov
//!   change directly.

pub mod controller;
pub mod lyapunov;
pub mod params;
pub mod state;

pub use controller::{Drift, FeeController};
pub use lyapunov::{lyapunov_value, signed_diff};
pub use params::FeeControllerParams;
pub use state::FeeState;

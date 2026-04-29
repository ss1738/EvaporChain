//! Crooks-MEV Refund.
//!
//! Per `research/INVENTION_STACK.md` §A1.3 (Tier-0 supporting):
//!
//! > **Crooks-MEV Refund** | Crooks 1999 fluctuation-theorem ratio |
//! > refund formula falls out of CFM; fair restitution = ΔF
//! > computed from forward/reverse work distributions.
//!
//! ## How the math gives the refund
//!
//! Crooks 1999 fluctuation theorem:
//!
//! ```text
//!   P_F(W) / P_R(−W)  =  exp(β · (W − ΔF))
//! ```
//!
//! Solving for the free-energy difference:
//!
//! ```text
//!   ΔF  =  W − (1/β) · ln( P_F(W) / P_R(−W) )
//! ```
//!
//! `ΔF` is the *intrinsic* (non-dissipative) value transferred. The
//! difference `W − ΔF` is the *dissipated work* — the MEV the attacker
//! extracted by stretching the system out of equilibrium. The fair
//! restitution to the victim is exactly that dissipated work.
//!
//! ## Substrate
//!
//! - [`refund::compute_delta_f_millibits(w_milli, log_ratio_milli, beta_mb)`]
//!   reuses `evaporchain_cfm::crooks_log_ratio_millibits` to compute
//!   the LHS, then solves for `ΔF` in millibits-per-fee-unit.
//! - [`refund::compute_refund(work_extracted, delta_f_milli)`] returns
//!   the dissipated-work amount in `Energy`.

pub mod refund;

pub use refund::{compute_delta_f_millibits, compute_refund, RefundError};

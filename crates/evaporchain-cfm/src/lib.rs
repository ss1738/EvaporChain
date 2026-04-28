//! Crooks-Singh Fee Equilibrium (CFM).
//!
//! Per `research/INVENTION_STACK.md` §A1.2 T2 (Tier-0 theorem-grade):
//!
//! > Crooks-Singh Fee Equilibrium (CFM) — closed-form fee distribution
//! > `p_eq(f) ∝ exp(−β f) · ρ_mempool(f)` with `β = 1/λ`.
//! > Crooks 1999 (Fluctuation Theorem); Jarzynski 1997.
//! >
//! > "Our fee market satisfies an *exact equality* between work and
//! > free-energy difference (not a bound), with the inverse temperature
//! > supplied by our decay constant."
//!
//! ## What this crate exposes
//!
//! - [`beta`] — derive the inverse temperature `β = 1/λ` from
//!   `ChainLambda`, in fixed-point millibits per fee unit.
//! - [`weight`] — integer Boltzmann weight `2^(−β·f)` (shift-based
//!   approximation; deterministic, monotone, chain-safe).
//! - [`equilibrium`] — `cfm_equilibrium(mempool_pmf, fees, beta)`
//!   reweights the mempool distribution with Boltzmann factors and
//!   renormalises to a proper [`Distribution`].
//! - [`crooks`] — Crooks identity:
//!   `log(P_F(W) / P_R(−W)) = β · (W − ΔF)`. Returns the LHS so a
//!   caller can compare against the RHS.
//!
//! Both [`Distribution`] and `FIXED_POINT_SCALE` are re-exported from
//! `evaporchain-sanov-slashing` so the two fee-market siblings share
//! the same probability-vector representation.

pub mod beta;
pub mod crooks;
pub mod equilibrium;
pub mod weight;

pub use beta::{beta_millibits_per_fee, BetaError};
pub use crooks::{crooks_log_ratio_millibits, CrooksError};
pub use equilibrium::{cfm_equilibrium, EquilibriumError};
pub use evaporchain_sanov_slashing::{Distribution, FIXED_POINT_SCALE};
pub use weight::boltzmann_weight;

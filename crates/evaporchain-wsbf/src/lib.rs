//! Wilson-Singh Block Flow (WSBF) — Tier 2 primitive.
//!
//! # Doctrine
//!
//! Per INVENTION_STACK.md §4.2:
//! > RG-flow on chain history; old blocks become "effective theory" parameters.
//!
//! # Physical analogy
//!
//! Wilson-Kogut 1974 renormalization group: integrate out UV degrees of
//! freedom (high-energy, short-distance) in successive shells, yielding an
//! effective Lagrangian at each IR scale.  Applied to chain history:
//!
//! - **UV = recent high-energy blocks** (large account energies, many active
//!   objects, short-lived tokens).
//! - **IR = coarse-grained long-range effective theory** (surviving accounts,
//!   long-λ steady-state behaviour).
//! - Each RG step integrates over a block window of size `coarse_grain`:
//!   replaces N detailed blocks with 1 effective-parameter set.
//!
//! The RG "flow" in WSBF is controlled by λ: each step produces an
//! `EffectiveParams` whose `lambda_eff` is the volume-weighted average of
//! the input window's λ, shifted by the entropy of the window's energy
//! distribution.  As the chain ages, λ_eff drifts toward the basin of
//! attraction defined by the dominant account structure.
//!
//! # Citations
//!
//! Wilson-Kogut 1974, *The renormalization group and the ε expansion*,
//! Phys. Rep. 12(2).  Cardy 1996, *Scaling and Renormalization in Statistical
//! Physics*, Cambridge.

pub mod flow;
pub mod params;

pub use flow::{rg_step, RgFlowError};
pub use params::{BlockSummary, EffectiveParams, RgFlowParams};

//! RG Consensus Phase Map — A4.3.11.
//!
//! # Doctrine
//!
//! From INVENTION_STACK.md A4.3.11 (PROMISING):
//! > Produce a **phase diagram** of consensus regimes under varying λ,
//! > validator count, and adversary fraction.  Map "fixed points" of the
//! > RG flow to operational regimes (liveness-stable, safety-stable, frozen,
//! > chaotic).  First-of-its-kind diagnostic for L1 operators.
//!
//! # Fixed points of the WSBF RG flow
//!
//! In Wilson's RG, a fixed point is a parameter set that does not change
//! under further renormalization.  In WSBF, the fixed points are:
//!
//! | Fixed point | What it means | Operational regime |
//! |---|---|---|
//! | λ → 0 | All energy decayed; chain frozen | `Frozen` |
//! | λ → ∞ | No decay; chain grows unbounded | `Unbounded` (not a real attractor) |
//! | λ = λ* (stable) | Balanced decay/refresh | `LivenessStable` |
//! | Oscillating | Decay/refresh cycle | `Chaotic` |
//!
//! A safety fixed point exists when the adversary fraction `f < 1/3` and
//! the validator set is large enough that stake-weighted quorum is robust.
//!
//! # Phase boundary conditions
//!
//! Based on the Tendermint BFT safety threshold (f < 1/3) and the
//! Boltzmann-stake liveness threshold (quorum reachable when T > T_freeze):
//!
//! - **Frozen**: `λ_eff < LAMBDA_FREEZE` — effective half-life so short that
//!   validators can't maintain stake to vote.
//! - **Chaotic**: `adversary_fraction ≥ 1/3` — BFT safety threshold breached.
//! - **SafetyStable**: `f < 1/3` AND `n_validators ≥ MIN_QUORUM` AND `λ_eff ≥ LAMBDA_FREEZE`.
//! - **LivenessStable**: `f < 1/10` AND `n_validators ≥ MIN_QUORUM` AND `λ_eff ≥ LAMBDA_LIVENESS`.
//!
//! Both stable phases exist simultaneously when conditions overlap; in that
//! case we return `LivenessStable` (the stronger condition implies the weaker).
//!
//! # Citations
//!
//! Wilson 1971 *Renormalization Group and Critical Phenomena I & II*,
//! Phys. Rev. B 4(9).  Cardy 1996 *Scaling and Renormalization in
//! Statistical Physics*, Cambridge LNP.

pub mod phase;

pub use phase::{ConsensusPhase, PhaseMapParams, classify_regime, phase_trajectory};

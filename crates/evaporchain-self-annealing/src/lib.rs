//! Self-Annealing Validator Set — A4.3.2.
//!
//! # Doctrine
//!
//! From INVENTION_STACK.md A4.3.2 (PROMISING):
//! > Cooling schedule IS λ. As chain energy evaporates the validator set
//! > crystallises — high-performing validators are locked in, low-performing
//! > ones are ejected.  Kirkpatrick-Gelatt-Vecchi 1983 SA.
//!
//! # How it works
//!
//! Standard simulated annealing accepts a worse candidate with probability
//! `exp(−ΔE / T)` where T is the current temperature.  Here:
//!
//! - **Temperature T** = the chain's λ-derived effective half-life for the
//!   current epoch: `T(epoch) = λ × 2^(−epoch / λ)`.  As epochs accumulate,
//!   T falls monotonically toward 0 — the "cooling schedule".
//!
//! - **Energy of a validator** = `−score(v)` where score combines stake,
//!   activity, and uptime (lower energy = better candidate, matching SA
//!   minimisation convention).
//!
//! - **Acceptance**: at epoch E, candidate v' replaces incumbent v iff
//!   `score(v') > score(v)`  OR  rand < `exp(−ΔE / T(E))`.
//!   When T → 0 the acceptance becomes a pure greedy max-score selection —
//!   the set "crystallises".
//!
//! # Relationship to Singh-Boltzmann Stake
//!
//! `evaporchain-boltzmann-stake` provides `proposer_weight` (Boltzmann
//! distribution over activity) and `ValidatorStake` (decay/refresh state).
//! This crate extends that with:
//!
//! - `AnnealingParams` — carries the chain's λ and the epoch-temperature
//!   schedule.
//! - `AnnealedScore` — a merged score that feeds the SA acceptance test.
//! - `accepts_candidate` — the per-slot SA gate used by consensus to decide
//!   whether a proposed validator set change is accepted.

pub mod annealing;
pub mod score;

pub use annealing::{accepts_candidate, effective_temperature, AnnealingParams};
pub use score::{validator_score, AnnealedScore};

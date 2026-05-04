//! Singh-Inequality V2 — variance-aware Bernstein bound.
//!
//! ## Background
//!
//! V1 (`evaporchain-singh-inequality`) ships an energy-weighted
//! Hoeffding bound: σ²_H = Σ ω_i² with ω_i = (b_i − a_i)·e_i / E_max.
//! That bound only uses the range (b_i − a_i) — it is the worst-case
//! variance for a bounded random variable, achieved exactly when the
//! variable is supported only on the two endpoints with equal mass.
//!
//! Real chain data is rarely two-point: most signals concentrate
//! near the centre of their range. Bernstein's inequality
//! (Bernstein 1924) gives a strictly tighter tail bound when the
//! actual variance is known and small relative to the range:
//!
//! ```text
//!   P(|S − E[S]| ≥ ε)  ≤  2·exp( −ε² / (2σ² + (2/3)·M·ε) )
//! ```
//!
//! where σ² is the (energy-weighted) variance and M is the maximum
//! range. As σ² → 0 with M fixed and ε small, Bernstein admits
//! tighter intervals than Hoeffding (which would use σ²_H = ΣM²
//! regardless).
//!
//! ## What V2 ships
//!
//! - [`bound`] — `singh_bernstein_variance(contribs)` summing
//!   energy-weighted observed variance proxies, plus
//!   `passes_singh_bernstein_gate` enforcing the structural integer
//!   inequality `3·ε² ≥ K·(3·σ² + M·ε)`.
//! - [`compare`] — `bernstein_admits_when_hoeffding_does_not`
//!   helper for surfacing the V2 advantage to operators.
//!
//! All arithmetic is u128 with overflow guards. Pure-integer; no
//! floats anywhere on the consensus path.

pub mod bound;
pub mod compare;

pub use bound::{
    passes_singh_bernstein_gate, singh_bernstein_variance, BernsteinError,
    ContributorWithVariance,
};
pub use compare::{bernstein_strictly_tighter, BernsteinAdvantage};

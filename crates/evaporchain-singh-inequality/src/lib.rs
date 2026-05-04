//! Singh Inequality — energy-weighted Hoeffding-style tail bound.
//!
//! ## Standard Hoeffding (1963)
//!
//! For independent random variables `X_i` with `X_i ∈ [a_i, b_i]`,
//! the sum `S = Σ X_i` satisfies:
//!
//!     P(|S − E[S]| ≥ ε) ≤ 2 exp(−2ε² / Σ (b_i − a_i)²)
//!
//! The denominator `Σ (b_i − a_i)²` is the **sum of squared raw
//! ranges** — every contributor's full range counts equally.
//!
//! ## What Singh adds
//!
//! On EvaporChain, each contributor `X_i` has an associated
//! **energy** `e_i` representing how recently / firmly it was
//! attested. A decayed contributor has small `e_i`. The Singh
//! Inequality replaces the raw range `(b_i − a_i)` with an
//! energy-weighted range:
//!
//!     ω_i = (b_i − a_i) · e_i / E_max
//!
//! where `E_max = max_i e_i`. The Singh-Hoeffding bound is:
//!
//!     P(|S − E[S]| ≥ ε) ≤ 2 exp(−2ε² / Σ ω_i²)
//!
//! Properties:
//! - Each `ω_i ≤ (b_i − a_i)` so the Singh denominator is
//!   ≤ Hoeffding's. The bound is at least as TIGHT.
//! - Decayed contributors (small `e_i`) shrink ω_i toward 0,
//!   removing them from the variance contribution.
//! - At full energy (`e_i = E_max` for all i), the Singh bound
//!   collapses back to standard Hoeffding.
//!
//! ## What this crate ships
//!
//! 1. `singh_variance_bound(contributors, e_max) -> u128` —
//!    pure-integer Σ ω_i² where ω_i = (b−a)·e/E_max, all in
//!    fixed-point.
//! 2. `tail_bound_lower_estimate(eps_squared, var_bound, ...)` —
//!    a sound *lower* bound on the tail probability's negative
//!    log (we cannot evaluate `exp` validator-deterministically
//!    over arbitrary ratios; we ship the *exponent* as the
//!    hardness measure, plus a discrete decision gate).
//! 3. `passes_singh_gate(deviation, eps_squared, var_bound)` —
//!    structural gate: the deviation is admissible if
//!    `2·ε² ≥ K · variance_bound` for the chain-supplied
//!    "soundness multiplier" K.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Pure-integer arithmetic**, all u128 with checked / saturating
//!    operations. Validator-deterministic.
//!
//! 2. **Singh ≤ Hoeffding** (Singh's bound is at least as tight).
//!    Tested via direct comparison: `singh_variance_bound ≤
//!    hoeffding_variance_bound`.
//!
//! 3. **Decay collapses the bound**: as one contributor's energy
//!    drops to 0, its ω_i drops to 0, and the variance bound
//!    shrinks correspondingly.
//!
//! ## Module map
//!
//! - [`bound`] — variance-bound computation + tail-gate driver.

pub mod bound;

pub use bound::{
    hoeffding_variance_bound, passes_singh_gate, singh_variance_bound, BoundError, Contributor,
};

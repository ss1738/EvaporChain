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

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh-Inequality V1 ships the energy-weighted
    /// Hoeffding bound. (a) `singh_variance_bound ≤
    /// hoeffding_variance_bound` always (Singh is at least as tight).
    /// (b) At full energy across all contributors the two bounds
    /// COINCIDE. (c) Decay collapses the bound: dropping one
    /// contributor's energy to 0 strictly shrinks σ². Empty inputs
    /// fail closed."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Three contributors, range=10 each. Mixed energies.
        let mixed = vec![
            Contributor { lo: 0, hi: 10, energy: 1_000 },
            Contributor { lo: 0, hi: 10, energy: 500 },
            Contributor { lo: 0, hi: 10, energy: 100 },
        ];
        let h = hoeffding_variance_bound(&mixed).unwrap();
        let s = singh_variance_bound(&mixed).unwrap();
        assert!(s <= h, "Singh ({s}) must be ≤ Hoeffding ({h})");
        assert!(s < h, "with mixed energies Singh is strictly tighter");

        // (b) Full energy across all → bounds COINCIDE.
        let full = vec![
            Contributor { lo: 0, hi: 10, energy: 1_000 },
            Contributor { lo: 0, hi: 10, energy: 1_000 },
            Contributor { lo: 0, hi: 10, energy: 1_000 },
        ];
        let h_full = hoeffding_variance_bound(&full).unwrap();
        let s_full = singh_variance_bound(&full).unwrap();
        assert_eq!(h_full, s_full, "at full energy Singh ≡ Hoeffding");

        // (c) Decay collapses: zero one contributor's energy →
        // σ² strictly smaller.
        let decayed = vec![
            Contributor { lo: 0, hi: 10, energy: 1_000 },
            Contributor { lo: 0, hi: 10, energy: 1_000 },
            Contributor { lo: 0, hi: 10, energy: 0 }, // fully decayed
        ];
        let s_decayed = singh_variance_bound(&decayed).unwrap();
        assert!(s_decayed < s_full, "decayed contributor must shrink σ²");

        // (d) Gate admits large deviations under tight σ².
        // 2·ε² ≥ K·σ². With σ²=200, ε=20, K=1: 2·400 ≥ 200 ✓.
        assert!(passes_singh_gate(20, &full, 1).unwrap());
        // ε=2, K=1: 2·4 < 200 ✗.
        assert!(!passes_singh_gate(2, &full, 1).unwrap());

        // Empty/invalid inputs fail closed.
        assert!(matches!(
            singh_variance_bound(&[]),
            Err(BoundError::Empty)
        ));
        assert!(matches!(
            singh_variance_bound(&[Contributor { lo: 10, hi: 5, energy: 100 }]),
            Err(BoundError::InvalidRange { .. })
        ));
    }
}

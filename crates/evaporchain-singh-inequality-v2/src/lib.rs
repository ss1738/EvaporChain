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
    passes_singh_bernstein_gate, singh_bernstein_variance, BernsteinError, ContributorWithVariance,
};
pub use compare::{bernstein_strictly_tighter, BernsteinAdvantage};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh-Inequality V2 ships an energy-weighted
    /// Bernstein bound. When the observed variance is small, the
    /// integer gate `3·ε² ≥ K·(6·σ² + 2·M·ε)` admits deviations
    /// that the (variance-blind) Singh-Hoeffding bound would not.
    /// Popoviciu's quadratic guard is enforced; degenerate inputs
    /// fail closed."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Two low-variance contributors. (hi−lo)=10 → range²=100;
        // variance_proxy=4 (well below Popoviciu's bound of 25).
        let low_var = vec![
            ContributorWithVariance {
                lo: 0,
                hi: 10,
                energy: 1_000,
                variance_proxy: 4,
            },
            ContributorWithVariance {
                lo: 0,
                hi: 10,
                energy: 1_000,
                variance_proxy: 4,
            },
        ];

        // σ²_SB = 8 (sum of (1)² · 4 + (1)² · 4); M = 10.
        let var = singh_bernstein_variance(&low_var).unwrap();
        assert_eq!(var, 8);

        // Gate admits ε=10 with K=1: 3·100 = 300 ≥ 1·(6·8 + 2·10·10) = 248 ✓.
        assert!(passes_singh_bernstein_gate(10, &low_var, 1).unwrap());

        // Gate refuses ε=2 with K=1: 3·4 = 12 < 1·(48 + 40) = 88 ✗.
        assert!(!passes_singh_bernstein_gate(2, &low_var, 1).unwrap());

        // Popoviciu guard: variance_proxy > range² is rejected.
        let bad = vec![ContributorWithVariance {
            lo: 0,
            hi: 10,
            energy: 1_000,
            variance_proxy: 200,
        }];
        assert!(matches!(
            singh_bernstein_variance(&bad),
            Err(BernsteinError::VarianceExceedsRangeSquared { .. })
        ));

        // Empty input fails closed.
        assert!(matches!(
            singh_bernstein_variance(&[]),
            Err(BernsteinError::Empty)
        ));
    }
}

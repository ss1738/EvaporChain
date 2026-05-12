//! Bernstein bound + structural gate.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BernsteinError {
    #[error("contributors list empty")]
    Empty,
    #[error("zero E_max — at least one contributor must have positive energy")]
    ZeroEMax,
    #[error("contributor {idx} has lo > hi (range {lo}..{hi} is invalid)")]
    InvalidRange { idx: usize, lo: u64, hi: u64 },
    #[error("contributor {idx} has variance_proxy ({var}) > range² ({range_sq}) — Popoviciu's inequality violated")]
    VarianceExceedsRangeSquared {
        idx: usize,
        var: u128,
        range_sq: u128,
    },
    #[error("arithmetic overflow")]
    Overflow,
}

/// A contributor with both range and an observed/proxy variance.
///
/// `variance_proxy` is in the same scale as (hi − lo)². It must
/// satisfy Popoviciu's inequality: σ²_i ≤ (hi − lo)² / 4.
/// In the worst case (two-point support at the extremes) the
/// variance proxy equals (hi − lo)²/4; for typical chain signals it
/// is much smaller.
///
/// V2 in this crate enforces the looser bound `var ≤ (hi − lo)²`
/// (Popoviciu's quadratic guard) — chains with verified
/// concentration can pre-multiply by 4 to enforce the tighter
/// version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributorWithVariance {
    pub lo: u64,
    pub hi: u64,
    pub energy: u64,
    /// σ²_i in u128. Same scale as range².
    pub variance_proxy: u128,
}

/// Singh-weighted Bernstein variance accumulator:
///
/// ```text
///   σ²_SB = Σ (e_i / E_max)² · σ²_i
/// ```
///
/// Decayed contributors (small e_i) shrink their variance
/// contribution quadratically, just like in Singh-Hoeffding.
pub fn singh_bernstein_variance(
    contribs: &[ContributorWithVariance],
) -> Result<u128, BernsteinError> {
    if contribs.is_empty() {
        return Err(BernsteinError::Empty);
    }

    // Validate ranges + Popoviciu's inequality.
    for (i, c) in contribs.iter().enumerate() {
        if c.lo > c.hi {
            return Err(BernsteinError::InvalidRange {
                idx: i,
                lo: c.lo,
                hi: c.hi,
            });
        }
        let range = (c.hi - c.lo) as u128;
        let range_sq = range.checked_mul(range).ok_or(BernsteinError::Overflow)?;
        if c.variance_proxy > range_sq {
            return Err(BernsteinError::VarianceExceedsRangeSquared {
                idx: i,
                var: c.variance_proxy,
                range_sq,
            });
        }
    }

    let e_max: u128 = contribs.iter().map(|c| c.energy as u128).max().unwrap_or(0);
    if e_max == 0 {
        return Err(BernsteinError::ZeroEMax);
    }
    let e_max_sq = e_max.checked_mul(e_max).ok_or(BernsteinError::Overflow)?;

    let mut acc: u128 = 0;
    for c in contribs.iter() {
        let e = c.energy as u128;
        let e_sq = e.checked_mul(e).ok_or(BernsteinError::Overflow)?;
        // (e_i / E_max)² · σ²_i = (e_i² · σ²_i) / E_max²
        let weighted = e_sq
            .checked_mul(c.variance_proxy)
            .ok_or(BernsteinError::Overflow)?
            / e_max_sq;
        acc = acc.checked_add(weighted).ok_or(BernsteinError::Overflow)?;
    }
    Ok(acc)
}

/// Maximum (hi − lo) across contributors. Used as `M` in the
/// Bernstein denominator.
pub fn max_range(contribs: &[ContributorWithVariance]) -> Result<u128, BernsteinError> {
    if contribs.is_empty() {
        return Err(BernsteinError::Empty);
    }
    let mut m: u128 = 0;
    for (i, c) in contribs.iter().enumerate() {
        if c.lo > c.hi {
            return Err(BernsteinError::InvalidRange {
                idx: i,
                lo: c.lo,
                hi: c.hi,
            });
        }
        let r = (c.hi - c.lo) as u128;
        if r > m {
            m = r;
        }
    }
    Ok(m)
}

/// Structural Bernstein gate.
///
/// Bernstein's inequality says the failure probability of the
/// concentration claim is bounded by `2·exp(−ε² / (2σ² + (2/3)·M·ε))`.
/// To enforce a soundness multiplier `K` over the integer
/// arithmetic without evaluating `exp`, we require the exponent's
/// magnitude to dominate `K`-many "natural log scales", which
/// translates to:
///
/// ```text
///   ε² / ((2σ² + (2/3)·M·ε))  ≥  K
/// ⇔ 3·ε²                      ≥  K · (6·σ² + 2·M·ε)
/// ⇔ 3·ε²                      ≥  K · (6·σ² + 2·M·ε)
/// ```
///
/// Returns `Ok(true)` iff the gate admits the deviation.
pub fn passes_singh_bernstein_gate(
    deviation: u128,
    contribs: &[ContributorWithVariance],
    soundness_multiplier: u128,
) -> Result<bool, BernsteinError> {
    let var = singh_bernstein_variance(contribs)?;
    let m = max_range(contribs)?;
    let three_eps_sq = deviation
        .checked_mul(deviation)
        .ok_or(BernsteinError::Overflow)?
        .checked_mul(3)
        .ok_or(BernsteinError::Overflow)?;
    let six_sigma_sq = var.checked_mul(6).ok_or(BernsteinError::Overflow)?;
    let two_m_eps = m
        .checked_mul(2)
        .ok_or(BernsteinError::Overflow)?
        .checked_mul(deviation)
        .ok_or(BernsteinError::Overflow)?;
    let denom_sum = six_sigma_sq
        .checked_add(two_m_eps)
        .ok_or(BernsteinError::Overflow)?;
    let rhs = soundness_multiplier
        .checked_mul(denom_sum)
        .ok_or(BernsteinError::Overflow)?;
    Ok(three_eps_sq >= rhs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cv(lo: u64, hi: u64, energy: u64, var: u128) -> ContributorWithVariance {
        ContributorWithVariance {
            lo,
            hi,
            energy,
            variance_proxy: var,
        }
    }

    // ── shape errors ─────────────────────────────────────────────

    #[test]
    fn empty_rejected() {
        assert_eq!(
            singh_bernstein_variance(&[]).unwrap_err(),
            BernsteinError::Empty
        );
        assert_eq!(max_range(&[]).unwrap_err(), BernsteinError::Empty);
    }

    #[test]
    fn invalid_range_rejected() {
        let err = singh_bernstein_variance(&[cv(10, 5, 1000, 0)]).unwrap_err();
        assert!(matches!(err, BernsteinError::InvalidRange { .. }));
    }

    #[test]
    fn zero_e_max_rejected() {
        let err = singh_bernstein_variance(&[cv(0, 10, 0, 25)]).unwrap_err();
        assert_eq!(err, BernsteinError::ZeroEMax);
    }

    #[test]
    fn variance_exceeding_range_squared_rejected() {
        // range = 10 → range² = 100. variance_proxy = 200 > 100.
        let err = singh_bernstein_variance(&[cv(0, 10, 1000, 200)]).unwrap_err();
        assert!(matches!(
            err,
            BernsteinError::VarianceExceedsRangeSquared { .. }
        ));
    }

    // ── basic computation ────────────────────────────────────────

    #[test]
    fn singh_bernstein_one_contributor_at_max_energy() {
        // (e/E)² = 1; σ²_proxy = 25.
        let r = singh_bernstein_variance(&[cv(0, 10, 1000, 25)]).unwrap();
        assert_eq!(r, 25);
    }

    #[test]
    fn singh_bernstein_decayed_contributor_shrinks_quadratically() {
        // Anchor E_max with a fresh contributor (var=0, energy=1000).
        // Decayed contributor has energy = E_max/2 → weight = 1/4 →
        // 25 · 1/4 = 6 (integer floor). Total = 6 + 0 = 6.
        let r = singh_bernstein_variance(&[cv(0, 10, 500, 25), cv(0, 10, 1000, 0)]).unwrap();
        assert_eq!(r, 6);
    }

    #[test]
    fn singh_bernstein_zero_variance_yields_zero() {
        // Concentrated signal (e.g. constant) → variance 0 → bound 0.
        let r = singh_bernstein_variance(&[cv(0, 10, 1000, 0), cv(0, 10, 1000, 0)]).unwrap();
        assert_eq!(r, 0);
    }

    // ── Bernstein vs Hoeffding (the V2 win) ──────────────────────

    #[test]
    fn bernstein_strictly_smaller_when_variance_below_range_sq() {
        // Hoeffding (V1) bound: Σ range² weighted by (e/E)².
        // Bernstein (V2) bound: Σ var weighted by (e/E)².
        // With var << range² and uniform energy, V2 ≪ V1.
        let contribs = vec![cv(0, 10, 1000, 4); 5]; // range=10, var=4
        let v2 = singh_bernstein_variance(&contribs).unwrap();
        // V1 analogue: each contributor adds range²·(e/E)² = 100.
        let v1_analogue: u128 = 5 * 100;
        // V2 analogue: each contributor adds var·(e/E)² = 4.
        assert_eq!(v2, 5 * 4);
        assert!(v2 < v1_analogue);
    }

    // ── gate behaviour ───────────────────────────────────────────

    #[test]
    fn gate_admits_when_deviation_dominates() {
        // Big ε, modest σ² and M → 3ε² ≥ K·(6σ² + 2Mε).
        // ε = 100, σ² = 25, M = 10, K = 1.
        // 3·10000 = 30000. 6·25 + 2·10·100 = 150 + 2000 = 2150.
        // 30000 ≥ 2150 → admit.
        let contribs = vec![cv(0, 10, 1000, 25)];
        assert!(passes_singh_bernstein_gate(100, &contribs, 1).unwrap());
    }

    #[test]
    fn gate_rejects_when_variance_dominates() {
        // ε = 1, σ² = 25, M = 10, K = 1.
        // 3·1 = 3. 6·25 + 2·10·1 = 170. 3 < 170 → reject.
        let contribs = vec![cv(0, 10, 1000, 25)];
        assert!(!passes_singh_bernstein_gate(1, &contribs, 1).unwrap());
    }

    #[test]
    fn gate_at_zero_variance_dominated_by_linear_term() {
        // σ² = 0 (concentrated signal), M = 10. Gate becomes:
        // 3·ε² ≥ K · 2·M·ε  ⇔  3·ε ≥ 2·M·K  (after dividing by ε > 0)
        // ε = 7, M = 10, K = 1: 3·7 = 21. 2·10 = 20. 21 ≥ 20 → admit.
        let contribs = vec![cv(0, 10, 1000, 0)];
        assert!(passes_singh_bernstein_gate(7, &contribs, 1).unwrap());
        // ε = 6: 3·6 = 18. 2·10 = 20. 18 < 20 → reject.
        assert!(!passes_singh_bernstein_gate(6, &contribs, 1).unwrap());
    }

    #[test]
    fn max_range_returns_largest_range() {
        let m = max_range(&[cv(0, 5, 100, 0), cv(0, 20, 100, 0), cv(10, 15, 100, 0)]).unwrap();
        assert_eq!(m, 20);
    }

    proptest::proptest! {
        #[test]
        fn property_zero_variance_yields_zero_bound(
            r0 in 0u64..50u64,
            r1 in 0u64..50u64,
            e0 in 1u64..1000u64,
            e1 in 1u64..1000u64,
        ) {
            let contribs = vec![cv(0, r0, e0, 0), cv(0, r1, e1, 0)];
            let v = singh_bernstein_variance(&contribs).unwrap();
            proptest::prop_assert_eq!(v, 0);
        }

        #[test]
        fn property_more_energy_means_larger_bound(
            r in 1u64..50u64,
            v in 0u128..2500u128,
            e_low in 1u64..500u64,
        ) {
            // Doubling a single contributor's energy quadruples its
            // (e/E)² weight when E_max stays fixed by another
            // contributor. Test with two contributors: one fixed at
            // E_max=1000, other goes from e_low to 2*e_low.
            let r_sq = (r as u128) * (r as u128);
            // Clamp variance to range² to satisfy Popoviciu.
            let v = v.min(r_sq);
            let e_high = (e_low * 2).min(1000);
            let contribs_low = vec![cv(0, r, e_low, v), cv(0, r, 1000, 0)];
            let contribs_high = vec![cv(0, r, e_high, v), cv(0, r, 1000, 0)];
            let bound_low = singh_bernstein_variance(&contribs_low).unwrap();
            let bound_high = singh_bernstein_variance(&contribs_high).unwrap();
            proptest::prop_assert!(bound_high >= bound_low);
        }
    }

    /// T1.20 — max_range error paths: empty input + invalid range
    /// (lines 102-113).
    #[test]
    fn t1_20_max_range_empty_and_invalid_range() {
        assert_eq!(max_range(&[]).unwrap_err(), BernsteinError::Empty);
        let err = max_range(&[cv(50, 10, 100, 0)]).unwrap_err();
        match err {
            BernsteinError::InvalidRange { idx, lo, hi } => {
                assert_eq!(idx, 0);
                assert_eq!(lo, 50);
                assert_eq!(hi, 10);
            }
            other => panic!("expected InvalidRange, got {other:?}"),
        }
    }
}

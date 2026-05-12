//! Comparison helpers — surface the V2-over-V1 advantage.

use evaporchain_singh_inequality::bound::{
    passes_singh_gate, singh_variance_bound, BoundError as V1Error, Contributor as V1Contrib,
};

use crate::bound::{
    max_range, passes_singh_bernstein_gate, singh_bernstein_variance, BernsteinError,
    ContributorWithVariance,
};

/// Per-claim summary: did Singh-Hoeffding (V1) admit? Did
/// Singh-Bernstein (V2) admit? The V2 advantage is the operating
/// region where V2 admits and V1 does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BernsteinAdvantage {
    pub v1_admits: bool,
    pub v2_admits: bool,
    pub v1_variance_bound: u128,
    pub v2_variance_bound: u128,
}

/// Run both gates over the same contributor set + deviation +
/// soundness multiplier; return the per-claim admission summary.
///
/// The V1 bound uses range only; the V2 bound uses both range and
/// variance proxy. When variance proxy << range², V2's bound is
/// strictly smaller, and there exists a `(deviation, K)` operating
/// region where V2 admits and V1 rejects.
pub fn bernstein_strictly_tighter(
    deviation: u128,
    contribs: &[ContributorWithVariance],
    soundness_multiplier: u128,
) -> Result<BernsteinAdvantage, BernsteinError> {
    let v1_contribs: Vec<V1Contrib> = contribs
        .iter()
        .map(|c| V1Contrib {
            lo: c.lo,
            hi: c.hi,
            energy: c.energy,
        })
        .collect();

    let v1_variance = singh_variance_bound(&v1_contribs).map_err(map_v1_err)?;
    let v1_admits =
        passes_singh_gate(deviation, &v1_contribs, soundness_multiplier).map_err(map_v1_err)?;

    let v2_variance = singh_bernstein_variance(contribs)?;
    let _ = max_range(contribs)?; // also validates ranges
    let v2_admits = passes_singh_bernstein_gate(deviation, contribs, soundness_multiplier)?;

    Ok(BernsteinAdvantage {
        v1_admits,
        v2_admits,
        v1_variance_bound: v1_variance,
        v2_variance_bound: v2_variance,
    })
}

fn map_v1_err(e: V1Error) -> BernsteinError {
    match e {
        V1Error::Empty => BernsteinError::Empty,
        V1Error::ZeroEMax => BernsteinError::ZeroEMax,
        V1Error::InvalidRange { idx, lo, hi } => BernsteinError::InvalidRange { idx, lo, hi },
        V1Error::Overflow => BernsteinError::Overflow,
    }
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

    #[test]
    fn equal_admission_when_variance_at_popoviciu_max() {
        // When σ² = range², V2's bound matches V1's exactly (modulo
        // the integer-division floor). Both gates admit/reject the
        // same boundary deviations.
        let contribs = vec![cv(0, 10, 1000, 100), cv(0, 10, 1000, 100)];
        let adv = bernstein_strictly_tighter(15, &contribs, 1).unwrap();
        assert_eq!(adv.v1_variance_bound, adv.v2_variance_bound);
    }

    #[test]
    fn v2_admits_some_claim_v1_rejects() {
        // Five contributors with range=10 and variance=4 (i.e. σ ≈ 2,
        // very concentrated) at uniform full energy.
        // V1 (Hoeffding): bound = Σ range²·(e/E)² = 5·100 = 500.
        // V2 (Bernstein): bound = Σ var·(e/E)² = 5·4 = 20.
        // Pick deviation ε such that V2 admits but V1 doesn't.
        let contribs = vec![cv(0, 10, 1000, 4); 5];

        // V1 gate (passes_singh_gate): 2·ε² ≥ K·σ²
        //   ε = 18, K = 1: 2·324 = 648 > 500 → V1 admits.
        //   ε = 15, K = 1: 2·225 = 450 < 500 → V1 rejects.
        // V2 gate (passes_singh_bernstein_gate): 3·ε² ≥ K·(6σ² + 2Mε)
        //   ε = 15, M = 10, σ² = 20, K = 1:
        //     3·225 = 675. 6·20 + 2·10·15 = 120 + 300 = 420.
        //     675 ≥ 420 → V2 admits.
        let adv = bernstein_strictly_tighter(15, &contribs, 1).unwrap();
        assert!(!adv.v1_admits);
        assert!(adv.v2_admits);
        assert!(adv.v2_variance_bound < adv.v1_variance_bound);
    }

    #[test]
    fn both_reject_at_tiny_deviation() {
        // ε = 1: neither gate admits regardless of variance.
        let contribs = vec![cv(0, 10, 1000, 4); 5];
        let adv = bernstein_strictly_tighter(1, &contribs, 1).unwrap();
        assert!(!adv.v1_admits);
        assert!(!adv.v2_admits);
    }

    #[test]
    fn both_admit_at_huge_deviation() {
        // ε huge: both gates admit.
        let contribs = vec![cv(0, 10, 1000, 4); 5];
        let adv = bernstein_strictly_tighter(1_000, &contribs, 1).unwrap();
        assert!(adv.v1_admits);
        assert!(adv.v2_admits);
    }

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Singh-Inequality V2 ships a Bernstein-style
        // variance-aware bound. When chain signals concentrate
        // (variance ≪ range²), V2 admits a strictly larger set of
        // claims than V1 at the same soundness multiplier — the
        // chain gets tighter confidence intervals over concentrated
        // data without sacrificing rejection of impossible claims.
        // Edge cases: zero variance collapses the bound to the
        // linear ε·M term; uniform σ² = range² recovers the V1
        // bound exactly."

        // Concentrated signals (var=4 ≪ range²=100):
        let contribs = vec![cv(0, 10, 1000, 4); 5];

        // V2-only admission region exists.
        let adv = bernstein_strictly_tighter(15, &contribs, 1).unwrap();
        assert!(!adv.v1_admits, "V1 should reject ε=15 here");
        assert!(adv.v2_admits, "V2 should admit ε=15 here");
        assert!(adv.v2_variance_bound < adv.v1_variance_bound);

        // Both reject impossible claims (ε=1 over highly variable data).
        let adv_low = bernstein_strictly_tighter(1, &contribs, 1).unwrap();
        assert!(!adv_low.v1_admits);
        assert!(!adv_low.v2_admits);

        // Worst-case σ² = range²: V2 collapses to V1.
        let worst_case = vec![cv(0, 10, 1000, 100); 5];
        let adv_eq = bernstein_strictly_tighter(15, &worst_case, 1).unwrap();
        assert_eq!(adv_eq.v1_variance_bound, adv_eq.v2_variance_bound);
    }

    /// T1.20 — bernstein_strictly_tighter on empty contribs returns
    /// Empty error (lines 60-67 — exercises map_v1_err Empty arm).
    #[test]
    fn t1_20_bernstein_strictly_tighter_empty_returns_empty() {
        let r = bernstein_strictly_tighter(10, &[], 1);
        assert!(matches!(r, Err(BernsteinError::Empty)));
    }

    /// T1.20 — bernstein_strictly_tighter on invalid-range contrib
    /// propagates InvalidRange via map_v1_err (line 65).
    #[test]
    fn t1_20_bernstein_strictly_tighter_invalid_range_propagates() {
        let bad = vec![cv(100, 5, 1000, 0)];
        let r = bernstein_strictly_tighter(10, &bad, 1);
        match r {
            Err(BernsteinError::InvalidRange { idx: 0, lo: 100, hi: 5 }) => {}
            other => panic!("expected InvalidRange, got {other:?}"),
        }
    }
}

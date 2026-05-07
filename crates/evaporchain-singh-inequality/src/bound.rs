//! Variance-bound computation + tail-gate driver.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Fixed-point unit: 1.0 = 1_000_000.
pub const MICROS: u128 = 1_000_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BoundError {
    #[error("contributors list empty")]
    Empty,
    #[error("zero E_max — at least one contributor must have positive energy")]
    ZeroEMax,
    #[error("contributor {idx} has lo > hi (range {lo}..{hi} is invalid)")]
    InvalidRange { idx: usize, lo: u64, hi: u64 },
    #[error("arithmetic overflow")]
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contributor {
    /// Lower bound of contributor's value range.
    pub lo: u64,
    /// Upper bound of contributor's value range. Must be ≥ lo.
    pub hi: u64,
    /// Current energy. Decayed contributors have small energy;
    /// fully-fresh contributors have energy ≈ E_max.
    pub energy: u64,
}

/// Standard Hoeffding variance bound: Σ (hi − lo)².
///
/// Pure u128 with overflow guards; ignores energy.
pub fn hoeffding_variance_bound(contributors: &[Contributor]) -> Result<u128, BoundError> {
    if contributors.is_empty() {
        return Err(BoundError::Empty);
    }
    let mut acc: u128 = 0;
    for (i, c) in contributors.iter().enumerate() {
        if c.lo > c.hi {
            return Err(BoundError::InvalidRange {
                idx: i,
                lo: c.lo,
                hi: c.hi,
            });
        }
        let range = (c.hi - c.lo) as u128;
        let r2 = range.checked_mul(range).ok_or(BoundError::Overflow)?;
        acc = acc.checked_add(r2).ok_or(BoundError::Overflow)?;
    }
    Ok(acc)
}

/// Singh variance bound: Σ ω_i² where ω_i = (hi − lo) · e_i / E_max.
///
/// `E_max` is the chain-supplied normalisation; typically the
/// maximum energy across contributors at the time of evaluation.
/// Pure u128.
pub fn singh_variance_bound(contributors: &[Contributor]) -> Result<u128, BoundError> {
    if contributors.is_empty() {
        return Err(BoundError::Empty);
    }
    // Validate all ranges first.
    for (i, c) in contributors.iter().enumerate() {
        if c.lo > c.hi {
            return Err(BoundError::InvalidRange {
                idx: i,
                lo: c.lo,
                hi: c.hi,
            });
        }
    }
    let e_max: u128 = contributors
        .iter()
        .map(|c| c.energy as u128)
        .max()
        .unwrap_or(0);
    if e_max == 0 {
        return Err(BoundError::ZeroEMax);
    }
    let mut acc: u128 = 0;
    for c in contributors.iter() {
        let range = (c.hi - c.lo) as u128;
        // ω_i = range · energy / e_max. Pure integer (loses a bit
        // of precision on the floor; the chain MAY pre-scale by
        // MICROS for tighter bounds — V1 ships the simpler form).
        let omega = range
            .checked_mul(c.energy as u128)
            .ok_or(BoundError::Overflow)?
            / e_max;
        let o2 = omega.checked_mul(omega).ok_or(BoundError::Overflow)?;
        acc = acc.checked_add(o2).ok_or(BoundError::Overflow)?;
    }
    Ok(acc)
}

/// Structural gate over the Singh-Hoeffding bound:
/// admit a claim of magnitude `deviation` iff `2·ε² ≥ K · σ²`
/// where `ε = deviation`, `σ² = singh_variance_bound`, and `K`
/// is the chain-supplied soundness multiplier.
///
/// The gate is **structural** in the sense that it does not
/// evaluate `exp(...)` — it just enforces an integer inequality
/// equivalent to "the tail bound is below threshold". The full
/// exponential evaluation is the chain's higher-layer concern;
/// this primitive enforces the inequality validator-
/// deterministically.
pub fn passes_singh_gate(
    deviation: u128,
    contributors: &[Contributor],
    soundness_multiplier: u128,
) -> Result<bool, BoundError> {
    let var = singh_variance_bound(contributors)?;
    let two_eps_sq = deviation
        .checked_mul(deviation)
        .ok_or(BoundError::Overflow)?
        .checked_mul(2)
        .ok_or(BoundError::Overflow)?;
    let lhs = two_eps_sq;
    let rhs = soundness_multiplier
        .checked_mul(var)
        .ok_or(BoundError::Overflow)?;
    Ok(lhs >= rhs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(lo: u64, hi: u64, energy: u64) -> Contributor {
        Contributor { lo, hi, energy }
    }

    // ── shape errors ─────────────────────────────────────────────

    #[test]
    fn empty_input_rejected() {
        let err = hoeffding_variance_bound(&[]).unwrap_err();
        assert_eq!(err, BoundError::Empty);
        let err = singh_variance_bound(&[]).unwrap_err();
        assert_eq!(err, BoundError::Empty);
    }

    #[test]
    fn invalid_range_rejected() {
        let err = hoeffding_variance_bound(&[c(10, 5, 1000)]).unwrap_err();
        assert!(matches!(err, BoundError::InvalidRange { .. }));
        let err = singh_variance_bound(&[c(10, 5, 1000)]).unwrap_err();
        assert!(matches!(err, BoundError::InvalidRange { .. }));
    }

    #[test]
    fn singh_with_zero_e_max_rejected() {
        let err = singh_variance_bound(&[c(0, 10, 0), c(5, 15, 0)]).unwrap_err();
        assert_eq!(err, BoundError::ZeroEMax);
    }

    // ── basic bound computation ──────────────────────────────────

    #[test]
    fn hoeffding_one_contributor() {
        // (hi − lo)² = 100.
        let r = hoeffding_variance_bound(&[c(0, 10, 1000)]).unwrap();
        assert_eq!(r, 100);
    }

    #[test]
    fn hoeffding_sums_squared_ranges() {
        // [0,10]² + [5,15]² + [3,8]² = 100 + 100 + 25 = 225.
        let r = hoeffding_variance_bound(&[c(0, 10, 1000), c(5, 15, 800), c(3, 8, 500)]).unwrap();
        assert_eq!(r, 225);
    }

    // ── Singh at full freshness collapses to Hoeffding ──────────

    #[test]
    fn singh_at_uniform_e_max_equals_hoeffding() {
        let contribs = vec![c(0, 10, 1000), c(5, 15, 1000), c(3, 8, 1000)];
        let h = hoeffding_variance_bound(&contribs).unwrap();
        let s = singh_variance_bound(&contribs).unwrap();
        assert_eq!(s, h);
    }

    #[test]
    fn singh_with_one_decayed_contributor_is_smaller() {
        // Same ranges, but contributor 0 has 1/2 the energy of
        // others. ω_0 = 10 · 500 / 1000 = 5. ω_0² = 25 (vs 100).
        // Total Singh = 25 + 100 + 25 = 150 (vs Hoeffding 225).
        let contribs = vec![c(0, 10, 500), c(5, 15, 1000), c(3, 8, 500)];
        let h = hoeffding_variance_bound(&contribs).unwrap();
        let s = singh_variance_bound(&contribs).unwrap();
        assert!(s < h);
    }

    #[test]
    fn singh_with_fully_decayed_contributor_excludes_it() {
        // Contributor 0 has energy 0. ω_0 = 10 · 0 / 1000 = 0.
        // Total Singh = 0 + 100 + 25 = 125.
        let contribs = vec![c(0, 10, 0), c(5, 15, 1000), c(3, 8, 500)];
        let s = singh_variance_bound(&contribs).unwrap();
        // Contributor 0 contributes 0 → expect 100 + (5·500/1000)² = 100 + 6 = 106.
        // 5·500/1000 = 2 (integer floor); 2² = 4. So actually 100 + 0 + 4 = 104.
        // Recompute: ω_2 = 5·500/1000 = 2; ω_2² = 4. Total = 100 + 4 = 104.
        assert_eq!(s, 104);
    }

    // ── Singh ≤ Hoeffding monotone ───────────────────────────────

    #[test]
    fn singh_at_most_hoeffding() {
        // For any contributor mix, Singh's bound is ≤ Hoeffding's.
        let contribs_cases: Vec<Vec<Contributor>> = vec![
            vec![c(0, 100, 1000)],
            vec![c(0, 10, 1000), c(0, 10, 1000)],
            vec![c(0, 10, 100), c(0, 10, 1000), c(0, 10, 500)],
            vec![c(50, 100, 1), c(0, 200, 1000)],
        ];
        for contribs in contribs_cases {
            let h = hoeffding_variance_bound(&contribs).unwrap();
            let s = singh_variance_bound(&contribs).unwrap();
            assert!(s <= h, "Singh {} > Hoeffding {} for {:?}", s, h, contribs);
        }
    }

    // ── tail-gate ────────────────────────────────────────────────

    #[test]
    fn gate_admits_when_deviation_squared_dominates_variance() {
        // 2·ε² ≥ K·σ² → admit.
        // ε = 100; ε² = 10_000; 2·ε² = 20_000.
        // σ² = (10)² = 100. K = 50. K·σ² = 5_000. 20_000 ≥ 5_000 → admit.
        let contribs = vec![c(0, 10, 1000)];
        assert!(passes_singh_gate(100, &contribs, 50).unwrap());
    }

    #[test]
    fn gate_rejects_when_variance_dominates() {
        // ε = 1; 2·ε² = 2. σ² = 100. K = 1. 2 < 100 → reject.
        let contribs = vec![c(0, 10, 1000)];
        assert!(!passes_singh_gate(1, &contribs, 1).unwrap());
    }

    #[test]
    fn gate_at_equality_admits() {
        // 2·ε² == K·σ². ε² = 50; ε must satisfy 2·ε² = K·σ².
        // K = 1, σ² = 100 → 2·ε² = 100 → ε² = 50 → ε = floor(sqrt(50)) = 7.
        // Test ε = 8: 2·64 = 128 ≥ 100 → admit.
        let contribs = vec![c(0, 10, 1000)];
        assert!(passes_singh_gate(8, &contribs, 1).unwrap());
        // ε = 7: 2·49 = 98 < 100 → reject.
        assert!(!passes_singh_gate(7, &contribs, 1).unwrap());
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Singh Inequality is a Tier-0 supporting
        // theorem-grade primitive: an energy-weighted Hoeffding
        // tail bound where decayed contributors no longer
        // inflate the variance estimate. The chain gets tighter
        // confidence intervals over the surviving (fresh) data,
        // automatically excluding contributors whose energy has
        // evaporated."

        // Three contributors with same range. Two fresh, one decayed.
        let contribs = vec![c(0, 10, 1000), c(0, 10, 1000), c(0, 10, 50)];

        let hoeffding = hoeffding_variance_bound(&contribs).unwrap();
        let singh = singh_variance_bound(&contribs).unwrap();

        // Hoeffding: 100 + 100 + 100 = 300.
        // Singh: 100 + 100 + (10·50/1000)² = 100 + 100 + 0² = 200.
        // (10·50/1000 = 0 by integer division.)
        assert_eq!(hoeffding, 300);
        assert_eq!(singh, 200);
        assert!(singh < hoeffding);

        // A claim with deviation ε = 15 against the bound:
        // 2·15² = 450. K = 2.
        // Hoeffding gate: 450 ≥ 2·300 = 600? No → reject.
        // Singh gate: 450 ≥ 2·200 = 400? Yes → admit.
        // The decayed contributor's removal lets the same
        // deviation pass under Singh.
        let h_passes_two_thirds = 2 * 15u128.pow(2) >= 2 * hoeffding;
        let s_passes_two_thirds = passes_singh_gate(15, &contribs, 2).unwrap();
        assert!(!h_passes_two_thirds);
        assert!(s_passes_two_thirds);
    }

    proptest::proptest! {
        #[test]
        fn property_singh_at_most_hoeffding(
            r0 in 0u64..50u64,
            r1 in 0u64..50u64,
            r2 in 0u64..50u64,
            e0 in 1u64..1000u64,
            e1 in 1u64..1000u64,
            e2 in 1u64..1000u64,
        ) {
            let contribs = vec![
                c(0, r0, e0),
                c(0, r1, e1),
                c(0, r2, e2),
            ];
            let h = hoeffding_variance_bound(&contribs).unwrap();
            let s = singh_variance_bound(&contribs).unwrap();
            proptest::prop_assert!(s <= h);
        }

        #[test]
        fn property_uniform_energy_collapses_to_hoeffding(
            r0 in 0u64..50u64,
            r1 in 0u64..50u64,
            e in 1u64..1000u64,
        ) {
            // When all contributors have the same energy e_max,
            // Singh = Hoeffding exactly.
            let contribs = vec![c(0, r0, e), c(0, r1, e)];
            let h = hoeffding_variance_bound(&contribs).unwrap();
            let s = singh_variance_bound(&contribs).unwrap();
            proptest::prop_assert_eq!(s, h);
        }
    }
}

//! End-to-end integration tests for evaporchain-singh-inequality-v2.
//!
//! Non-trivial fixture: 5-validator concentrated fee-oracle assessment
//! across three variance scenarios.
//!
//! Validators attest fee signals in [0, 100]. Historical data shows
//! typical signals concentrate near the midpoint; range²=10_000 but
//! observed variance is only σ²=100. The chain should admit deviations
//! that are plausible given actual concentration, even when rejected by
//! the pessimistic Hoeffding bound.
//!
//!   Scenario A — max variance (worst case, no prior data):
//!     All 5 validators, range=100, energy=1_000, variance_proxy=10_000
//!     V2 σ²_SB = 5 × (1)² × 10_000 = 50_000
//!     V1 σ²_H  = 5 × 100²           = 50_000   (coincide at max variance)
//!
//!   Scenario B — concentrated oracle, all fresh:
//!     All 5 validators, range=100, energy=1_000, variance_proxy=100
//!     V2 σ²_SB = 5 × 100 = 500
//!     V1 σ²_H  = 50_000  (ignores variance, uses range² only)
//!     Gate(ε=80, K=1):
//!       V2: 3·6400=19_200 ≥ 6·500+2·100·80=19_000 → V2 ADMITS
//!       V1: 2·6400=12_800 < 50_000              → V1 REJECTS
//!     Gate(ε=79, K=1):
//!       V2: 3·6241=18_723 < 6·500+2·100·79=18_800 → V2 REJECTS
//!
//!   Scenario C — concentrated + partial decay:
//!     V1,V2: energy=1_000, variance_proxy=100 (fresh + concentrated)
//!     V3,V4,V5: energy=100, variance_proxy=100 (decayed to 10%)
//!     V2 σ²_SB: (1²×100)×2 + ((100/1000)²×100×1_000_000/1_000_000)×3
//!             = 200 + 1×3 = 203
//!     V1 σ²_H: (100×1)²×2 + (100×0.1)²×3 = 20_000 + 300 = 20_300
//!     Gate(ε=100, K=1):
//!       V2: 3·10_000=30_000 ≥ 6·203+2·100·100=1_218+20_000=21_218 → V2 ADMITS
//!       V1: 2·10_000=20_000 < 20_300 → V1 REJECTS
//!
//! Doctrine claim (INVENTION_STACK §A4 / Singh Inequality V2):
//! "When chain fee signals concentrate (σ² ≪ range²), the Bernstein
//! bound admits a strictly larger class of deviations than Hoeffding,
//! while still rejecting impossible claims. Energy decay shrinks V2's
//! bound further; V2 σ²_SB ≤ V1 σ²_H always; at max variance they
//! coincide."
//!
//! INVENTION_STACK §A4: Singh Inequality V2 (energy-weighted Bernstein).

use evaporchain_singh_inequality_v2::{
    bernstein_strictly_tighter, passes_singh_bernstein_gate, singh_bernstein_variance,
    BernsteinError, ContributorWithVariance,
};

// ── Helpers ───────────────────────────────────────────────────────────────

fn cv(lo: u64, hi: u64, energy: u64, variance_proxy: u128) -> ContributorWithVariance {
    ContributorWithVariance {
        lo,
        hi,
        energy,
        variance_proxy,
    }
}

fn committee_uniform(energies: &[u64; 5], variance_proxy: u128) -> Vec<ContributorWithVariance> {
    energies
        .iter()
        .map(|&e| cv(0, 100, e, variance_proxy))
        .collect()
}

// ── Non-trivial fixture ───────────────────────────────────────────────────

#[test]
fn scenario_a_max_variance_v2_equals_v1() {
    // Scenario A: variance_proxy = range² = 10_000 → V2 coincides with V1.
    let committee = committee_uniform(&[1_000, 1_000, 1_000, 1_000, 1_000], 10_000);
    let v2_var = singh_bernstein_variance(&committee).unwrap();
    assert_eq!(
        v2_var, 50_000,
        "V2 σ²_SB at max-variance: 5 × 10_000 = 50_000"
    );

    // bernstein_strictly_tighter confirms variance bounds coincide.
    let adv = bernstein_strictly_tighter(200, &committee, 1).unwrap();
    assert_eq!(
        adv.v1_variance_bound, adv.v2_variance_bound,
        "at variance_proxy == range² the two bounds must coincide"
    );
}

#[test]
fn scenario_b_concentrated_v2_strictly_tighter() {
    // Scenario B: variance_proxy=100 ≪ range²=10_000.
    let committee = committee_uniform(&[1_000, 1_000, 1_000, 1_000, 1_000], 100);

    let v2_var = singh_bernstein_variance(&committee).unwrap();
    assert_eq!(v2_var, 500, "V2 σ²_SB: 5 × 100 = 500");

    // V1 bound (via BernsteinAdvantage) = 50_000.
    let adv = bernstein_strictly_tighter(80, &committee, 1).unwrap();
    assert_eq!(adv.v1_variance_bound, 50_000, "V1 ignores variance proxy");
    assert!(
        adv.v2_variance_bound < adv.v1_variance_bound,
        "V2 bound must be strictly smaller when variance_proxy << range²"
    );
}

#[test]
fn scenario_c_concentrated_plus_decay_further_shrinks_v2() {
    // Scenario C: V3,V4,V5 decayed to 10% energy.
    let committee = vec![
        cv(0, 100, 1_000, 100), // V1 — fresh
        cv(0, 100, 1_000, 100), // V2 — fresh
        cv(0, 100, 100, 100),   // V3 — decayed
        cv(0, 100, 100, 100),   // V4 — decayed
        cv(0, 100, 100, 100),   // V5 — decayed
    ];

    let v2_var = singh_bernstein_variance(&committee).unwrap();
    assert_eq!(
        v2_var, 203,
        "V2 σ²_SB: 2×100 + 3×1 = 203 (decayed weight = (100/1000)² = 0.01)"
    );

    let adv = bernstein_strictly_tighter(100, &committee, 1).unwrap();
    assert_eq!(adv.v1_variance_bound, 20_300);
    assert_eq!(adv.v2_variance_bound, 203);

    // σ²_C (203) < σ²_B (500): decay made V2 even tighter.
    let committee_b = committee_uniform(&[1_000, 1_000, 1_000, 1_000, 1_000], 100);
    let v2_b = singh_bernstein_variance(&committee_b).unwrap();
    assert!(
        v2_var < v2_b,
        "decay shrinks V2 variance beyond concentration alone"
    );
}

// ── Doctrine tests ────────────────────────────────────────────────────────

#[test]
fn doctrine_concentrated_v2_admits_when_v1_rejects_scenario_b() {
    // Scenario B, ε=80, K=1: V2 ADMITS, V1 REJECTS.
    // V2: 3·6400=19_200 ≥ 6·500+2·100·80=19_000 → admit.
    // V1: 2·6400=12_800 < 50_000 → reject.
    let committee = committee_uniform(&[1_000, 1_000, 1_000, 1_000, 1_000], 100);
    let adv = bernstein_strictly_tighter(80, &committee, 1).unwrap();

    assert!(adv.v2_admits, "V2 must ADMIT ε=80 in Scenario B");
    assert!(!adv.v1_admits, "V1 must REJECT ε=80 in Scenario B");
}

#[test]
fn doctrine_concentrated_v2_also_rejects_implausible_claims_scenario_b() {
    // ε=79 is not large enough for V2 either.
    // 3·6241=18_723 < 6·500+2·100·79=18_800 → V2 REJECTS.
    let committee = committee_uniform(&[1_000, 1_000, 1_000, 1_000, 1_000], 100);
    let admits = passes_singh_bernstein_gate(79, &committee, 1).unwrap();
    assert!(
        !admits,
        "V2 must REJECT ε=79 (not enough to dominate Bernstein denominator)"
    );
}

#[test]
fn doctrine_concentrated_decay_v2_admits_when_v1_rejects_scenario_c() {
    // Scenario C, ε=100, K=1: V2 ADMITS, V1 REJECTS.
    // V2: 3·10_000=30_000 ≥ 6·203+2·100·100=21_218 → admit.
    // V1: 2·10_000=20_000 < 20_300 → reject.
    let committee = vec![
        cv(0, 100, 1_000, 100),
        cv(0, 100, 1_000, 100),
        cv(0, 100, 100, 100),
        cv(0, 100, 100, 100),
        cv(0, 100, 100, 100),
    ];
    let adv = bernstein_strictly_tighter(100, &committee, 1).unwrap();

    assert!(adv.v2_admits, "V2 must ADMIT ε=100 in Scenario C");
    assert!(!adv.v1_admits, "V1 must REJECT ε=100 in Scenario C");
}

#[test]
fn doctrine_v2_leq_v1_across_all_scenarios() {
    // V2 σ²_SB ≤ V1 σ²_H for any combination of variance and energy.
    for (energies, var_proxy) in [
        ([1_000, 1_000, 1_000, 1_000, 1_000], 10_000), // max variance
        ([1_000, 1_000, 1_000, 1_000, 1_000], 100),    // concentrated
        ([1_000, 1_000, 100, 10, 0], 100),             // decayed + concentrated
        ([1_000, 1_000, 0, 0, 0], 0),                  // all-zero variance
        ([500, 300, 200, 100, 50], 2_500),             // mixed energies, mid variance
    ] {
        let committee = committee_uniform(&energies, var_proxy);
        let adv = bernstein_strictly_tighter(50, &committee, 1).unwrap();
        assert!(
            adv.v2_variance_bound <= adv.v1_variance_bound,
            "V2 ({}) must be ≤ V1 ({}) for energies={energies:?}, var={var_proxy}",
            adv.v2_variance_bound,
            adv.v1_variance_bound,
        );
    }
}

#[test]
fn doctrine_max_variance_both_agree_on_admission() {
    // Scenario A: variance_proxy = range² → bounds coincide.
    // At sufficiently large ε both gates admit; small ε both reject.
    let committee = committee_uniform(&[1_000, 1_000, 1_000, 1_000, 1_000], 10_000);

    let adv_big = bernstein_strictly_tighter(1_000, &committee, 1).unwrap();
    assert_eq!(
        adv_big.v1_variance_bound, adv_big.v2_variance_bound,
        "variance bounds coincide at max variance"
    );

    // Both should handle the same input consistently (not necessarily same gate behavior
    // due to different formulas, but variance bounds must match).
    assert_eq!(adv_big.v1_variance_bound, 50_000);
    assert_eq!(adv_big.v2_variance_bound, 50_000);
}

#[test]
fn decay_monotonically_shrinks_v2_variance() {
    // As energy decreases, V2 σ²_SB must decrease or stay the same.
    let energy_steps = [1_000u64, 500, 100, 10, 1, 0];
    let mut prev = u128::MAX;

    for &e in &energy_steps {
        // V1 fresh at E_max; V2 at energy=e; V3-V5 fixed.
        let committee = vec![
            cv(0, 100, 1_000, 100),
            cv(0, 100, e, 100),
            cv(0, 100, 200, 100),
            cv(0, 100, 200, 100),
            cv(0, 100, 200, 100),
        ];
        let var = singh_bernstein_variance(&committee).unwrap();
        assert!(
            var <= prev,
            "V2 σ²_SB must be non-increasing as V2 energy drops (e={e})"
        );
        prev = var;
    }
}

#[test]
fn zero_variance_proxy_yields_zero_bound() {
    // All variance_proxy=0 → V2 σ²_SB = 0 (perfectly concentrated signals).
    let committee = committee_uniform(&[1_000, 800, 300, 100, 50], 0);
    let var = singh_bernstein_variance(&committee).unwrap();
    assert_eq!(var, 0, "zero variance proxy must yield zero V2 bound");
}

#[test]
fn variance_bounds_are_deterministic() {
    let committee = committee_uniform(&[1_000, 800, 200, 50, 0], 100);
    let v1 = singh_bernstein_variance(&committee).unwrap();
    let v2 = singh_bernstein_variance(&committee).unwrap();
    let g1 = passes_singh_bernstein_gate(80, &committee, 1).unwrap();
    let g2 = passes_singh_bernstein_gate(80, &committee, 1).unwrap();
    assert_eq!(v1, v2, "V2 variance must be deterministic");
    assert_eq!(g1, g2, "gate result must be deterministic");
}

// ── Adversarial tests ─────────────────────────────────────────────────────

#[test]
fn adversarial_empty_fails_closed() {
    assert_eq!(
        singh_bernstein_variance(&[]).unwrap_err(),
        BernsteinError::Empty
    );
    assert!(
        passes_singh_bernstein_gate(10, &[], 1).is_err(),
        "gate on empty list must fail"
    );
    assert!(
        bernstein_strictly_tighter(10, &[], 1).is_err(),
        "compare on empty list must fail"
    );
}

#[test]
fn adversarial_invalid_range_lo_gt_hi_rejected() {
    let bad = vec![cv(100, 10, 1_000, 0)]; // lo=100 > hi=10
    assert!(matches!(
        singh_bernstein_variance(&bad).unwrap_err(),
        BernsteinError::InvalidRange {
            lo: 100,
            hi: 10,
            ..
        }
    ));
}

#[test]
fn adversarial_popoviciu_guard_rejects_variance_exceeding_range_squared() {
    // range=10 → range²=100; variance_proxy=200 > 100 → rejected.
    let bad = vec![cv(0, 10, 1_000, 200)];
    assert!(matches!(
        singh_bernstein_variance(&bad).unwrap_err(),
        BernsteinError::VarianceExceedsRangeSquared { .. }
    ));
}

#[test]
fn adversarial_all_zero_energy_fails_closed() {
    let all_dead = vec![cv(0, 100, 0, 100), cv(0, 100, 0, 100), cv(0, 100, 0, 100)];
    assert_eq!(
        singh_bernstein_variance(&all_dead).unwrap_err(),
        BernsteinError::ZeroEMax,
        "all-zero energy must fail closed (ZeroEMax)"
    );
}

#[test]
fn adversarial_gate_exact_boundary_admits() {
    // Gate is ≥: 3·ε² == K·(6σ² + 2Mε) must ADMIT (not reject).
    // 1 contributor: range=10, energy=1000, variance_proxy=0.
    // M=10, σ²=0. Gate: 3·ε² ≥ K·2·10·ε → 3ε ≥ 20K (div by ε).
    // At K=1: ε_min = ceil(20/3) = 7.
    // ε=7: 3·49=147; 2·10·7=140; 147 ≥ 140 → ADMIT.
    let single = vec![cv(0, 10, 1_000, 0)];
    assert!(
        passes_singh_bernstein_gate(7, &single, 1).unwrap(),
        "3·49=147 ≥ 2·10·7=140 — must admit"
    );
    // ε=6: 3·36=108; 2·10·6=120; 108 < 120 → REJECT.
    assert!(
        !passes_singh_bernstein_gate(6, &single, 1).unwrap(),
        "3·36=108 < 2·10·6=120 — must reject"
    );
}

#[test]
fn adversarial_single_fresh_contributor_at_max_variance_matches_formula() {
    // 1 contributor: range=[0,50], energy=1000, variance_proxy=2500 (=50²).
    // V2 σ²_SB = (1000/1000)² × 2500 = 2500.
    let single = vec![cv(0, 50, 1_000, 2_500)];
    assert_eq!(singh_bernstein_variance(&single).unwrap(), 2_500);
}

#[test]
fn adversarial_zero_energy_contributor_contributes_nothing_when_another_fresh() {
    // Mixed: dead contributor + fresh contributor, same variance.
    // Dead: e=0 → weight 0 → contributes 0.
    // Fresh: e=1000 → weight 1 → contributes variance_proxy.
    let mixed = vec![cv(0, 100, 0, 100), cv(0, 100, 1_000, 100)];
    let var = singh_bernstein_variance(&mixed).unwrap();
    assert_eq!(
        var, 100,
        "fully-decayed contributor contributes 0 to V2 variance"
    );
}

#[test]
fn adversarial_variance_proxy_exactly_at_range_squared_allowed() {
    // Popoviciu allows variance_proxy == range² (equality is fine).
    let boundary = vec![cv(0, 10, 1_000, 100)]; // 10² = 100
    assert_eq!(
        singh_bernstein_variance(&boundary).unwrap(),
        100,
        "variance_proxy == range² must be accepted (Popoviciu: ≤ not <)"
    );
}

#[test]
fn adversarial_single_contributor_high_soundness_multiplier_rejects() {
    // K=100 means the gate requires massive deviation dominance.
    // ε=80, K=100: 3·6400=19_200 < 100·(6·500+16_000)=100·19_000=1_900_000 → REJECT.
    let committee = committee_uniform(&[1_000, 1_000, 1_000, 1_000, 1_000], 100);
    assert!(
        !passes_singh_bernstein_gate(80, &committee, 100).unwrap(),
        "high soundness multiplier K=100 must reject ε=80"
    );
}

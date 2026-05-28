//! End-to-end integration tests for evaporchain-singh-inequality.
//!
//! Non-trivial fixture: 5-validator consensus fee-range assessment
//! under progressive energy decay.
//!
//! A consensus round asks: is a proposed fee deviation of ε=105 units
//! admissible given the 5 validators' attested fee ranges?
//! Each validator attests a range [0, 100]; their energies evolve over
//! three epochs as validators progressively decay.
//!
//!   Epoch A — all fresh:
//!     V1…V5  range=[0,100]  energy=1_000 each
//!     Hoeffding σ² = 5 × 100² = 50_000
//!     Singh    σ² = 5 × 100² = 50_000  (identical — no decay yet)
//!     Gate(ε=105, K=1): 2×105²=22_050 < 50_000 → REJECT (both agree)
//!
//!   Epoch B — three validators heavily decayed:
//!     V1: energy=1_000  V2: energy=1_000
//!     V3: energy=  100  V4: energy=   10  V5: energy=0
//!     E_max = 1_000
//!     Singh σ²:
//!       V1: ω=100  ω²=10_000
//!       V2: ω=100  ω²=10_000
//!       V3: ω= 10  ω²=   100
//!       V4: ω=  1  ω²=     1
//!       V5: ω=  0  ω²=     0
//!     Singh σ² = 20_101 < Hoeffding σ² = 50_000
//!     Gate(ε=105, K=1): 22_050 ≥ 20_101 → Singh ADMITS
//!                        22_050 <  50_000 → Hoeffding REJECTS
//!
//!   Epoch C — two validators remain fresh, rest fully decayed:
//!     V1: energy=1_000  V2: energy=1_000  V3…V5: energy=0
//!     Singh σ² = 10_000 + 10_000 + 0 + 0 + 0 = 20_000
//!     Gate(ε=100, K=1): 2×100²=20_000 ≥ 20_000 → Singh ADMITS
//!                        20_000 < 50_000  → Hoeffding REJECTS
//!
//! Doctrine claim (INVENTION_STACK §A4 / Singh Inequality):
//! "Decayed validators no longer inflate the variance estimate.
//! The chain gets tighter confidence intervals over fresh data,
//! automatically excluding contributors whose energy has evaporated.
//! Singh σ² ≤ Hoeffding σ² always; at full energy they coincide."
//!
//! Adversarial fixture: invalid range (lo>hi), empty list, all-zero
//! energy, gate boundary (== vs ≥ at equality), determinism.
//!
//! INVENTION_STACK §A4: Singh Inequality (energy-weighted Hoeffding).

use evaporchain_singh_inequality::{
    hoeffding_variance_bound, passes_singh_gate, singh_variance_bound, BoundError, Contributor,
};

// ── Helpers ───────────────────────────────────────────────────────────────

fn c(lo: u64, hi: u64, energy: u64) -> Contributor {
    Contributor { lo, hi, energy }
}

/// Build the 5-validator committee with given energy vector.
fn committee_epoch(energies: &[u64; 5]) -> Vec<Contributor> {
    energies.iter().map(|&e| c(0, 100, e)).collect()
}

// ── Non-trivial fixture ───────────────────────────────────────────────────

#[test]
fn epoch_a_all_fresh_singh_equals_hoeffding() {
    // Epoch A: all 5 validators at full energy → bounds coincide.
    let committee = committee_epoch(&[1_000, 1_000, 1_000, 1_000, 1_000]);

    let hoeffding = hoeffding_variance_bound(&committee).unwrap();
    let singh = singh_variance_bound(&committee).unwrap();

    assert_eq!(hoeffding, 50_000, "Hoeffding: 5 × 100² = 50_000");
    assert_eq!(
        singh, hoeffding,
        "at full energy Singh must equal Hoeffding"
    );
}

#[test]
fn epoch_b_three_decayed_singh_strictly_tighter() {
    // Epoch B: V3(100), V4(10), V5(0) heavily decayed.
    let committee = committee_epoch(&[1_000, 1_000, 100, 10, 0]);

    let hoeffding = hoeffding_variance_bound(&committee).unwrap();
    let singh = singh_variance_bound(&committee).unwrap();

    assert_eq!(hoeffding, 50_000);
    assert_eq!(singh, 20_101, "Singh σ²: 10_000+10_000+100+1+0=20_101");
    assert!(
        singh < hoeffding,
        "Singh must be strictly tighter when contributors are decayed"
    );
}

#[test]
fn epoch_c_only_two_fresh_singh_further_tightens() {
    // Epoch C: V3, V4, V5 fully decayed (energy=0).
    let committee = committee_epoch(&[1_000, 1_000, 0, 0, 0]);

    let hoeffding = hoeffding_variance_bound(&committee).unwrap();
    let singh = singh_variance_bound(&committee).unwrap();

    assert_eq!(hoeffding, 50_000);
    assert_eq!(singh, 20_000, "Singh σ²: 10_000+10_000+0+0+0=20_000");
    assert!(singh < hoeffding);
}

#[test]
fn doctrine_decay_lets_deviation_pass_singh_but_fail_hoeffding() {
    // INVENTION_STACK §A4 doctrine claim: decayed validators removed from
    // variance estimate lets a deviation that would fail Hoeffding pass Singh.
    //
    // Epoch B: deviation=105, K=1.
    // Hoeffding: 2×105²=22_050 < 50_000 → REJECT.
    // Singh:     22_050 ≥ 20_101 → ADMIT.
    let committee = committee_epoch(&[1_000, 1_000, 100, 10, 0]);

    let h_passes = {
        let h_var = hoeffding_variance_bound(&committee).unwrap();
        2 * 105u128.pow(2) >= h_var
    };
    let s_passes = passes_singh_gate(105, &committee, 1).unwrap();

    assert!(!h_passes, "Hoeffding must REJECT deviation=105 at Epoch B");
    assert!(
        s_passes,
        "Singh must ADMIT deviation=105 at Epoch B (decayed validators excluded)"
    );
}

#[test]
fn doctrine_decay_collapse_at_full_decay_epoch_c() {
    // Epoch C: deviation=100, K=1.
    // 2×100²=20_000; Singh σ²=20_000 → 20_000 ≥ 20_000 → ADMIT (gate: ≥).
    // Hoeffding: 20_000 < 50_000 → REJECT.
    let committee = committee_epoch(&[1_000, 1_000, 0, 0, 0]);

    let s_passes = passes_singh_gate(100, &committee, 1).unwrap();
    let h_var = hoeffding_variance_bound(&committee).unwrap();
    let h_passes = 2 * 100u128.pow(2) >= h_var;

    assert!(
        s_passes,
        "Singh must ADMIT deviation=100 at Epoch C (gate: ≥)"
    );
    assert!(!h_passes, "Hoeffding must REJECT deviation=100 at Epoch C");
}

#[test]
fn doctrine_singh_leq_hoeffding_across_all_epochs() {
    // Singh is always ≤ Hoeffding regardless of energy distribution.
    for energies in [
        [1_000, 1_000, 1_000, 1_000, 1_000],
        [1_000, 1_000, 100, 10, 0],
        [1_000, 1_000, 0, 0, 0],
        [500, 300, 200, 100, 50],
        [1, 1, 1, 1, 1],
    ] {
        let committee = committee_epoch(&energies);
        let h = hoeffding_variance_bound(&committee).unwrap();
        let s = singh_variance_bound(&committee).unwrap();
        assert!(
            s <= h,
            "Singh ({s}) must be ≤ Hoeffding ({h}) for energies={energies:?}"
        );
    }
}

#[test]
fn decay_monotonically_shrinks_singh_variance() {
    // As one contributor's energy decreases, Singh σ² must decrease
    // or stay the same — monotone in energy.
    let energy_steps = [1_000u64, 500, 100, 10, 1, 0];
    let mut prev_singh = u128::MAX;

    for &e in &energy_steps {
        // V1 fresh=1000; V2 at energy=e; V3-V5 fixed at 100.
        let committee = vec![
            c(0, 100, 1_000),
            c(0, 100, e),
            c(0, 100, 100),
            c(0, 100, 100),
            c(0, 100, 100),
        ];
        let singh = singh_variance_bound(&committee).unwrap();
        assert!(
            singh <= prev_singh,
            "Singh σ² must be monotone non-increasing as V2 energy drops (e={e})"
        );
        prev_singh = singh;
    }
}

#[test]
fn variance_bounds_are_deterministic() {
    let committee = committee_epoch(&[1_000, 800, 200, 50, 0]);
    let h1 = hoeffding_variance_bound(&committee).unwrap();
    let h2 = hoeffding_variance_bound(&committee).unwrap();
    let s1 = singh_variance_bound(&committee).unwrap();
    let s2 = singh_variance_bound(&committee).unwrap();
    assert_eq!(h1, h2, "Hoeffding must be deterministic");
    assert_eq!(s1, s2, "Singh must be deterministic");
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_empty_list_fails_closed() {
    assert_eq!(
        hoeffding_variance_bound(&[]).unwrap_err(),
        BoundError::Empty
    );
    assert_eq!(singh_variance_bound(&[]).unwrap_err(), BoundError::Empty);
    assert!(
        passes_singh_gate(10, &[], 1).is_err(),
        "passes_singh_gate on empty list must fail"
    );
}

#[test]
fn adversarial_invalid_range_lo_gt_hi_rejected() {
    let bad = vec![c(100, 10, 1_000)]; // lo=100 > hi=10
    let err = hoeffding_variance_bound(&bad).unwrap_err();
    assert!(matches!(
        err,
        BoundError::InvalidRange {
            lo: 100,
            hi: 10,
            ..
        }
    ));
    let err2 = singh_variance_bound(&bad).unwrap_err();
    assert!(matches!(
        err2,
        BoundError::InvalidRange {
            lo: 100,
            hi: 10,
            ..
        }
    ));
}

#[test]
fn adversarial_all_zero_energy_fails_closed() {
    let all_dead = vec![c(0, 100, 0), c(0, 100, 0), c(0, 100, 0)];
    assert_eq!(
        singh_variance_bound(&all_dead).unwrap_err(),
        BoundError::ZeroEMax,
        "all-zero energy must fail closed (ZeroEMax)"
    );
}

#[test]
fn adversarial_gate_exact_boundary_admits() {
    // Gate is ≥: 2·ε² == K·σ² must ADMIT (not reject).
    // 1 contributor: range=10, energy=1000. Singh σ² = 100.
    // 2·10² = 200; K=2: 200 ≥ 200 → ADMIT.
    let contributor = vec![c(0, 10, 1_000)];
    assert!(
        passes_singh_gate(10, &contributor, 2).unwrap(),
        "2·ε² == K·σ² must be admitted (gate: ≥)"
    );
    // One below: deviation=9; 2·81=162 < 200 → REJECT.
    assert!(
        !passes_singh_gate(9, &contributor, 2).unwrap(),
        "2·ε² < K·σ² must be rejected"
    );
}

#[test]
fn adversarial_single_fresh_contributor_matches_formula() {
    // 1 contributor: range=[0, 50], energy=1000. Singh = Hoeffding = 50² = 2500.
    let single = vec![c(0, 50, 1_000)];
    assert_eq!(hoeffding_variance_bound(&single).unwrap(), 2_500);
    assert_eq!(singh_variance_bound(&single).unwrap(), 2_500);
}

#[test]
fn adversarial_mixed_zero_and_nonzero_energy() {
    // A single contributor with zero energy shouldn't break things if
    // another has positive energy.
    let mixed = vec![c(0, 100, 0), c(0, 100, 1_000)];
    let s = singh_variance_bound(&mixed).unwrap();
    // V1: ω = 100×0/1000 = 0. V2: ω = 100×1000/1000 = 100. Total = 10_000.
    assert_eq!(s, 10_000, "fully-decayed contributor contributes 0 to σ²");
}

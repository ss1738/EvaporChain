//! End-to-end integration tests for evaporchain-singh-attractor-v2.
//!
//! Non-trivial fixture: anti-grinding five-epoch fee-controller session.
//!
//! A validator cannot predict the fallback attractor before the Bell-
//! certified seed is published. Five epochs exercise the three operating
//! regimes plus two no-basin transitions where the Bell seed governs.
//!
//!   LOW  regime: centre=50_000,  basin=20_000  → [30_000,   70_000]
//!   MID  regime: centre=500_000, basin=100_000 → [400_000,  600_000]
//!   HIGH regime: centre=5_000_000, basin=1_000_000 → [4_000_000, 6_000_000]
//!
//!   Epoch 1  state=45_000     → LOW  (in basin, seed-invariant)
//!   Epoch 2  state=550_000    → MID  (in basin, seed-invariant)
//!   Epoch 3  state=200_000    → gap LOW↔MID   → fallback (seed-dependent)
//!   Epoch 4  state=1_000_000  → gap MID↔HIGH  → fallback (seed-dependent)
//!   Epoch 5  state=7_000_000  → above HIGH    → fallback (seed-dependent)
//!
//! In-basin epochs are deterministic (seed unused).
//! Out-of-basin epochs draw from the Bell-certified seed — unpredictable
//! until published, closing the V1 grinding gap.
//!
//! Doctrine claim (INVENTION_STACK §4.2): "Singh Attractor V2 anchors
//! fallback selection to a Bell-Certified Beacon seed. In-basin selection
//! is unchanged from V1. Out-of-basin, a malicious validator cannot
//! predict which attractor the chain selects before the certificate is
//! published. Anti-grinding follows from Bell-Beacon's own
//! anti-grinding properties."
//!
//! Adversarial fixture: empty set, degenerate attractor, zero-basin
//! (non-degenerate), Lyapunov convergence verification.
//!
//! INVENTION_STACK §4.2: Singh Attractor Consensus V2.

use evaporchain_singh_attractor_v2::{draw_attractor, AttractorV2, DrawError};

// ── Regime definitions ────────────────────────────────────────────────────

const LOW_CENTER:  u64 =   50_000;
const LOW_BASIN:   u64 =   20_000; // [30_000, 70_000]
const LOW_DRIFT:   u64 =    5_000;

const MID_CENTER:  u64 =  500_000;
const MID_BASIN:   u64 =  100_000; // [400_000, 600_000]
const MID_DRIFT:   u64 =   10_000;

const HIGH_CENTER: u64 = 5_000_000;
const HIGH_BASIN:  u64 = 1_000_000; // [4_000_000, 6_000_000]
const HIGH_DRIFT:  u64 =    50_000;

fn three_regime_v2() -> [AttractorV2; 3] {
    [
        AttractorV2::new(LOW_CENTER,  LOW_BASIN,  LOW_DRIFT),
        AttractorV2::new(MID_CENTER,  MID_BASIN,  MID_DRIFT),
        AttractorV2::new(HIGH_CENTER, HIGH_BASIN, HIGH_DRIFT),
    ]
}

fn seed(b: u8) -> [u8; 32] { [b; 32] }

// ── Non-trivial fixture: five-epoch anti-grinding session ─────────────────

#[test]
fn five_epoch_session_in_basin_and_fallback() {
    let attractors = three_regime_v2();

    // Epoch 1: inside LOW basin → deterministic, no seed needed.
    let e1 = draw_attractor(45_000, &attractors, &seed(0xAA)).unwrap();
    assert_eq!(e1.selected_center, LOW_CENTER,  "E1: LOW-traffic state must pick LOW regime");
    assert!(!e1.used_fallback, "E1: in-basin must not use fallback");
    assert!(e1.drift > 0, "E1: state below center, drift must be positive");
    assert!(e1.drift as u64 <= LOW_DRIFT);

    // Epoch 2: inside MID basin → deterministic.
    let e2 = draw_attractor(550_000, &attractors, &seed(0xBB)).unwrap();
    assert_eq!(e2.selected_center, MID_CENTER, "E2: active-DEX state must pick MID regime");
    assert!(!e2.used_fallback);
    assert!(e2.drift < 0, "E2: state above center, drift must be negative");
    assert!(e2.drift.unsigned_abs() <= MID_DRIFT as u128);

    // Epoch 3: no basin (gap LOW↔MID) → fallback active.
    let e3 = draw_attractor(200_000, &attractors, &seed(0x01)).unwrap();
    assert!(e3.used_fallback, "E3: gap state must use Bell-seed fallback");
    // Drift bounded by the selected attractor's drift_rate.
    let chosen3 = &attractors[e3.selected_index];
    assert!(e3.drift.unsigned_abs() <= chosen3.drift_rate as u128,
        "E3: drift must not exceed selected attractor's drift_rate");

    // Epoch 4: no basin (gap MID↔HIGH) → fallback active.
    let e4 = draw_attractor(1_000_000, &attractors, &seed(0x02)).unwrap();
    assert!(e4.used_fallback, "E4: gap state must use Bell-seed fallback");
    let chosen4 = &attractors[e4.selected_index];
    assert!(e4.drift.unsigned_abs() <= chosen4.drift_rate as u128);

    // Epoch 5: above HIGH ceiling → fallback active.
    let e5 = draw_attractor(7_000_000, &attractors, &seed(0x03)).unwrap();
    assert!(e5.used_fallback, "E5: post-HIGH state must use Bell-seed fallback");
    let chosen5 = &attractors[e5.selected_index];
    assert!(e5.drift.unsigned_abs() <= chosen5.drift_rate as u128);
}

#[test]
fn in_basin_selection_is_seed_invariant() {
    // In-basin selection must produce the same result regardless of the
    // Bell-certified seed — the seed is only consumed on the fallback path.
    let attractors = three_regime_v2();
    let state = 45_000; // inside LOW basin

    let r1 = draw_attractor(state, &attractors, &seed(0x00)).unwrap();
    let r2 = draw_attractor(state, &attractors, &seed(0xFF)).unwrap();
    let r3 = draw_attractor(state, &attractors, &seed(0x7F)).unwrap();

    assert_eq!(r1.selected_center, r2.selected_center);
    assert_eq!(r1.selected_center, r3.selected_center);
    assert_eq!(r1.drift,           r2.drift);
    assert_eq!(r1.drift,           r3.drift);
    assert!(!r1.used_fallback);
}

#[test]
fn fallback_path_is_validator_deterministic() {
    // Same state + same seed → identical result every time.
    // This is the "validator BFT property": all honest validators
    // who see the same Bell certificate agree on the attractor.
    let attractors = three_regime_v2();
    let state = 200_000; // no basin
    let fixed_seed = seed(0x42);

    let r1 = draw_attractor(state, &attractors, &fixed_seed).unwrap();
    let r2 = draw_attractor(state, &attractors, &fixed_seed).unwrap();
    let r3 = draw_attractor(state, &attractors, &fixed_seed).unwrap();

    assert_eq!(r1, r2, "same seed must produce identical DrawResult");
    assert_eq!(r1, r3);
}

#[test]
fn fallback_path_is_seed_dependent() {
    // Different seeds must sometimes yield different attractors —
    // this is the anti-grinding property: an adversary who submits
    // a block before the Bell seed is published cannot predict the
    // chain's attractor selection.
    let attractors = three_regime_v2();
    let state = 200_000; // gap LOW↔MID

    let mut seen_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for b in 0u8..=60 {
        let r = draw_attractor(state, &attractors, &seed(b)).unwrap();
        seen_indices.insert(r.selected_index);
        if seen_indices.len() >= 2 {
            break;
        }
    }
    assert!(
        seen_indices.len() >= 2,
        "fallback path must produce at least 2 distinct selections across 61 seeds"
    );
}

#[test]
fn fallback_closer_attractor_wins_more_often() {
    // Inverse-distance weighting means the closer centre is favoured.
    // state=75_000 is just outside LOW basin (top=70_000) and far from
    // MID (400_000) and HIGH (5_000_000).
    // Distance to LOW=50_000: 25_000
    // Distance to MID=500_000: 425_000
    // Distance to HIGH=5_000_000: 4_925_000
    // LOW must win ≥80% of the time across varied seeds.
    let attractors = three_regime_v2();
    let state = 75_000u64; // just outside LOW basin

    let mut count_low = 0u32;
    let total = 100u32;
    for b in 0u8..100 {
        let r = draw_attractor(state, &attractors, &seed(b)).unwrap();
        assert!(r.used_fallback);
        if r.selected_center == LOW_CENTER {
            count_low += 1;
        }
    }
    assert!(
        count_low * 100 >= total * 80,
        "LOW attractor (closest) should win ≥80% of draws; got {count_low}/{total}"
    );
}

#[test]
fn drift_direction_and_magnitude_correct() {
    let attractors = [AttractorV2::new(1000, 100, 50)];

    // State below center → positive drift (nudge up).
    let below = draw_attractor(900, &attractors, &seed(0)).unwrap();
    assert_eq!(below.drift, 50, "state below center: drift must be +drift_rate");

    // State above center → negative drift (nudge down).
    let above = draw_attractor(1100, &attractors, &seed(0)).unwrap();
    assert_eq!(above.drift, -50, "state above center: drift must be -drift_rate");

    // State at center → zero drift.
    let at_center = draw_attractor(1000, &attractors, &seed(0)).unwrap();
    assert_eq!(at_center.drift, 0, "state at center: drift must be zero");

    // Close to center: drift clamped to actual distance.
    let close = draw_attractor(995, &attractors, &seed(0)).unwrap();
    assert_eq!(close.drift, 5, "close state: drift clamped to distance (5), not rate (50)");
}

#[test]
fn doctrine_lyapunov_convergence_to_center() {
    // INVENTION_STACK §4.2 doctrine: Lyapunov stability — applying
    // drift strictly decreases |state − center| each step until arrival.
    // Verify convergence from both sides of the center.
    let attractors = [AttractorV2::new(1000, 200, 37)];

    for &start in &[600u64, 1400u64] {
        let mut state = start;
        let mut prev_dist = state.abs_diff(1000);
        for _ in 0..200 {
            let r = draw_attractor(state, &attractors, &seed(0)).unwrap();
            let new_state = (state as i128 + r.drift) as u64;
            let new_dist = new_state.abs_diff(1000);
            assert!(
                new_dist <= prev_dist,
                "Lyapunov: distance must monotonically decrease (state={state}, drift={}, new_dist={new_dist}, prev_dist={prev_dist})",
                r.drift
            );
            state = new_state;
            prev_dist = new_dist;
            if state == 1000 { break; }
        }
        assert_eq!(state, 1000, "must converge to center starting from {start}");
    }
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_empty_attractor_set_returns_error() {
    let err = draw_attractor(500_000, &[], &seed(0)).unwrap_err();
    assert_eq!(err, DrawError::Empty,
        "empty attractor list must return DrawError::Empty");
}

#[test]
fn adversarial_degenerate_attractor_basin_zero_drift_zero_rejected() {
    // basin_radius=0 AND drift_rate=0 means the attractor can never
    // be matched AND can never pull state toward it — fully inert.
    let bad = [AttractorV2::new(1000, 0, 0)];
    let err = draw_attractor(500, &bad, &seed(0)).unwrap_err();
    assert!(
        matches!(err, DrawError::DegenerateAttractor { idx: 0 }),
        "degenerate attractor must be rejected with DegenerateAttractor{{idx:0}}"
    );
}

#[test]
fn adversarial_zero_basin_with_nonzero_drift_is_not_degenerate() {
    // basin_radius=0 is a point basin (only exact center matches),
    // but if drift_rate > 0 the attractor can still pull state.
    // This must NOT be rejected as degenerate.
    let point = [AttractorV2::new(1000, 0, 10)];
    // State=999 is not in the point basin; fallback used.
    let r = draw_attractor(999, &point, &seed(0));
    assert!(r.is_ok(), "zero-basin with drift_rate>0 must not be rejected");
}

#[test]
fn adversarial_mixed_set_one_degenerate_rejected_early() {
    // Even if the first attractor is valid, a degenerate one later
    // in the list must still be rejected.
    let mixed = [
        AttractorV2::new(100,   50,  5), // valid
        AttractorV2::new(1000,   0,  0), // degenerate (idx=1)
    ];
    let err = draw_attractor(150, &mixed, &seed(0)).unwrap_err();
    assert!(
        matches!(err, DrawError::DegenerateAttractor { idx: 1 }),
        "degenerate attractor at idx=1 must be caught even when valid attractor precedes it"
    );
}

#[test]
fn adversarial_fallback_path_drift_bounded_for_all_seeds() {
    // Regardless of the Bell seed, fallback drift must never exceed
    // the selected attractor's drift_rate.
    let attractors = three_regime_v2();
    let state = 200_000; // gap LOW↔MID

    for b in 0u8..=255 {
        let r = draw_attractor(state, &attractors, &seed(b)).unwrap();
        let chosen = &attractors[r.selected_index];
        assert!(
            r.drift.unsigned_abs() <= chosen.drift_rate as u128,
            "seed=0x{b:02X}: drift {} exceeds drift_rate {} for center {}",
            r.drift, chosen.drift_rate, chosen.center
        );
    }
}

//! End-to-end integration tests for evaporchain-singh-attractor.
//!
//! Non-trivial fixture: three-regime chain fee session.
//!
//! A chain runs through a simulated 24-hour trading day with three
//! distinct operating modes. The consensus rule selects the attractor
//! whose basin contains the current state — different epochs fall into
//! different regimes.
//!
//!   QUIET  regime: low traffic (center=100_000, radius=40_000)
//!     basin [60_000, 140_000]
//!   ACTIVE regime: normal DEX activity (center=500_000, radius=150_000)
//!     basin [350_000, 650_000]
//!   SURGE  regime: MEV peak / heavy load (center=2_000_000, radius=500_000)
//!     basin [1_500_000, 2_500_000]
//!
//! Six epoch snapshots across the session:
//!
//!   E1  energy=  80_000  → QUIET  (in basin)
//!   E2  energy= 400_000  → ACTIVE (in basin)
//!   E3  energy= 600_000  → ACTIVE (barely inside, 650k ceiling)
//!   E4  energy=1_800_000 → SURGE  (in basin)
//!   E5  energy=  750_000 → no basin → fallback nearest ACTIVE  (dist 250k)
//!   E6  energy=3_000_000 → no basin → fallback nearest SURGE   (dist 1M)
//!
//! Basin-based selection handles 4 of 6 epochs; deterministic
//! nearest-centre fallback handles the remaining 2.
//!
//! Doctrine claim (INVENTION_STACK §4.2): "Singh Attractor Consensus
//! extends Singh-Lyapunov's single-equilibrium fee controller to
//! multiple equilibria. A chain with distinct steady-state regimes
//! (quiet vs DEX-peak) cannot be governed by a single fixed target
//! — `select_attractor` picks the right one automatically."
//!
//! Adversarial fixture: empty attractor set, overlapping basins,
//! single overly-wide basin, exact boundary membership.
//!
//! INVENTION_STACK §4.2: Singh Attractor Consensus.

use evaporchain_singh_attractor::{select_attractor, Attractor};

// ── Regime definitions ────────────────────────────────────────────────────

const QUIET_CENTER:  u64 = 100_000;
const QUIET_RADIUS:  u64 =  40_000; // basin [60_000, 140_000]

const ACTIVE_CENTER: u64 = 500_000;
const ACTIVE_RADIUS: u64 = 150_000; // basin [350_000, 650_000]

const SURGE_CENTER:  u64 = 2_000_000;
const SURGE_RADIUS:  u64 =   500_000; // basin [1_500_000, 2_500_000]

fn three_regime_attractors() -> [Attractor; 3] {
    [
        Attractor::new(QUIET_CENTER,  QUIET_RADIUS),
        Attractor::new(ACTIVE_CENTER, ACTIVE_RADIUS),
        Attractor::new(SURGE_CENTER,  SURGE_RADIUS),
    ]
}

// ── Non-trivial fixture: three-regime chain fee session ───────────────────

#[test]
fn chain_session_regime_selection_across_six_epochs() {
    // The fee controller picks the right regime at each epoch snapshot.
    let attractors = three_regime_attractors();

    // E1: quiet night — well inside QUIET basin.
    let e1 = select_attractor(80_000, &attractors).unwrap();
    assert_eq!(e1.center, QUIET_CENTER, "E1: low-traffic state must select QUIET regime");
    assert!(e1.contains(80_000));

    // E2: morning activity — well inside ACTIVE basin.
    let e2 = select_attractor(400_000, &attractors).unwrap();
    assert_eq!(e2.center, ACTIVE_CENTER, "E2: normal-DEX state must select ACTIVE regime");
    assert!(e2.contains(400_000));

    // E3: DEX near ceiling — barely inside ACTIVE basin (edge=650_000).
    let e3 = select_attractor(600_000, &attractors).unwrap();
    assert_eq!(e3.center, ACTIVE_CENTER, "E3: near-ceiling state must still select ACTIVE regime");
    assert!(e3.contains(600_000));

    // E4: MEV peak — inside SURGE basin.
    let e4 = select_attractor(1_800_000, &attractors).unwrap();
    assert_eq!(e4.center, SURGE_CENTER, "E4: MEV-peak state must select SURGE regime");
    assert!(e4.contains(1_800_000));

    // E5: between ACTIVE top (650k) and SURGE bottom (1.5M) — no basin.
    //   dist to QUIET=100k:  |750k-100k| = 650_000
    //   dist to ACTIVE=500k: |750k-500k| = 250_000  ← nearest
    //   dist to SURGE=2M:    |750k-2M|   = 1_250_000
    let e5 = select_attractor(750_000, &attractors).unwrap();
    assert_eq!(e5.center, ACTIVE_CENTER,
        "E5: no-basin gap state must fall back to nearest center (ACTIVE at 500k)");
    assert!(!e5.contains(750_000), "fallback attractor does NOT contain the state");

    // E6: beyond SURGE ceiling (2.5M) — no basin.
    //   dist to QUIET=100k:  2_900_000
    //   dist to ACTIVE=500k: 2_500_000
    //   dist to SURGE=2M:    1_000_000  ← nearest
    let e6 = select_attractor(3_000_000, &attractors).unwrap();
    assert_eq!(e6.center, SURGE_CENTER,
        "E6: post-surge state must fall back to nearest center (SURGE at 2M)");
    assert!(!e6.contains(3_000_000));
}

#[test]
fn quiet_regime_basin_boundaries_exact() {
    // QUIET basin is [60_000, 140_000] inclusive.
    let attractors = three_regime_attractors();

    let lo = QUIET_CENTER - QUIET_RADIUS; // 60_000
    let hi = QUIET_CENTER + QUIET_RADIUS; // 140_000

    // Exact lower boundary → in QUIET basin.
    let a_lo = select_attractor(lo, &attractors).unwrap();
    assert_eq!(a_lo.center, QUIET_CENTER, "lower boundary {lo} must be QUIET");

    // Exact upper boundary → in QUIET basin.
    let a_hi = select_attractor(hi, &attractors).unwrap();
    assert_eq!(a_hi.center, QUIET_CENTER, "upper boundary {hi} must be QUIET");

    // One below lower boundary → not in QUIET basin.
    assert!(
        !Attractor::new(QUIET_CENTER, QUIET_RADIUS).contains(lo - 1),
        "one below lower boundary must be outside QUIET basin"
    );

    // One above upper boundary → not in QUIET basin.
    assert!(
        !Attractor::new(QUIET_CENTER, QUIET_RADIUS).contains(hi + 1),
        "one above upper boundary must be outside QUIET basin"
    );
}

#[test]
fn active_regime_basin_boundaries_exact() {
    // ACTIVE basin is [350_000, 650_000] inclusive.
    let lo = ACTIVE_CENTER - ACTIVE_RADIUS; // 350_000
    let hi = ACTIVE_CENTER + ACTIVE_RADIUS; // 650_000

    let a = Attractor::new(ACTIVE_CENTER, ACTIVE_RADIUS);
    assert!(a.contains(lo),     "ACTIVE lo={lo}: must be inside");
    assert!(a.contains(hi),     "ACTIVE hi={hi}: must be inside");
    assert!(!a.contains(lo - 1), "ACTIVE lo-1: must be outside");
    assert!(!a.contains(hi + 1), "ACTIVE hi+1: must be outside");
}

#[test]
fn surge_regime_basin_boundaries_exact() {
    // SURGE basin is [1_500_000, 2_500_000] inclusive.
    let lo = SURGE_CENTER - SURGE_RADIUS; // 1_500_000
    let hi = SURGE_CENTER + SURGE_RADIUS; // 2_500_000

    let a = Attractor::new(SURGE_CENTER, SURGE_RADIUS);
    assert!(a.contains(lo),     "SURGE lo={lo}: must be inside");
    assert!(a.contains(hi),     "SURGE hi={hi}: must be inside");
    assert!(!a.contains(lo - 1), "SURGE lo-1: must be outside");
    assert!(!a.contains(hi + 1), "SURGE hi+1: must be outside");
}

#[test]
fn four_of_six_epochs_have_basin_membership() {
    // Verifies the session fixture claim: 4 in-basin, 2 fallback.
    let attractors = three_regime_attractors();
    let epoch_states: &[u64] = &[80_000, 400_000, 600_000, 1_800_000, 750_000, 3_000_000];

    let in_basin_count = epoch_states.iter().filter(|&&s| {
        attractors.iter().any(|a| a.contains(s))
    }).count();

    assert_eq!(in_basin_count, 4,
        "exactly 4 of 6 epoch snapshots must have a basin owner");
}

#[test]
fn doctrine_multi_equilibrium_outperforms_single_fixed_target() {
    // INVENTION_STACK §4.2 doctrine claim:
    // "A chain with distinct regimes cannot be governed by a single target."
    //
    // Single-target comparison: use the midpoint of QUIET and SURGE as a
    // hypothetical unified target — 1_050_000. Under a single-equilibrium
    // controller, ALL epoch states drift toward 1_050_000.
    //
    // Under Singh Attractor, each state drifts toward ITS OWN regime center.
    //
    // Measure: |state − selected_center| summed across all 6 epochs.
    // Singh Attractor total drift distance must be less than single-target.
    let attractors = three_regime_attractors();
    let epoch_states: &[u64] = &[80_000, 400_000, 600_000, 1_800_000, 750_000, 3_000_000];
    let single_target: u64 = 1_050_000; // midpoint of QUIET and SURGE centers

    let attractor_total_dist: u64 = epoch_states.iter().map(|&s| {
        let a = select_attractor(s, &attractors).unwrap();
        a.center.abs_diff(s)
    }).sum();

    let single_total_dist: u64 = epoch_states.iter().map(|&s| {
        single_target.abs_diff(s)
    }).sum();

    assert!(
        attractor_total_dist < single_total_dist,
        "multi-attractor total drift {attractor_total_dist} must beat single-target drift {single_total_dist}"
    );
}

#[test]
fn first_match_wins_for_in_basin_selection() {
    // When two attractors overlap, the FIRST one in the slice wins.
    // This is deterministic: ordering in the consensus config determines priority.
    let wide  = Attractor::new(500, 400); // basin [100, 900]
    let narrow = Attractor::new(500, 50);  // basin [450, 550]

    let first_wide   = [wide, narrow];
    let first_narrow = [narrow, wide];

    // State 500 is in both basins.
    let bound_fw = first_wide;
    let bound_fn = first_narrow;
    let a1 = select_attractor(500, &bound_fw).unwrap();
    let a2 = select_attractor(500, &bound_fn).unwrap();

    assert_eq!(a1.basin_radius, 400, "first (wide) attractor must win when listed first");
    assert_eq!(a2.basin_radius, 50,  "narrow attractor must win when listed first");
}

#[test]
fn fallback_resolves_tie_by_first_in_slice_order() {
    // Two attractors equidistant from state — tie broken by first-in-slice.
    // State=500; attractor A=0 (dist=500), attractor B=1000 (dist=500).
    let a = Attractor::new(0, 1);
    let b = Attractor::new(1000, 1);

    // Both have same distance (500). `min_by_key` returns first on equal keys.
    let ab = [a, b];
    let ba = [b, a];
    let picked_ab = select_attractor(500, &ab).unwrap();
    let picked_ba = select_attractor(500, &ba).unwrap();

    assert_eq!(picked_ab.center, 0,
        "equidistant tie: first in slice (center=0) must win");
    assert_eq!(picked_ba.center, 1000,
        "equidistant tie: first in slice (center=1000) must win when reversed");
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_empty_attractor_set_returns_none() {
    // An empty consensus config must not panic. Returning None lets
    // the caller fall back to a safe default (e.g. do-nothing step).
    assert!(
        select_attractor(1_000_000, &[]).is_none(),
        "empty attractor slice must return None — no regime to apply"
    );
}

#[test]
fn adversarial_single_overly_wide_basin_always_matches() {
    // A deployment with one universal attractor (radius = u64::MAX / 2)
    // must always return that attractor. Guards against the zero-attractor
    // pathology while still supporting single-regime chains.
    let universal = Attractor::new(1_000_000, u64::MAX / 2);
    for &state in &[0u64, 1, 999_999, 1_000_000, u64::MAX / 2] {
        let uni_slice = [universal];
        let a = select_attractor(state, &uni_slice).unwrap();
        assert_eq!(a.center, 1_000_000,
            "universal basin must match every state (state={state})");
        assert!(a.contains(state));
    }
}

#[test]
fn adversarial_zero_radius_is_single_point_basin() {
    // A degenerate attractor with radius=0 is a point basin.
    // Only the center itself is a member.
    let point = Attractor::new(42, 0);
    assert!(point.contains(42),  "center itself must be in a zero-radius basin");
    assert!(!point.contains(41), "center-1 must be outside a zero-radius basin");
    assert!(!point.contains(43), "center+1 must be outside a zero-radius basin");
}

#[test]
fn adversarial_state_near_center_zero_saturates_correctly() {
    // Attractor at center=10, radius=50 — lower bound saturates to 0.
    // State=0 must be inside.
    let near_zero = Attractor::new(10, 50); // lo = 10.saturating_sub(50) = 0
    assert!(near_zero.contains(0),  "state=0 with lo-saturated basin must be inside");
    assert!(near_zero.contains(10), "center must be inside");
    assert!(near_zero.contains(60), "hi=60 must be inside");
    assert!(!near_zero.contains(61), "hi+1=61 must be outside");
}

//! §Modular-Form Beacon (§A1.4) e2e
//!
//! Scenario: "EvaporChain epoch randomness" — the beacon emits an
//! (E_4, E_6, Δ) triple per epoch. Block producers RAHUL and SUNITA
//! verify the modular identity holds at τ=0 and confirm no two
//! consecutive epochs produce the same triple (no aliasing). OSCAR
//! attempts to forge a beacon by falsifying one component; verify
//! catches him.
//!
//! The suite proves: τ=0 identity is exact; determinism holds;
//! distinct τs produce distinct triples; tolerance guard works.

use evaporchain_modular_beacon::{
    compute_beacon, evaluate_delta, evaluate_e4, evaluate_e6,
    verify_modular_identity, Beacon, BeaconError,
};

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn tau_zero_is_canonical_anchor() {
    // E_4(0)=1, E_6(0)=1, Δ(0)=0 — the substrate zero point.
    let b = compute_beacon(0);
    assert_eq!(b.tau, 0);
    assert_eq!(b.e4, 1,    "E_4(0) must be 1");
    assert_eq!(b.e6, 1,    "E_6(0) must be 1");
    assert_eq!(b.delta, 0, "Δ(0) must be 0");
}

#[test]
fn modular_identity_exact_at_tau_zero() {
    // E_4³ − E_6² = 1 − 1 = 0 = 1728·0 — exact at q=0.
    let b = compute_beacon(0);
    verify_modular_identity(&b, 0).expect("identity must hold exactly at τ=0");
}

#[test]
fn compute_beacon_is_deterministic() {
    // Two block producers independently compute the same τ=1 beacon.
    let rahul  = compute_beacon(1);
    let sunita = compute_beacon(1);
    assert_eq!(rahul, sunita,
        "beacon must be deterministic — validators must agree");
}

#[test]
fn consecutive_epochs_produce_distinct_beacons() {
    // No two consecutive τs alias — beacon has per-epoch uniqueness.
    let beacons: Vec<Beacon> = (0u64..8).map(compute_beacon).collect();
    for i in 0..beacons.len() {
        for j in (i+1)..beacons.len() {
            assert_ne!(beacons[i], beacons[j],
                "τ={i} and τ={j} must produce distinct beacons");
        }
    }
}

#[test]
fn tau_one_e4_equals_sum_of_e4_coeffs() {
    // At q=1 every term q^k = 1 so E_4(1) = Σ E4_COEFFS.
    let expected: i128 = 1 + 240 + 2160 + 6720 + 17520 + 30240 + 60480 + 82560;
    assert_eq!(evaluate_e4(1), expected,
        "E_4(1) must equal sum of coefficient table");
}

#[test]
fn tau_one_delta_matches_known_value() {
    // δ(1) = 65275 per the unit test; e2e confirms the evaluate path.
    assert_eq!(evaluate_delta(1), 65275,
        "Δ(1) must equal the known q-expansion value");
}

#[test]
fn e4_monotone_on_small_tau() {
    // E_4 has all-positive q-expansion coefficients so it grows monotonically.
    // E_6 does NOT — its first non-constant coefficient is −504, so E_6(1) < E_6(0).
    for tau in 0u64..4 {
        assert!(evaluate_e4(tau + 1) > evaluate_e4(tau),
            "E_4 must grow from τ={tau} to τ={}", tau + 1);
    }
    // E_6 drops from its τ=0 value of 1 once q > 0.
    assert!(evaluate_e6(1) < evaluate_e6(0),
        "E_6 must decrease at τ=1 (leading −504q coefficient)");
}

#[test]
fn forged_beacon_component_caught_by_identity_check() {
    // OSCAR flips the delta field — the identity check catches the forgery.
    let mut forged = compute_beacon(0);
    forged.delta += 1; // tamper
    assert!(
        verify_modular_identity(&forged, 0).is_err(),
        "tampered delta must fail identity check"
    );
}

#[test]
fn forged_e4_caught_by_identity_check() {
    let mut forged = compute_beacon(0);
    forged.e4 += 1;
    assert!(verify_modular_identity(&forged, 0).is_err(),
        "tampered e4 must fail identity check");
}

#[test]
fn identity_error_carries_diagnostic_fields() {
    // BeaconError::IdentityFailed must surface residual and tolerance.
    let mut bad = compute_beacon(0);
    bad.delta = 1;
    let err = verify_modular_identity(&bad, 0).unwrap_err();
    assert!(matches!(err, BeaconError::IdentityFailed { residual, .. } if residual > 0),
        "error must carry non-zero residual: {:?}", err);
}

#[test]
fn tolerance_widens_acceptance_window() {
    // tau=2 fails at tolerance=0 but must pass at a large enough tolerance.
    let b = compute_beacon(2);
    assert!(verify_modular_identity(&b, 0).is_err(),
        "tau=2 must fail at tolerance=0");
    // A very large tolerance lets through the truncation residual.
    assert!(verify_modular_identity(&b, i128::MAX).is_ok(),
        "MAX tolerance must always pass");
}

#[test]
fn rahul_sunita_epoch_randomness_full_arc() {
    // Full arc: epochs 0..5 — both producers derive identical beacons
    // and pass identity at τ=0; epochs 1..5 are distinct from τ=0.
    let b0 = compute_beacon(0);
    verify_modular_identity(&b0, 0).expect("epoch 0 must satisfy identity exactly");
    for tau in 1u64..=5 {
        let b = compute_beacon(tau);
        assert_eq!(b, compute_beacon(tau), "epoch {tau} must be deterministic");
        assert_ne!(b, b0, "epoch {tau} must differ from epoch 0");
    }
}

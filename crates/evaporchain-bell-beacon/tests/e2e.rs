//! End-to-end integration tests for evaporchain-bell-beacon.
//!
//! Non-trivial fixture: validator beacon certification session.
//!
//! Five measurement windows are collected; validators filter them through
//! the `bell_certified` gate to select quantum-genuine entropy sources.
//!
//!   Window 1 (classical attacker)  S=1800  → NOT certified (below boundary)
//!   Window 2 (classical boundary)  S=2000  → NOT certified (not strictly above)
//!   Window 3 (weak quantum)        S=2003  → certified (barely above boundary)
//!   Window 4 (Bell state)          S=2828  → certified (Tsirelson's bound: 2√2 × 1000)
//!   Window 5 (strong quantum)      S=2650  → certified
//!
//! The gate admits exactly 3 of the 5 windows as beacon-eligible.
//!
//! Doctrine claim (INVENTION_STACK §4.2): "Bell-Certified Beacon:
//! Device-independent randomness from CHSH Bell tests. No local hidden
//! variable model can produce S > 2000 — only quantum entanglement can.
//! A measured S > LOCAL_REALISM_S_MILLI (2000) is an unforgeable
//! proof of genuine randomness."
//!
//! Adversarial fixture: out-of-range correlations, tightened thresholds,
//! cartel-spoofing attempt at the classical boundary.
//!
//! INVENTION_STACK §4.2: Bell-Certified Beacon.

use evaporchain_bell_beacon::{bell_certified, chsh_s_value, ChshError, LOCAL_REALISM_S_MILLI};

// ── Helpers ───────────────────────────────────────────────────────────────

/// CHSH S value for a window defined by four correlation milli-values.
/// |E(a,b) − E(a,b') + E(a',b) + E(a',b')| in milli-units.
fn s(e_ab: i64, e_ab_prime: i64, e_a_prime_b: i64, e_a_prime_b_prime: i64) -> u64 {
    chsh_s_value(e_ab, e_ab_prime, e_a_prime_b, e_a_prime_b_prime).unwrap()
}

// ── Non-trivial fixture: five-window beacon session ───────────────────────

#[test]
fn beacon_session_exactly_3_of_5_windows_certified() {
    // Full five-window session. Validators admit only windows with S > 2000.
    //   Window 1: classical attacker,  S=1800 — NOT certified
    //   Window 2: classical boundary,  S=2000 — NOT certified (not strictly above)
    //   Window 3: weak quantum,        S=2003 — certified
    //   Window 4: Bell state,          S=2828 — certified (Tsirelson's bound)
    //   Window 5: strong quantum,      S=2650 — certified
    let windows: &[(i64, i64, i64, i64)] = &[
        (900, -900, 0, 0),     // W1: S=1800
        (1000, -1000, 0, 0),   // W2: S=2000
        (501, -501, 500, 501), // W3: S=2003
        (707, -707, 707, 707), // W4: S=2828 (Bell state)
        (663, -663, 663, 661), // W5: S=2650
    ];

    let certified_count = windows
        .iter()
        .filter(|(a, b, c, d)| {
            let s_val = chsh_s_value(*a, *b, *c, *d).unwrap();
            bell_certified(s_val, LOCAL_REALISM_S_MILLI)
        })
        .count();

    assert_eq!(
        certified_count, 3,
        "exactly 3 of 5 windows must be beacon-eligible"
    );

    // Spot-check the S values.
    assert_eq!(s(900, -900, 0, 0), 1800);
    assert_eq!(s(1000, -1000, 0, 0), 2000);
    assert_eq!(s(501, -501, 500, 501), 2003);
    assert_eq!(s(707, -707, 707, 707), 2828);
    assert_eq!(s(663, -663, 663, 661), 2650);
}

#[test]
fn classical_boundary_s2000_not_certified() {
    // S = 2000 is exactly the classical CHSH boundary — NOT certified.
    // The gate is strictly greater: `S > threshold`, not `≥`.
    // A classical adversary that somehow saturates the algebraic max
    // (2000 milli-units) is still rejected.
    let s_val = s(1000, -1000, 0, 0);
    assert_eq!(s_val, LOCAL_REALISM_S_MILLI);
    assert!(
        !bell_certified(s_val, LOCAL_REALISM_S_MILLI),
        "S=2000 must NOT be certified (boundary case, not strict violation)"
    );
}

#[test]
fn bell_state_s2828_certified_at_tsirelson_bound() {
    // Standard Bell-state measurement angles:
    //   E(a,b)=E(a',b)=E(a',b') = +0.707, E(a,b') = −0.707
    //   → S = 0.707 − (−0.707) + 0.707 + 0.707 = 4×0.707 ≈ 2.828 = 2√2
    // In milli-units: 707 + 707 + 707 + 707 = 2828 = round(2√2 × 1000).
    // This is Tsirelson's quantum upper bound — the maximum any quantum
    // system can achieve. All four correlations are equal in magnitude.
    let s_val = s(707, -707, 707, 707);
    assert_eq!(s_val, 2828);
    assert!(
        bell_certified(s_val, LOCAL_REALISM_S_MILLI),
        "Bell-state S=2828 must be certified (41% above classical bound)"
    );
}

#[test]
fn weak_quantum_just_above_boundary_certified() {
    // S = 2003 — barely above the classical boundary.
    // Even a marginal quantum violation is enough to certify randomness;
    // the device-independence argument holds for any S > 2000.
    let s_val = s(501, -501, 500, 501);
    assert_eq!(s_val, 2003);
    assert!(bell_certified(s_val, LOCAL_REALISM_S_MILLI));
}

#[test]
fn doctrine_quantum_exceeds_classical_by_at_least_40_percent() {
    // INVENTION_STACK §4.2 doctrine claim:
    // "No local hidden variable can produce S > 2000."
    // Quantum Bell states reach S ≈ 2828, 41.4% above the classical max.
    //
    // Bell state (S=2828) vs classical max (S=2000):
    //   improvement = (2828 − 2000) / 2000 = 41.4%
    let quantum_s = s(707, -707, 707, 707); // Tsirelson = 2828
    let classical_max = LOCAL_REALISM_S_MILLI;

    let improvement_pct = (quantum_s - classical_max) * 100 / classical_max;
    assert!(
        improvement_pct >= 40,
        "quantum Bell state must exceed classical max by ≥40%; got {improvement_pct}%"
    );
    assert!(bell_certified(quantum_s, classical_max));
    assert!(!bell_certified(classical_max, classical_max));
}

#[test]
fn s_value_is_absolute_symmetric_around_zero() {
    // The S value takes the absolute value of the signed sum. Negating
    // all four correlations produces the same |S|.
    let s_positive = s(707, -707, 707, 707); // Bell state, normal orientation
    let s_negative = s(-707, 707, -707, -707); // all negated → same |S|
    assert_eq!(
        s_positive, s_negative,
        "|S| must be symmetric: same value for negated correlations"
    );
    assert_eq!(s_positive, 2828);
}

#[test]
fn uncorrelated_measurements_yield_s_zero() {
    // If all four correlations are zero (no entanglement, pure noise),
    // the S value is 0 — well below the classical bound. The beacon
    // emitter produces no certified entropy.
    let s_val = s(0, 0, 0, 0);
    assert_eq!(s_val, 0);
    assert!(!bell_certified(s_val, LOCAL_REALISM_S_MILLI));
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_out_of_range_correlation_rejected() {
    // An attacker submits a correlation value outside [-1000, 1000].
    // This must be caught before any S computation — garbage input
    // cannot produce a falsely-certified beacon.
    let err = chsh_s_value(1001, 0, 0, 0).unwrap_err();
    assert_eq!(err, ChshError::OutOfRange(1001));

    let err2 = chsh_s_value(0, -1001, 0, 0).unwrap_err();
    assert_eq!(err2, ChshError::OutOfRange(-1001));

    let err3 = chsh_s_value(0, 0, 9999, 0).unwrap_err();
    assert_eq!(err3, ChshError::OutOfRange(9999));
}

#[test]
fn adversarial_tightened_threshold_rejects_marginal_signal() {
    // Some validators run at a higher security level, requiring
    // S > 2500 (25% above the classical boundary). This rejects
    // weak quantum signals while still admitting Tsirelson-grade ones.
    let tight_threshold: u64 = 2500;
    let marginal = s(600, -600, 500, 501); // S = |600+600+500+501| = 2201
    let bell_state = s(707, -707, 707, 707); // S = 2828

    assert!(
        !bell_certified(marginal, tight_threshold),
        "S=2201 must be rejected by the tightened threshold of 2500"
    );
    assert!(
        bell_certified(bell_state, tight_threshold),
        "S=2828 must still pass the tightened threshold of 2500"
    );
}

#[test]
fn adversarial_classical_cartel_cannot_certify_at_boundary() {
    // A coordinated cartel of classical validators tries to pool
    // measurements to forge a CHSH certification. The best any
    // classical hidden-variable strategy can do is S = 2000. The
    // strict-greater-than gate rejects this — cartel cannot spoof
    // device-independent randomness.
    //
    // Cartel forges the algebraic maximum with classical correlations
    // (e.g., deterministic +1/-1 assignments). S ≤ 2000 for any
    // LHV strategy; the gate requires S > 2000.
    let cartel_s = s(1000, -1000, 0, 0); // algebraic classical max
    assert_eq!(cartel_s, LOCAL_REALISM_S_MILLI);
    assert!(
        !bell_certified(cartel_s, LOCAL_REALISM_S_MILLI),
        "cartel classical max S=2000 must NOT pass the bell-certified gate"
    );
}

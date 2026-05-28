//! Coverage tests for Evaporative Pixel (EG-FSS) — energy-indexed
//! forward-secure signatures (Tier 2 per `INVENTION_STACK.md §4.2`).
//! Underwrites Evaporated-Fork Certs at the signature layer.
//!
//! Existing in-module tests cover sign/verify round-trip + wrong-
//! message + one-evolution scenario + forward-security flip. This
//! file adds coverage for:
//!
//!   - `EgFssKey::from_seed` initial state
//!   - `evolve` boundary cases (below / at / multiple-cross threshold)
//!   - `evolve` residual arithmetic and saturating safety
//!   - `evolve` determinism + one-way evolution
//!   - `sign` determinism + DST + message + period binding
//!   - `Signature` serde round-trip
//!   - `verify` PeriodMismatch path (existing tests only hit MacMismatch)
//!   - Error type Display + Eq ergonomics

use evaporchain_eg_fss::{sign, verify, EgFssKey, KeyError, Signature, VerifyError};

// =================================================================
// EgFssKey::from_seed
// =================================================================

#[test]
fn from_seed_initial_state() {
    let seed = [0xABu8; 32];
    let k = EgFssKey::from_seed(seed);
    assert_eq!(k.period_index, 0);
    assert_eq!(k.key_material, seed);
    assert_eq!(k.energy_residual, 0);
}

// =================================================================
// EgFssKey::evolve — boundary cases
// =================================================================

#[test]
fn evolve_zero_threshold_rejected() {
    let k = EgFssKey::from_seed([1u8; 32]);
    let err = k.evolve(1_000, 0).expect_err("zero threshold must fail");
    assert_eq!(err, KeyError::ZeroThreshold);
}

#[test]
fn evolve_below_threshold_keeps_period_and_accumulates_residual() {
    let k = EgFssKey::from_seed([1u8; 32]);
    let evolved = k.clone().evolve(99, 100).unwrap();
    assert_eq!(evolved.period_index, 0, "no crossing → no period change");
    assert_eq!(evolved.energy_residual, 99);
    assert_eq!(evolved.key_material, k.key_material, "key material unchanged");
}

#[test]
fn evolve_at_threshold_advances_exactly_one_period() {
    let k = EgFssKey::from_seed([1u8; 32]);
    let evolved = k.evolve(100, 100).unwrap();
    assert_eq!(evolved.period_index, 1);
    assert_eq!(evolved.energy_residual, 0, "exact crossing leaves no residual");
}

#[test]
fn evolve_collapses_multiple_period_crossings_in_one_call() {
    let k = EgFssKey::from_seed([1u8; 32]);
    let evolved = k.evolve(1_050, 100).unwrap();
    assert_eq!(evolved.period_index, 10, "1050 / 100 = 10 crossings");
    assert_eq!(evolved.energy_residual, 50, "1050 % 100 = 50 residual");
}

#[test]
fn evolve_carries_prior_residual_into_next_call() {
    let k0 = EgFssKey::from_seed([1u8; 32]);
    let k1 = k0.evolve(60, 100).unwrap();      // residual = 60, period = 0
    let k2 = k1.evolve(50, 100).unwrap();      // 60 + 50 = 110 → 1 crossing, residual=10
    assert_eq!(k2.period_index, 1);
    assert_eq!(k2.energy_residual, 10);
}

#[test]
fn evolve_saturating_add_safety_on_residual() {
    // residual + energy must clamp at u64::MAX without panic. Test
    // with a LARGE threshold so the resulting advance count is bounded
    // — using a small threshold here exposes the eg_fss_evolve_unbounded_loop
    // finding (u64::MAX/100 = 1.8×10^17 blake3 hashes), which is a
    // separate audit item, not a regression for this coverage PR.
    let mut k = EgFssKey::from_seed([1u8; 32]);
    k.energy_residual = u64::MAX - 10;
    let threshold = u64::MAX / 4;
    let evolved = k.evolve(1_000, threshold).expect("must not panic");
    assert!(evolved.period_index >= 1);
    assert!(evolved.period_index <= 5, "advances bounded by total/threshold");
}

#[test]
fn evolve_is_deterministic_for_fixed_inputs() {
    let k = EgFssKey::from_seed([5u8; 32]);
    let a = k.clone().evolve(500, 100).unwrap();
    let b = k.evolve(500, 100).unwrap();
    assert_eq!(a, b, "same seed + energy + threshold → same evolved key");
}

#[test]
fn evolve_changes_key_material_after_advance() {
    let seed = [9u8; 32];
    let k = EgFssKey::from_seed(seed);
    let evolved = k.evolve(100, 100).unwrap();
    assert_eq!(evolved.period_index, 1);
    assert_ne!(
        evolved.key_material, seed,
        "key_material must change on period advance (one-way evolution)"
    );
}

// =================================================================
// sign — determinism + binding
// =================================================================

#[test]
fn sign_is_deterministic() {
    let k = EgFssKey::from_seed([7u8; 32]);
    let s1 = sign(&k, b"hello");
    let s2 = sign(&k, b"hello");
    assert_eq!(s1, s2, "same key + message must yield same signature");
}

#[test]
fn sign_distinguishes_messages() {
    let k = EgFssKey::from_seed([7u8; 32]);
    let s1 = sign(&k, b"hello");
    let s2 = sign(&k, b"world");
    assert_ne!(s1.mac, s2.mac);
    assert_eq!(s1.period_index, s2.period_index, "period_index unaffected");
}

#[test]
fn sign_uses_dst_prefix() {
    // If a refactor drops the SIGN_TAG, this test fires: raw blake3
    // over the same fields must differ from the actual signature.
    let k = EgFssKey::from_seed([7u8; 32]);
    let s = sign(&k, b"hello");
    let mut raw = blake3::Hasher::new();
    raw.update(&k.key_material);
    raw.update(&k.period_index.to_le_bytes());
    raw.update(b"hello");
    let no_dst: [u8; 32] = *raw.finalize().as_bytes();
    assert_ne!(s.mac, no_dst, "signature must include SIGN_TAG DST");
}

#[test]
fn sign_period_in_mac_changes_with_evolution() {
    // Same key material *would* produce identical MACs except that
    // period_index is also folded in. Force the period to differ
    // while holding key_material constant.
    let k_a = EgFssKey::from_seed([7u8; 32]);
    let mut k_b = EgFssKey::from_seed([7u8; 32]);
    k_b.period_index = 42; // key_material unchanged
    let s_a = sign(&k_a, b"x");
    let s_b = sign(&k_b, b"x");
    assert_ne!(s_a.mac, s_b.mac, "period_index must affect the MAC");
    assert_eq!(s_a.period_index, 0);
    assert_eq!(s_b.period_index, 42);
}

#[test]
fn signature_serde_round_trips() {
    let k = EgFssKey::from_seed([7u8; 32]);
    let s = sign(&k, b"payload");
    let json = serde_json::to_string(&s).expect("serialize");
    let back: Signature = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, s);
}

// =================================================================
// verify — PeriodMismatch path
// =================================================================

#[test]
fn verify_rejects_period_index_mismatch() {
    let k = EgFssKey::from_seed([7u8; 32]);
    let s = sign(&k, b"hello");
    // Caller claims a different period to `verify` than the signature carries.
    let err = verify(k.key_material, 99, b"hello", &s).expect_err("must reject");
    match err {
        VerifyError::PeriodMismatch { claimed, expected } => {
            assert_eq!(claimed, 0, "signature's period_index");
            assert_eq!(expected, 99, "chain-side index");
        }
        other => panic!("expected PeriodMismatch, got {other:?}"),
    }
}

// =================================================================
// Error ergonomics
// =================================================================

#[test]
fn verify_error_displays_both_variants() {
    let m = VerifyError::MacMismatch.to_string();
    let p = VerifyError::PeriodMismatch { claimed: 1, expected: 2 }.to_string();
    assert!(m.contains("MAC") || m.contains("mismatch"), "got: {m}");
    assert!(p.contains("1") && p.contains("2"), "got: {p}");
}

#[test]
fn verify_error_eq_discriminates_variants() {
    let p12 = VerifyError::PeriodMismatch { claimed: 1, expected: 2 };
    let p12b = VerifyError::PeriodMismatch { claimed: 1, expected: 2 };
    let p13 = VerifyError::PeriodMismatch { claimed: 1, expected: 3 };
    assert_eq!(p12, p12b);
    assert_ne!(p12, p13);
    assert_ne!(p12, VerifyError::MacMismatch);
}

#[test]
fn key_error_eq_and_display() {
    let a = KeyError::ZeroThreshold;
    let b = KeyError::ZeroThreshold;
    assert_eq!(a, b);
    assert!(a.to_string().contains("threshold"), "got: {a}");
}

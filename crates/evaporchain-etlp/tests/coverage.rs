//! Coverage tests for the Cone-locked Capsule (ETLP) substrate.
//!
//! Existing in-module tests cover the happy path. This file adds:
//!
//!   - Capsule constructor error paths + boundary cases
//!   - Witness binding determinism + distinguishability (every
//!     field of the binding hash must change the output)
//!   - DST presence — binding hash MUST use the
//!     `evaporchain-etlp-witness` domain-separation tag
//!   - `can_unlock` boundary cases (exact threshold, observed-epoch
//!     == current_epoch, current_epoch < observed_epoch)
//!
//! See `research/INVENTION_STACK.md §4.2` for the doctrine anchor.

use evaporchain_etlp::capsule::{Capsule, CapsuleError};
use evaporchain_etlp::unlock::can_unlock;
use evaporchain_etlp::witness::EnergyWitness;
use evaporchain_energy_kernel::{ChainLambda, Lambda};

fn lambda_100() -> ChainLambda {
    ChainLambda::new(Lambda::from_epochs(100))
}

fn capsule_with(seal_epoch: u64, threshold: u64) -> Capsule {
    Capsule::new(seal_epoch, threshold, vec![0xAA; 16]).expect("valid")
}

fn witness_for(committed: u64, observed: u64, capsule: &Capsule) -> EnergyWitness {
    let binding = EnergyWitness::compute_binding(
        capsule.seal_epoch,
        capsule.energy_threshold,
        committed,
        observed,
    );
    EnergyWitness {
        committed_energy: committed,
        observed_epoch: observed,
        binding,
    }
}

// =================================================================
// Capsule constructor
// =================================================================

#[test]
fn capsule_new_rejects_empty_ciphertext() {
    let err = Capsule::new(0, 100, vec![]).expect_err("empty must fail");
    assert_eq!(err, CapsuleError::EmptyCiphertext);
}

#[test]
fn capsule_new_accepts_single_byte_ciphertext() {
    // The doctrine requires non-empty but not a minimum length above 1.
    let c = Capsule::new(0, 100, vec![0xFF]).expect("single-byte valid");
    assert_eq!(c.ciphertext_blob.len(), 1);
    assert_eq!(c.seal_epoch, 0);
    assert_eq!(c.energy_threshold, 100);
}

#[test]
fn capsule_serde_round_trips() {
    let c = Capsule::new(7, 999, b"hello world".to_vec()).expect("valid");
    let json = serde_json::to_string(&c).expect("serialize");
    let back: Capsule = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(c, back);
}

// =================================================================
// Witness binding hash — determinism + distinguishability
// =================================================================

#[test]
fn compute_binding_is_deterministic() {
    let a = EnergyWitness::compute_binding(0, 100, 1000, 5);
    let b = EnergyWitness::compute_binding(0, 100, 1000, 5);
    assert_eq!(a, b);
    // 1000 calls all identical.
    for _ in 0..1_000 {
        assert_eq!(EnergyWitness::compute_binding(0, 100, 1000, 5), a);
    }
}

#[test]
fn compute_binding_distinguishes_seal_epoch() {
    let a = EnergyWitness::compute_binding(0, 100, 1000, 5);
    let b = EnergyWitness::compute_binding(1, 100, 1000, 5);
    assert_ne!(a, b);
}

#[test]
fn compute_binding_distinguishes_threshold() {
    let a = EnergyWitness::compute_binding(0, 100, 1000, 5);
    let b = EnergyWitness::compute_binding(0, 101, 1000, 5);
    assert_ne!(a, b);
}

#[test]
fn compute_binding_distinguishes_committed_energy() {
    let a = EnergyWitness::compute_binding(0, 100, 1000, 5);
    let b = EnergyWitness::compute_binding(0, 100, 1001, 5);
    assert_ne!(a, b);
}

#[test]
fn compute_binding_distinguishes_observed_epoch() {
    let a = EnergyWitness::compute_binding(0, 100, 1000, 5);
    let b = EnergyWitness::compute_binding(0, 100, 1000, 6);
    assert_ne!(a, b);
}

#[test]
fn compute_binding_uses_dst_prefix() {
    // The binding must domain-separate against a raw blake3 hash of
    // the same fields. If a future refactor drops the DST, this test
    // fires loudly.
    let dst_binding = EnergyWitness::compute_binding(7, 100, 1000, 3);

    let mut raw = blake3::Hasher::new();
    raw.update(&7u64.to_le_bytes());
    raw.update(&100u64.to_le_bytes());
    raw.update(&1000u64.to_le_bytes());
    raw.update(&3u64.to_le_bytes());
    let no_dst: [u8; 32] = *raw.finalize().as_bytes();

    assert_ne!(
        dst_binding, no_dst,
        "compute_binding must include the evaporchain-etlp-witness DST"
    );
}

// =================================================================
// can_unlock boundary cases
// =================================================================

#[test]
fn unlock_at_exact_threshold_succeeds() {
    // committed == threshold; elapsed == 0 ⇒ remaining == threshold.
    // `remaining >= threshold` holds.
    let c = capsule_with(0, 500);
    let w = witness_for(500, 0, &c);
    assert!(can_unlock(&c, &w, lambda_100(), 0).expect("ok"));
}

#[test]
fn unlock_one_unit_below_threshold_fails() {
    let c = capsule_with(0, 500);
    let w = witness_for(499, 0, &c);
    assert!(!can_unlock(&c, &w, lambda_100(), 0).expect("ok"));
}

#[test]
fn unlock_at_observed_epoch_no_decay_applied() {
    // current_epoch == observed_epoch → elapsed=0 → no decay.
    let c = capsule_with(0, 500);
    let w = witness_for(1000, 50, &c);
    assert!(can_unlock(&c, &w, lambda_100(), 50).expect("ok"));
}

#[test]
fn unlock_current_epoch_before_observed_is_safe() {
    // current_epoch < observed_epoch must NOT panic; `saturating_sub`
    // clamps elapsed to 0 → no decay, just committed >= threshold check.
    let c = capsule_with(0, 500);
    let w = witness_for(1000, 100, &c);
    assert!(can_unlock(&c, &w, lambda_100(), 50).expect("ok — clamped elapsed"));
}

#[test]
fn unlock_at_one_half_life_yields_half_energy() {
    // half_life=100, elapsed=100 → energy halves. 1000 → 500.
    // threshold=500 ⇒ exactly at boundary, unlock succeeds.
    let c = capsule_with(0, 500);
    let w = witness_for(1000, 0, &c);
    assert!(can_unlock(&c, &w, lambda_100(), 100).expect("ok"));
}

#[test]
fn unlock_at_one_half_life_plus_one_epoch_below_threshold() {
    // Just past one half-life, energy drops just below threshold.
    let c = capsule_with(0, 500);
    let w = witness_for(1000, 0, &c);
    assert!(!can_unlock(&c, &w, lambda_100(), 101).expect("ok"));
}

#[test]
fn unlock_with_committed_energy_zero_at_zero_threshold_succeeds() {
    // Degenerate but legal: threshold=0 means "any energy unlocks";
    // committed=0 still satisfies `0 >= 0`.
    let c = capsule_with(0, 0);
    let w = witness_for(0, 0, &c);
    assert!(can_unlock(&c, &w, lambda_100(), 0).expect("ok"));
}

#[test]
fn unlock_rejects_witness_with_mismatched_seal_epoch_binding() {
    // Capsule says seal_epoch=10; witness's binding was computed
    // against seal_epoch=11. The pre-decay binding check must
    // catch this before any decay arithmetic runs.
    let c = capsule_with(10, 500);
    let mismatched_binding = EnergyWitness::compute_binding(
        /* WRONG */ 11,
        c.energy_threshold,
        1000,
        0,
    );
    let w = EnergyWitness {
        committed_energy: 1000,
        observed_epoch: 0,
        binding: mismatched_binding,
    };
    let err = can_unlock(&c, &w, lambda_100(), 0).expect_err("must reject");
    use evaporchain_etlp::witness::WitnessError;
    assert_eq!(err, WitnessError::BindingMismatch);
}

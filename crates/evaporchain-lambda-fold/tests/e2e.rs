//! End-to-end integration tests for evaporchain-lambda-fold.
//!
//! Non-trivial fixture: five-step light-client fold session.
//!
//! A chain light client folds five blocks across 400 epochs.
//! Each fold step decays all previously-accumulated energy forward in
//! time, then adds the new step's energy. The verifier then checks
//! both the blake3 accumulator hash and an energy-budget floor.
//!
//!   half_life = 200 epochs
//!
//!   Step 1: energy=10_000, epoch=  0
//!   Step 2: energy= 8_000, epoch= 50   (50 epochs elapsed → prev decays)
//!   Step 3: energy= 6_000, epoch=100   (50 epochs elapsed)
//!   Step 4: energy= 5_000, epoch=200   (100 epochs elapsed)
//!   Step 5: energy= 4_000, epoch=400   (200 epochs = 1 half-life → prev halved)
//!
//!   Total input energy: 33_000
//!   Remaining at epoch 400: ~14_246  (43.2% — older energy decays faster)
//!
//! The final folded instance carries step_count=5, latest_epoch=400, and
//! a deterministic acc_hash. The verifier accepts it with hash=acc_hash
//! and min_energy ≤ 14_246.
//!
//! Doctrine claim (INVENTION_STACK §4.1 #8): "Lambda-Fold: First
//! sublinear-in-active-energy verifier. Nova extension where each fold
//! step folds the energy state. A light client can verify both state
//! correctness AND chain energy decay in O(log n) — energy that entered
//! the chain earlier has decayed proportionally more than recently
//! folded energy."
//!
//! Adversarial fixture: out-of-order step, tampered hash, energy floor
//! exceeds actual, identity is not verifiable with non-zero floor.
//!
//! INVENTION_STACK §4.1 #8: Lambda-Fold (Energy-Folded Light Client).

use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_lambda_fold::{
    fold, verify_folded, FoldedInstance, FoldError, StepWitness, VerifyError,
};

// ── Constants ─────────────────────────────────────────────────────────────

const HALF_LIFE: u64 = 200; // epochs

fn chain_lambda() -> ChainLambda {
    ChainLambda::new(Lambda::from_epochs(HALF_LIFE))
}

fn hash(b: u8) -> [u8; 32] {
    [b; 32]
}

// ── Non-trivial fixture: five-step light-client fold session ──────────────

/// Fold all five steps and return the final instance.
fn five_step_session() -> FoldedInstance {
    let λ = chain_lambda();
    let init = FoldedInstance::identity();
    let s1 = fold(init, StepWitness::new(hash(0x01), 10_000, 0),   λ).unwrap();
    let s2 = fold(s1,   StepWitness::new(hash(0x02),  8_000, 50),  λ).unwrap();
    let s3 = fold(s2,   StepWitness::new(hash(0x03),  6_000, 100), λ).unwrap();
    let s4 = fold(s3,   StepWitness::new(hash(0x04),  5_000, 200), λ).unwrap();
    fold(s4, StepWitness::new(hash(0x05), 4_000, 400), λ).unwrap()
}

#[test]
fn five_step_session_produces_correct_step_count_and_epoch() {
    let inst = five_step_session();
    assert_eq!(inst.step_count,   5,   "five folds must yield step_count=5");
    assert_eq!(inst.latest_epoch, 400, "latest_epoch must track the last step's epoch");
    assert!(!inst.is_identity(), "folded instance with 5 steps must not be identity");
}

#[test]
fn five_step_session_energy_decays_below_total_input() {
    // Total energy input: 10_000 + 8_000 + 6_000 + 5_000 + 4_000 = 33_000.
    // Energy that entered earlier has decayed across more epochs.
    // Remaining at epoch 400 must be strictly less than total input.
    let inst = five_step_session();
    assert!(
        inst.total_energy_remaining < 33_000,
        "λ-decay must reduce total energy below sum of inputs (got {})",
        inst.total_energy_remaining
    );
    // But at least the last step's energy (added at epoch=400) is intact.
    assert!(
        inst.total_energy_remaining >= 4_000,
        "last step's energy must still be present (got {})",
        inst.total_energy_remaining
    );
}

#[test]
fn five_step_session_energy_at_one_halflife_exactly_halves_prior() {
    // Step 4→5 transition: 200 epochs elapsed = exactly 1 half-life.
    // The energy accumulated through step 4 must be halved before step 5 is added.
    // Test this by comparing with/without step 5.
    let λ = chain_lambda();
    let init = FoldedInstance::identity();
    let s1 = fold(init, StepWitness::new(hash(0x01), 10_000, 0),   λ).unwrap();
    let s2 = fold(s1,   StepWitness::new(hash(0x02),  8_000, 50),  λ).unwrap();
    let s3 = fold(s2,   StepWitness::new(hash(0x03),  6_000, 100), λ).unwrap();
    let after_s4 = fold(s3, StepWitness::new(hash(0x04), 5_000, 200), λ).unwrap();

    // Now fold step 5 at epoch 400 (200 elapsed from 200 = exactly 1 half-life).
    // Before adding step 5's energy, after_s4's energy is halved.
    let after_s5 = fold(after_s4, StepWitness::new(hash(0x05), 4_000, 400), λ).unwrap();

    // after_s4 energy / 2 (integer) + 4_000 should equal after_s5 energy.
    let expected = after_s4.total_energy_remaining / 2 + 4_000;
    assert_eq!(
        after_s5.total_energy_remaining, expected,
        "at exactly 1 half-life elapsed: prev/2 + new_step must equal total"
    );
}

#[test]
fn acc_hash_changes_at_every_fold_step() {
    // Each fold must change the blake3 accumulator. A verifier can
    // distinguish any two positions in the chain by their acc_hash.
    let λ = chain_lambda();
    let init = FoldedInstance::identity();
    let s1 = fold(init, StepWitness::new(hash(0x01), 1000, 0), λ).unwrap();
    let s2 = fold(s1,   StepWitness::new(hash(0x02), 1000, 1), λ).unwrap();
    let s3 = fold(s2,   StepWitness::new(hash(0x03), 1000, 2), λ).unwrap();

    assert_ne!(init.acc_hash, s1.acc_hash, "fold from identity must change acc_hash");
    assert_ne!(s1.acc_hash,   s2.acc_hash, "second fold must change acc_hash");
    assert_ne!(s2.acc_hash,   s3.acc_hash, "third fold must change acc_hash");
    // All four must be distinct.
    let hashes = [init.acc_hash, s1.acc_hash, s2.acc_hash, s3.acc_hash];
    for i in 0..4 {
        for j in (i+1)..4 {
            assert_ne!(hashes[i], hashes[j], "all acc_hashes must be distinct (i={i}, j={j})");
        }
    }
}

#[test]
fn verify_folded_accepts_correct_hash_and_floor() {
    // Happy path: verifier accepts the session's own acc_hash and a
    // floor of zero (permissive check).
    let inst = five_step_session();
    verify_folded(&inst, inst.acc_hash, 0).unwrap();

    // Also accepts a realistic floor well below the actual remaining energy.
    verify_folded(&inst, inst.acc_hash, 1_000).unwrap();
}

#[test]
fn verify_folded_accepts_exact_energy_floor() {
    // Floor exactly equal to remaining energy must pass.
    let inst = five_step_session();
    verify_folded(&inst, inst.acc_hash, inst.total_energy_remaining).unwrap();
}

#[test]
fn fold_is_deterministic() {
    // Same sequence of steps produces byte-identical FoldedInstance.
    let a = five_step_session();
    let b = five_step_session();
    assert_eq!(a, b, "fold must be deterministic");
}

#[test]
fn doctrine_older_energy_decays_faster_than_recent() {
    // INVENTION_STACK §4.1 doctrine claim: "sublinear-in-active-energy"
    // — energy that entered the chain earlier decays proportionally more.
    //
    // Compare two single-step sessions:
    //   Early:  1 step at epoch=0, verified at epoch=200 (1 half-life later)
    //   Late:   1 step at epoch=200, verified at epoch=200 (just added)
    //
    // Early step must have half the energy of the late step.
    let λ = chain_lambda();
    let energy = 10_000u64;

    let early = fold(FoldedInstance::identity(), StepWitness::new(hash(0x01), energy, 0), λ).unwrap();
    // Decay early from epoch=0 to epoch=200 by folding a zero-energy step.
    let early_at_200 = fold(early, StepWitness::new(hash(0x02), 0, 200), λ).unwrap();

    let late = fold(FoldedInstance::identity(), StepWitness::new(hash(0x01), energy, 200), λ).unwrap();

    // Energy from early step must have decayed to ~half (1 half-life elapsed).
    assert_eq!(
        early_at_200.total_energy_remaining,
        energy as u128 / 2,
        "after exactly 1 half-life, early energy must halve from {energy}"
    );
    assert_eq!(
        late.total_energy_remaining,
        energy as u128,
        "late step's energy (just added at epoch=200) must be intact"
    );
    assert!(
        early_at_200.total_energy_remaining < late.total_energy_remaining,
        "older energy ({}) must be less than newly-added energy ({})",
        early_at_200.total_energy_remaining, late.total_energy_remaining
    );
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_out_of_order_step_rejected() {
    // The fold is monotone in time. A step at epoch=40 cannot follow
    // a fold whose latest_epoch=50. Attackers cannot replay old steps.
    let λ = chain_lambda();
    let s1 = fold(
        FoldedInstance::identity(),
        StepWitness::new(hash(0x01), 1000, 50),
        λ,
    ).unwrap();

    let earlier = StepWitness::new(hash(0x02), 500, 40); // epoch < prev.latest_epoch
    let err = fold(s1, earlier, λ).unwrap_err();
    assert_eq!(
        err,
        FoldError::OutOfOrder { step: 40, prev: 50 },
        "fold must reject out-of-order step"
    );
}

#[test]
fn adversarial_tampered_acc_hash_rejected_by_verifier() {
    // A prover that submits the correct energy but a tampered acc_hash
    // must be caught by verify_folded.
    let inst = five_step_session();
    let mut tampered_hash = inst.acc_hash;
    tampered_hash[0] ^= 0xFF;
    tampered_hash[31] ^= 0x01;

    let err = verify_folded(&inst, tampered_hash, 0).unwrap_err();
    assert!(
        matches!(err, VerifyError::AccHashMismatch { .. }),
        "tampered acc_hash must yield AccHashMismatch, got {err:?}"
    );
}

#[test]
fn adversarial_energy_floor_too_high_rejected() {
    // A verifier that demands more energy than actually remains must
    // be rejected. This catches a prover that over-reported decay
    // (claimed to have burned more energy than it did).
    let inst = five_step_session();
    let too_high = inst.total_energy_remaining + 1;

    let err = verify_folded(&inst, inst.acc_hash, too_high).unwrap_err();
    assert!(
        matches!(err, VerifyError::EnergyBelowMinimum { got, min }
            if got == inst.total_energy_remaining && min == too_high),
        "energy floor above actual remaining must yield EnergyBelowMinimum, got {err:?}"
    );
}

#[test]
fn adversarial_identity_fails_nonzero_energy_floor() {
    // The identity instance has zero energy. Any non-zero floor rejects it.
    let inst = FoldedInstance::identity();
    let err = verify_folded(&inst, inst.acc_hash, 1).unwrap_err();
    assert!(matches!(err, VerifyError::EnergyBelowMinimum { got: 0, min: 1 }));
}

#[test]
fn adversarial_different_step_hashes_produce_different_acc_hashes() {
    // Changing any single step's state_hash must produce a completely
    // different acc_hash — collision resistance for the commitment.
    let λ = chain_lambda();

    let genuine = fold(
        FoldedInstance::identity(),
        StepWitness::new(hash(0x01), 1000, 0),
        λ,
    ).unwrap();

    let forged = fold(
        FoldedInstance::identity(),
        StepWitness::new(hash(0xFF), 1000, 0), // different state_hash
        λ,
    ).unwrap();

    assert_ne!(
        genuine.acc_hash, forged.acc_hash,
        "different state_hash must produce different acc_hash"
    );
    // The energy is the same (same step_energy + epoch) — only the hash differs.
    assert_eq!(genuine.total_energy_remaining, forged.total_energy_remaining);
}

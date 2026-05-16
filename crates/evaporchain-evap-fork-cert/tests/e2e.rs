//! End-to-end integration tests for evaporchain-evap-fork-cert.
//!
//! Non-trivial fixture: competing-fork evaporation at epoch 600.
//!
//! A chain's consensus must reject a competing fork whose aggregate
//! energy has decayed below the finalization threshold. The certificate
//! lets a light client verify this in O(1) — without scanning the fork.
//!
//!   half_life = 100 epochs   threshold = 300 energy units
//!
//!   Competing fork (5 blocks, all observed before epoch 400):
//!
//!     Block A  seed=500  observed=  0  elapsed=600  → decayed=  7
//!     Block B  seed=500  observed=100  elapsed=500  → decayed= 15
//!     Block C  seed=500  observed=200  elapsed=400  → decayed= 31
//!     Block D  seed=500  observed=300  elapsed=300  → decayed= 62
//!     Block E  seed=500  observed=400  elapsed=200  → decayed=125
//!
//!   Total seed energy: 2_500
//!   Decayed at epoch 600: 240
//!   Threshold: 300
//!   240 < 300 → EVAPORATED → certificate valid
//!
//! At threshold=200 the same fork is NOT evaporated (240 ≥ 200).
//!
//! Doctrine claim (INVENTION_STACK §4.1 #10): "Negative finality:
//! a fork cannot finalize because its aggregate energy decayed below
//! threshold. Light clients verify in O(1). Older energy decays
//! faster than recently-observed energy — the competing fork always
//! loses to the active chain as time passes."
//!
//! Adversarial fixture: tampered witness, tampered decayed energy,
//! tampered threshold, boundary-threshold, future block skipped.
//!
//! INVENTION_STACK §4.1 #10: Evaporated-Fork Certificates.

use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_evap_fork_cert::{
    prove_fork_evaporated, verify_evaporated_cert, CertError, ForkBlock,
};

// ── Constants ─────────────────────────────────────────────────────────────

const HALF_LIFE:   u64 = 100;
const EVAL_EPOCH:  u64 = 600;
const SEED_ENERGY: u64 = 500;
const THRESHOLD_EVAP:    u128 = 300; // 240 < 300 → evaporated
const THRESHOLD_NONEVAP: u128 = 200; // 240 ≥ 200 → NOT evaporated

fn chain_lambda() -> ChainLambda {
    ChainLambda::new(Lambda::from_epochs(HALF_LIFE))
}

fn fork_root() -> [u8; 32] { [0xABu8; 32] }

/// The five-block competing fork: all blocks observed before epoch 400.
fn competing_fork_blocks() -> Vec<ForkBlock> {
    vec![
        ForkBlock { seed_energy: SEED_ENERGY, observed_epoch:   0 },
        ForkBlock { seed_energy: SEED_ENERGY, observed_epoch: 100 },
        ForkBlock { seed_energy: SEED_ENERGY, observed_epoch: 200 },
        ForkBlock { seed_energy: SEED_ENERGY, observed_epoch: 300 },
        ForkBlock { seed_energy: SEED_ENERGY, observed_epoch: 400 },
    ]
}

// ── Non-trivial fixture: competing fork evaporation at epoch 600 ──────────

#[test]
fn competing_fork_evaporated_at_epoch_600() {
    // Full lifecycle: prove → verify.
    let cert = prove_fork_evaporated(
        fork_root(),
        &competing_fork_blocks(),
        chain_lambda(),
        EVAL_EPOCH,
        THRESHOLD_EVAP,
    );

    assert_eq!(cert.total_seed_energy, 2_500, "total seed must sum all 5 blocks");
    assert_eq!(cert.decayed_energy, 240,
        "decayed energy at epoch 600 must equal 7+15+31+62+125=240");
    assert!(
        cert.decayed_energy < cert.threshold,
        "decayed ({}) must be below threshold ({}) to evaporate",
        cert.decayed_energy, cert.threshold
    );

    verify_evaporated_cert(&cert).unwrap();
}

#[test]
fn per_block_decay_values_match_formula() {
    // Verify each block's contribution via single-block proofs.
    // Block at observed_epoch=X, evaluated at 600, elapsed=600-X.
    let λ = chain_lambda();
    let expected_per_block: &[(u64, u128)] = &[
        (  0,   7), // elapsed=600: 500>>6 = 7 (no frac remainder at 100-epoch boundaries)
        (100,  15), // elapsed=500: 500>>5 = 15
        (200,  31), // elapsed=400: 500>>4 = 31
        (300,  62), // elapsed=300: 500>>3 = 62
        (400, 125), // elapsed=200: 500>>2 = 125
    ];

    for &(observed, expected_decayed) in expected_per_block {
        let cert = prove_fork_evaporated(
            fork_root(),
            &[ForkBlock { seed_energy: SEED_ENERGY, observed_epoch: observed }],
            λ,
            EVAL_EPOCH,
            1,
        );
        assert_eq!(
            cert.decayed_energy, expected_decayed,
            "Block observed at epoch {observed}: expected decayed={expected_decayed}, got {}",
            cert.decayed_energy
        );
    }
}

#[test]
fn same_fork_not_evaporated_with_lower_threshold() {
    // Same blocks, threshold=200. Decayed=240 ≥ 200 → NOT evaporated.
    // The verifier must reject the certificate.
    let cert = prove_fork_evaporated(
        fork_root(),
        &competing_fork_blocks(),
        chain_lambda(),
        EVAL_EPOCH,
        THRESHOLD_NONEVAP,
    );
    assert_eq!(cert.decayed_energy, 240);
    let err = verify_evaporated_cert(&cert).unwrap_err();
    assert!(
        matches!(err, CertError::NotEvaporated { decayed: 240, threshold: 200 }),
        "must be NotEvaporated when decayed >= threshold, got {err:?}"
    );
}

#[test]
fn doctrine_older_blocks_decay_more_than_recent() {
    // INVENTION_STACK §4.1 doctrine: "older energy decays faster."
    // Block A (observed at epoch 0) has decayed to 7 by epoch 600.
    // Block E (observed at epoch 400) has decayed to 125 by epoch 600.
    // Both had identical seed energy (500). Time in chain = decay.
    let λ = chain_lambda();

    let cert_old    = prove_fork_evaporated(fork_root(), &[ForkBlock { seed_energy: 500, observed_epoch:   0 }], λ, EVAL_EPOCH, 1);
    let cert_recent = prove_fork_evaporated(fork_root(), &[ForkBlock { seed_energy: 500, observed_epoch: 400 }], λ, EVAL_EPOCH, 1);

    assert!(
        cert_old.decayed_energy < cert_recent.decayed_energy,
        "older block (decayed={}) must have less energy remaining than \
         recent block (decayed={})",
        cert_old.decayed_energy, cert_recent.decayed_energy
    );
    assert_eq!(cert_old.decayed_energy,    7,   "Block A (epoch 0) at epoch 600 must = 7");
    assert_eq!(cert_recent.decayed_energy, 125, "Block E (epoch 400) at epoch 600 must = 125");
}

#[test]
fn certificate_is_deterministic() {
    // Same prover inputs → byte-identical certificate every time.
    let a = prove_fork_evaporated(fork_root(), &competing_fork_blocks(), chain_lambda(), EVAL_EPOCH, THRESHOLD_EVAP);
    let b = prove_fork_evaporated(fork_root(), &competing_fork_blocks(), chain_lambda(), EVAL_EPOCH, THRESHOLD_EVAP);
    assert_eq!(a, b, "prove must be deterministic");
}

#[test]
fn empty_fork_has_zero_decayed_energy() {
    // A fork with no blocks cannot have any energy. O(1) proof that it
    // has evaporated against any positive threshold.
    let cert = prove_fork_evaporated(fork_root(), &[], chain_lambda(), EVAL_EPOCH, 1);
    assert_eq!(cert.decayed_energy, 0);
    assert_eq!(cert.total_seed_energy, 0);
    verify_evaporated_cert(&cert).unwrap();
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_tampered_decayed_energy_rejected() {
    // An attacker reduces the decayed_energy field to claim the fork
    // evaporated at a lower energy. The witness mismatch catches this.
    let mut cert = prove_fork_evaporated(
        fork_root(), &competing_fork_blocks(), chain_lambda(), EVAL_EPOCH, THRESHOLD_EVAP,
    );
    cert.decayed_energy = 1; // tampered to look more evaporated

    let err = verify_evaporated_cert(&cert).unwrap_err();
    assert!(
        matches!(err, CertError::WitnessMismatch { .. }),
        "tampered decayed_energy must yield WitnessMismatch, got {err:?}"
    );
}

#[test]
fn adversarial_tampered_threshold_rejected() {
    // An attacker raises the threshold to make the certificate valid for
    // a fork that would NOT have evaporated at the original threshold.
    let mut cert = prove_fork_evaporated(
        fork_root(), &competing_fork_blocks(), chain_lambda(), EVAL_EPOCH, THRESHOLD_NONEVAP,
    );
    // Fork NOT evaporated at 200 — attacker changes threshold to 300.
    cert.threshold = THRESHOLD_EVAP;

    let err = verify_evaporated_cert(&cert).unwrap_err();
    assert!(
        matches!(err, CertError::WitnessMismatch { .. }),
        "tampered threshold must yield WitnessMismatch, got {err:?}"
    );
}

#[test]
fn adversarial_tampered_fork_root_rejected() {
    // An attacker claims the certificate belongs to a different fork.
    let mut cert = prove_fork_evaporated(
        fork_root(), &competing_fork_blocks(), chain_lambda(), EVAL_EPOCH, THRESHOLD_EVAP,
    );
    cert.fork_root = [0xFF; 32]; // different fork root

    let err = verify_evaporated_cert(&cert).unwrap_err();
    assert!(
        matches!(err, CertError::WitnessMismatch { .. }),
        "tampered fork_root must yield WitnessMismatch"
    );
}

#[test]
fn adversarial_boundary_threshold_exact_is_not_evaporated() {
    // The gate is strict `<`: decayed == threshold means NOT evaporated.
    // Test that the boundary is not crossed.
    let cert = prove_fork_evaporated(
        fork_root(), &competing_fork_blocks(), chain_lambda(), EVAL_EPOCH, 240,
    );
    assert_eq!(cert.decayed_energy, 240, "decayed must equal 240");
    assert_eq!(cert.threshold, 240,     "threshold must equal 240");

    let err = verify_evaporated_cert(&cert).unwrap_err();
    assert!(
        matches!(err, CertError::NotEvaporated { decayed: 240, threshold: 240 }),
        "decayed == threshold must be NotEvaporated (strict <), got {err:?}"
    );
}

#[test]
fn adversarial_future_block_not_counted_in_decayed() {
    // A block whose observed_epoch is AFTER evaluated_at_epoch is skipped
    // in the decayed sum, but still counted in total_seed_energy.
    // This prevents a forger from injecting future blocks to inflate energy.
    let λ = chain_lambda();
    let blocks = vec![
        ForkBlock { seed_energy: 500, observed_epoch: 0 },   // counted
        ForkBlock { seed_energy: 999, observed_epoch: 999 }, // future — skipped in decayed
    ];
    let cert = prove_fork_evaporated(fork_root(), &blocks, λ, 100, 1);

    // total_seed includes both blocks.
    assert_eq!(cert.total_seed_energy, 1499, "future block seed must be in total");
    // decayed only includes the block at epoch 0 (elapsed=100, one halving → 500>>1=250).
    assert_eq!(cert.decayed_energy, 250,
        "future-block energy must NOT be in decayed sum");
}

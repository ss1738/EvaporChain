//! End-to-end integration tests for evaporchain-evap-fork-cert-v2.
//!
//! Non-trivial fixture: multi-round Bell-anchored fork evaporation monitor.
//!
//! A chain monitors three competing forks across two evaluation rounds.
//! Each round uses a freshly-published Bell seed as the certificate anchor,
//! preventing tactical forgers from pre-computing the witness before the
//! seed is known.
//!
//!   half_life = 100 epochs
//!
//!   Fork A (old, 3 blocks): seed_total=3500, evaluated at epoch=500
//!     Block 1  seed=2_000  epoch=  0  elapsed=500  → decayed= 62
//!     Block 2  seed=1_000  epoch=100  elapsed=400  → decayed= 62
//!     Block 3  seed=  500  epoch=200  elapsed=300  → decayed= 62
//!     total_decayed = 186 < threshold=300 → EVAPORATED ✓
//!
//!   Fork B (recent, 2 blocks): seed_total=8000, evaluated at epoch=500
//!     Block 1  seed=5_000  epoch=400  elapsed=100  → decayed=2_500
//!     Block 2  seed=3_000  epoch=450  elapsed= 50  → decayed=2_250
//!     total_decayed = 4_750 ≥ threshold=300 → NOT EVAPORATED ✗
//!
//!   Fork C (ancient, 1 block): seed_total=10000, evaluated at epoch=1000
//!     Block 1  seed=10_000 epoch=  0  elapsed=1000 → decayed=9
//!     total_decayed = 9 < threshold=100 → EVAPORATED ✓
//!
//! Round 1 (eval_epoch=500, Bell seed=[0xAA;32]): Fork A evaporated, B not.
//! Round 2 (eval_epoch=1000, Bell seed=[0xBB;32]): Fork C evaporated.
//!
//! Doctrine claim (INVENTION_STACK §4.1 #10 / V2 upgrade): "Bell-anchored
//! witness prevents pre-computation. A tactical forker who knows the
//! chain's half-life can compute V1 witnesses in advance; V2 binds the
//! witness to a Bell seed that is unpublished until the chain confirms
//! blocks at the evaluation epoch. Different seeds → different witnesses
//! for identical fork state — the anti-grinding property transfers from
//! the Bell Beacon to the fork certificate."
//!
//! Adversarial fixture: causality violation (seed issued after eval epoch),
//! tampered anchor, tampered anchor epoch, tampered decayed energy,
//! boundary threshold (strict <).
//!
//! INVENTION_STACK §4.1 #10: Evaporated-Fork Certificates V2.

use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_evap_fork_cert::ForkBlock;
use evaporchain_evap_fork_cert_v2::{
    prove_fork_evaporated_v2, verify_evaporated_cert_v2, CertV2Error,
};

// ── Constants ─────────────────────────────────────────────────────────────

const HALF_LIFE: u64 = 100;

fn lambda() -> ChainLambda {
    ChainLambda::new(Lambda::from_epochs(HALF_LIFE))
}

fn fork_root_a() -> [u8; 32] {
    [0xA0u8; 32]
}
fn fork_root_b() -> [u8; 32] {
    [0xB0u8; 32]
}
fn fork_root_c() -> [u8; 32] {
    [0xC0u8; 32]
}

fn bell_seed_round1() -> [u8; 32] {
    [0xAAu8; 32]
}
fn bell_seed_round2() -> [u8; 32] {
    [0xBBu8; 32]
}

fn fork_a_blocks() -> Vec<ForkBlock> {
    vec![
        ForkBlock {
            seed_energy: 2_000,
            observed_epoch: 0,
        },
        ForkBlock {
            seed_energy: 1_000,
            observed_epoch: 100,
        },
        ForkBlock {
            seed_energy: 500,
            observed_epoch: 200,
        },
    ]
}

fn fork_b_blocks() -> Vec<ForkBlock> {
    vec![
        ForkBlock {
            seed_energy: 5_000,
            observed_epoch: 400,
        },
        ForkBlock {
            seed_energy: 3_000,
            observed_epoch: 450,
        },
    ]
}

fn fork_c_blocks() -> Vec<ForkBlock> {
    vec![ForkBlock {
        seed_energy: 10_000,
        observed_epoch: 0,
    }]
}

// ── Non-trivial fixture: multi-round Bell-anchored fork monitor ───────────

#[test]
fn round1_fork_a_evaporated_with_bell_seed() {
    // Fork A: 3 old blocks, all 5–3 halvings deep at epoch 500.
    // decayed = 62+62+62 = 186 < threshold=300 → EVAPORATED.
    let cert = prove_fork_evaporated_v2(
        fork_root_a(),
        &fork_a_blocks(),
        lambda(),
        500,
        300,
        bell_seed_round1(),
        500,
    )
    .unwrap();

    assert_eq!(
        cert.decayed_energy, 186,
        "Fork A decayed must equal 62+62+62=186"
    );
    assert_eq!(
        cert.total_seed_energy, 3_500,
        "Fork A total seed must sum 3 blocks"
    );
    assert!(
        cert.decayed_energy < cert.threshold,
        "Fork A must be evaporated at epoch 500"
    );

    verify_evaporated_cert_v2(&cert).unwrap();
}

#[test]
fn round1_fork_b_not_evaporated_recent_blocks() {
    // Fork B: 2 recent blocks, only 1 halving or less at epoch 500.
    // decayed = 2500+2250 = 4750 ≥ threshold=300 → NOT EVAPORATED.
    let cert = prove_fork_evaporated_v2(
        fork_root_b(),
        &fork_b_blocks(),
        lambda(),
        500,
        300,
        bell_seed_round1(),
        500,
    )
    .unwrap();

    assert_eq!(
        cert.decayed_energy, 4_750,
        "Fork B decayed must equal 2500+2250=4750"
    );
    assert!(
        cert.decayed_energy >= cert.threshold,
        "Fork B must NOT be evaporated — too much energy remaining"
    );

    let err = verify_evaporated_cert_v2(&cert).unwrap_err();
    assert!(
        matches!(
            err,
            CertV2Error::NotEvaporated {
                decayed: 4750,
                threshold: 300
            }
        ),
        "must be NotEvaporated, got {err:?}"
    );
}

#[test]
fn round2_fork_c_evaporated_at_epoch_1000() {
    // Fork C: 1 ancient block, 10 halvings deep at epoch 1000.
    // decayed = 10_000 >> 10 = 9 < threshold=100 → EVAPORATED.
    let cert = prove_fork_evaporated_v2(
        fork_root_c(),
        &fork_c_blocks(),
        lambda(),
        1_000,
        100,
        bell_seed_round2(),
        1_000,
    )
    .unwrap();

    assert_eq!(
        cert.decayed_energy, 9,
        "Fork C at 10 halvings: 10_000>>10=9"
    );
    verify_evaporated_cert_v2(&cert).unwrap();
}

#[test]
fn doctrine_different_bell_seeds_produce_different_witnesses() {
    // INVENTION_STACK §4.1 V2 doctrine: different Bell seeds → different
    // witnesses for the SAME fork state. A forger cannot pre-compute the
    // witness without knowing which seed the chain will publish.
    let cert_aa = prove_fork_evaporated_v2(
        fork_root_a(),
        &fork_a_blocks(),
        lambda(),
        500,
        300,
        bell_seed_round1(),
        500,
    )
    .unwrap();
    let cert_bb = prove_fork_evaporated_v2(
        fork_root_a(),
        &fork_a_blocks(),
        lambda(),
        500,
        300,
        bell_seed_round2(),
        500,
    )
    .unwrap();

    // Same fork state, same eval epoch, same threshold — ONLY seed differs.
    assert_eq!(
        cert_aa.decayed_energy, cert_bb.decayed_energy,
        "energy must be identical for same fork state"
    );
    assert_ne!(
        cert_aa.witness, cert_bb.witness,
        "different Bell seeds must produce different witnesses"
    );

    // Both verify against their own witnesses.
    verify_evaporated_cert_v2(&cert_aa).unwrap();
    verify_evaporated_cert_v2(&cert_bb).unwrap();

    // Cross-verify fails (anchor mismatch → witness mismatch).
    let mut cross = cert_aa.clone();
    cross.bell_seed_anchor = bell_seed_round2();
    assert!(
        verify_evaporated_cert_v2(&cross).is_err(),
        "certificate for seed_round1 must not verify with seed_round2 anchor"
    );
}

#[test]
fn same_seed_produces_deterministic_witness() {
    // BFT validators who see the same Bell seed and fork state must
    // derive the same V2 witness — cross-validator agreement.
    let a = prove_fork_evaporated_v2(
        fork_root_a(),
        &fork_a_blocks(),
        lambda(),
        500,
        300,
        bell_seed_round1(),
        500,
    )
    .unwrap();
    let b = prove_fork_evaporated_v2(
        fork_root_a(),
        &fork_a_blocks(),
        lambda(),
        500,
        300,
        bell_seed_round1(),
        500,
    )
    .unwrap();
    assert_eq!(a, b, "same inputs must produce identical certificate");
}

#[test]
fn anchor_at_exactly_eval_epoch_accepted() {
    // seed_anchor_epoch == evaluated_at_epoch is the tightest valid case —
    // the Bell seed was published exactly when the fork is being evaluated.
    let cert = prove_fork_evaporated_v2(
        fork_root_a(),
        &fork_a_blocks(),
        lambda(),
        500, // eval_epoch
        300, // threshold
        bell_seed_round1(),
        500, // anchor_epoch == eval_epoch
    )
    .unwrap();
    verify_evaporated_cert_v2(&cert).unwrap();
}

#[test]
fn anchor_well_before_eval_epoch_accepted() {
    // Anchor epoch=0 (genesis Bell seed), eval at epoch=500.
    let cert = prove_fork_evaporated_v2(
        fork_root_a(),
        &fork_a_blocks(),
        lambda(),
        500,
        300,
        bell_seed_round1(),
        0,
    )
    .unwrap();
    verify_evaporated_cert_v2(&cert).unwrap();
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_causality_violation_at_prove_time() {
    // seed_anchor_epoch > evaluated_at_epoch — the seed was published AFTER
    // the epoch the certificate claims to evaluate. Rejected by prover.
    let err = prove_fork_evaporated_v2(
        fork_root_a(),
        &fork_a_blocks(),
        lambda(),
        500, // eval_epoch
        300,
        bell_seed_round1(),
        501, // anchor AFTER eval → causality violation
    )
    .unwrap_err();
    assert_eq!(
        err,
        CertV2Error::CausalityViolation {
            anchor: 501,
            eval: 500
        },
        "prover must reject causality violation"
    );
}

#[test]
fn adversarial_causality_violation_at_verify_time() {
    // A hand-crafted cert with anchor_epoch > eval_epoch must be caught
    // by the verifier even if the prover didn't build it.
    let mut cert = prove_fork_evaporated_v2(
        fork_root_a(),
        &fork_a_blocks(),
        lambda(),
        500,
        300,
        bell_seed_round1(),
        500,
    )
    .unwrap();
    cert.seed_anchor_epoch = 501; // post-mutate to violate causality

    let err = verify_evaporated_cert_v2(&cert).unwrap_err();
    assert!(
        matches!(
            err,
            CertV2Error::CausalityViolation {
                anchor: 501,
                eval: 500
            }
        ),
        "verifier must catch causality violation in hand-crafted cert"
    );
}

#[test]
fn adversarial_tampered_bell_anchor_rejected() {
    // A forger swaps the Bell seed after certification. The witness
    // mismatch catches the tamper — the anchor is bound into the hash.
    let mut cert = prove_fork_evaporated_v2(
        fork_root_a(),
        &fork_a_blocks(),
        lambda(),
        500,
        300,
        bell_seed_round1(),
        500,
    )
    .unwrap();
    cert.bell_seed_anchor = [0xFFu8; 32]; // tampered seed

    let err = verify_evaporated_cert_v2(&cert).unwrap_err();
    assert!(
        matches!(err, CertV2Error::WitnessMismatch { .. }),
        "tampered bell_seed_anchor must yield WitnessMismatch"
    );
}

#[test]
fn adversarial_boundary_threshold_strict_less() {
    // The evaporation gate is strict `<`. When decayed == threshold
    // the fork has NOT evaporated. Verify the boundary.
    let cert = prove_fork_evaporated_v2(
        fork_root_a(),
        &fork_a_blocks(),
        lambda(),
        500,
        186, // threshold == decayed
        bell_seed_round1(),
        500,
    )
    .unwrap();
    assert_eq!(
        cert.decayed_energy, cert.threshold,
        "must test the boundary"
    );

    let err = verify_evaporated_cert_v2(&cert).unwrap_err();
    assert!(
        matches!(
            err,
            CertV2Error::NotEvaporated {
                decayed: 186,
                threshold: 186
            }
        ),
        "decayed == threshold must be NotEvaporated (strict <)"
    );
}

#[test]
fn adversarial_v1_witness_does_not_satisfy_v2_verifier() {
    // A forger substitutes the V1 witness into a V2 cert to try to
    // bypass the anchor binding. The V2 verifier re-derives independently
    // and detects the mismatch.
    use evaporchain_evap_fork_cert::prove_fork_evaporated;

    let v1_cert = prove_fork_evaporated(fork_root_a(), &fork_a_blocks(), lambda(), 500, 300);

    let mut v2_cert = prove_fork_evaporated_v2(
        fork_root_a(),
        &fork_a_blocks(),
        lambda(),
        500,
        300,
        bell_seed_round1(),
        500,
    )
    .unwrap();
    v2_cert.witness = v1_cert.witness; // inject V1 witness into V2 cert

    let err = verify_evaporated_cert_v2(&v2_cert).unwrap_err();
    assert!(
        matches!(err, CertV2Error::WitnessMismatch { .. }),
        "V1 witness must not satisfy V2 verifier"
    );
}

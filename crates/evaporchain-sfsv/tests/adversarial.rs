//! SFSV adversarial test suite — direct mapping of the eight-row threat
//! model from `research/SFSV_ARCHITECTURE.md` §8 to integration tests
//! against the substrate-crate API.
//!
//! Each test names the §8 adversary it instantiates. Tests that are
//! chain-level only (B censor, C re-org, H quantum) are NOT in this
//! file — they belong to the consensus / cluster soak harness. This
//! file covers the substrate-crate-testable adversaries:
//!
//!   §8.1 — Adversary A: Present-Self Reneger
//!   §8.4 — Adversary D: State-Bloat / Dust Spammer
//!   §8.5 — Adversary E: Replay / Double-Reclaim
//!   §8.6 — Adversary F: MEV / Transfer-Claim Race
//!   §8.7 — Adversary G: Beneficiary-Key Loss (the testable corollary:
//!          a non-holder cannot transfer the claim out)
//!
//! Plus cross-cutting predicate-purity invariants the doctrine demands
//! (re-org safety + replay safety + DoS safety all depend on purity).

use evaporchain_sfsv::payout::{payout, PayoutError};
use evaporchain_sfsv::predicate::{evaluate, Predicate, PredicateContext};
use evaporchain_sfsv::vault::{Vault, VaultError, VaultStatus};
use evaporchain_types::{AccountAddress, Energy, Epoch, HalfLife};

// ── helpers ──────────────────────────────────────────────────────

fn addr(b: u8) -> AccountAddress {
    let mut x = [0u8; 32];
    x[0] = b;
    x
}

fn ctx(epoch_now: Epoch) -> PredicateContext {
    PredicateContext { epoch_now }
}

fn vault(predicate: Predicate, deposit: Energy) -> Vault {
    Vault::create(
        [0xAB; 32],
        /* creator     */ addr(0x01),
        /* future_self */ addr(0x02),
        deposit,
        predicate,
        /* created_at  */ 0,
    )
    .expect("valid vault construction")
}

fn energy_decays_predicate(threshold: Energy) -> Predicate {
    Predicate::EnergyDecaysBelow {
        initial_energy: 1_000_000,
        half_life: HalfLife::from(64u64),
        created_at_epoch: 0,
        threshold,
    }
}

// =================================================================
// §8.1 — Adversary A: Present-Self Reneger
// =================================================================

#[test]
fn adversary_a_payout_before_epoch_trips_is_rejected() {
    let mut v = vault(Predicate::EpochReached { release_epoch: 100 }, 1_000);
    let err = payout(&mut v, 50).expect_err("predicate not yet satisfied");
    assert_eq!(err, PayoutError::PredicateNotSatisfied { epoch_now: 50 });
    assert!(v.is_locked(), "vault must remain Locked after failed payout");
}

#[test]
fn adversary_a_payout_before_energy_decays_is_rejected() {
    // initial_energy=1_000_000, half_life=64. At epoch 0 the energy is
    // 1_000_000 >= any reasonable threshold ⇒ predicate false.
    let mut v = vault(energy_decays_predicate(500_000), 1_000);
    let err = payout(&mut v, 0).expect_err("predicate not yet satisfied at t=0");
    assert_eq!(err, PayoutError::PredicateNotSatisfied { epoch_now: 0 });
    assert!(v.is_locked());
}

#[test]
fn adversary_a_repeated_premature_attempts_do_not_drain_or_corrupt() {
    let mut v = vault(Predicate::EpochReached { release_epoch: 1_000 }, 42);
    for epoch in [0u64, 1, 50, 500, 999] {
        let err = payout(&mut v, epoch).expect_err("must remain unsatisfied");
        assert!(matches!(err, PayoutError::PredicateNotSatisfied { .. }));
    }
    // Vault unchanged after every adversarial attempt.
    assert_eq!(v.deposit, 42);
    assert!(v.is_locked());
    // The legitimate payout still succeeds at the trip epoch.
    let r = payout(&mut v, 1_000).expect("trip epoch");
    assert_eq!(r.amount, 42);
}

// =================================================================
// §8.4 — Adversary D: State-Bloat / Dust Spammer
// =================================================================

#[test]
fn adversary_d_zero_deposit_vault_is_rejected_at_construction() {
    let err = Vault::create(
        [0u8; 32],
        addr(0x01),
        addr(0x02),
        /* deposit */ 0,
        Predicate::EpochReached { release_epoch: 1 },
        0,
    )
    .expect_err("zero deposit must be rejected");
    assert_eq!(err, VaultError::ZeroDeposit);
}

#[test]
fn adversary_d_minimum_unit_deposit_constructs_but_documents_floor_gap() {
    // A single energy unit is currently *valid* at the crate API — the
    // dust-spam mitigation is a chain-level admission rule (gas premium
    // proportional to expected on-chain lifetime, per §8.4). This test
    // pins that the substrate crate does NOT silently reject deposit=1,
    // so the admission gate must live above this layer.
    let v = vault(Predicate::EpochReached { release_epoch: 1 }, 1);
    assert_eq!(v.deposit, 1);
    assert!(v.is_locked());
}

// =================================================================
// §8.5 — Adversary E: Replay / Double-Reclaim
// =================================================================

#[test]
fn adversary_e_double_payout_on_same_vault_is_rejected() {
    let mut v = vault(Predicate::EpochReached { release_epoch: 10 }, 1_000);
    let first = payout(&mut v, 10).expect("first payout");
    assert_eq!(first.amount, 1_000);

    let err = payout(&mut v, 10).expect_err("second payout must fail");
    assert_eq!(err, PayoutError::AlreadyReleased);
    assert!(!v.is_locked(), "vault must remain Released, not relocked");
}

#[test]
fn adversary_e_replay_with_different_epoch_after_release_still_rejected() {
    let mut v = vault(Predicate::EpochReached { release_epoch: 10 }, 100);
    payout(&mut v, 10).expect("first payout");
    // Adversary fabricates a fresh epoch hoping the released-state check
    // is skipped under future-epoch input.
    for replay_epoch in [11u64, 12, 100, u64::MAX] {
        let err = payout(&mut v, replay_epoch).expect_err("replay rejected");
        assert_eq!(err, PayoutError::AlreadyReleased);
    }
}

#[test]
fn adversary_e_replay_after_energy_release_still_rejected() {
    // Use a small initial_energy + low threshold so the predicate trips
    // by some far-future epoch.
    let p = Predicate::EnergyDecaysBelow {
        initial_energy: 1_000,
        half_life: HalfLife::from(1u64), // halves every epoch ⇒ rapid decay
        created_at_epoch: 0,
        threshold: 1, // releases as soon as live energy drops below 1
    };
    let mut v = vault(p, 100);
    payout(&mut v, 50).expect("first payout (energy long since decayed below 1)");
    let err = payout(&mut v, 51).expect_err("replay rejected");
    assert_eq!(err, PayoutError::AlreadyReleased);
}

// =================================================================
// §8.6 — Adversary F: MEV / Transfer-Claim Race
// =================================================================

#[test]
fn adversary_f_non_holder_cannot_transfer_claim() {
    let mut v = vault(Predicate::EpochReached { release_epoch: 1_000 }, 1);
    let mallory = addr(0x99);
    let bob = addr(0x42);
    // future_self == addr(0x02), so holder at creation is addr(0x02).
    let err = v
        .transfer_claim(mallory, bob)
        .expect_err("non-holder transfer rejected");
    assert!(matches!(err, VaultError::NotCurrentHolder { .. }));
    assert_eq!(v.current_holder(), Some(addr(0x02)));
}

#[test]
fn adversary_f_transfer_chain_preserves_holder_lineage() {
    let mut v = vault(Predicate::EpochReached { release_epoch: 1_000 }, 1);
    let initial_holder = addr(0x02);
    let bob = addr(0x42);
    let carol = addr(0x43);

    // initial → bob (legitimate)
    v.transfer_claim(initial_holder, bob).expect("initial → bob");
    assert_eq!(v.current_holder(), Some(bob));

    // initial_holder is now stale — cannot transfer again.
    let stale = v
        .transfer_claim(initial_holder, carol)
        .expect_err("stale holder rejected");
    assert!(matches!(stale, VaultError::NotCurrentHolder { .. }));

    // bob hands off to carol.
    v.transfer_claim(bob, carol).expect("bob → carol");
    assert_eq!(v.current_holder(), Some(carol));

    // bob now stale.
    let bob_stale = v
        .transfer_claim(bob, addr(0x44))
        .expect_err("bob is now stale");
    assert!(matches!(bob_stale, VaultError::NotCurrentHolder { .. }));
}

#[test]
fn adversary_f_transfer_after_release_is_rejected() {
    let mut v = vault(Predicate::EpochReached { release_epoch: 1 }, 1);
    payout(&mut v, 1).expect("payout");
    // Vault terminal — no holder.
    assert_eq!(v.current_holder(), None);
    let err = v
        .transfer_claim(addr(0x02), addr(0x99))
        .expect_err("transfer after release rejected");
    assert_eq!(err, VaultError::NotLocked);
}

#[test]
fn adversary_f_self_referential_transfer_succeeds_but_changes_nothing() {
    // A holder transferring to themselves is a no-op at the API level.
    // Pin that this is allowed (gas-bound on chain) and doesn't corrupt
    // state — an attacker can't use self-transfer to bypass holder check.
    let mut v = vault(Predicate::EpochReached { release_epoch: 1_000 }, 1);
    v.transfer_claim(addr(0x02), addr(0x02)).expect("self-transfer");
    assert_eq!(v.current_holder(), Some(addr(0x02)));
}

// =================================================================
// §8.7 — Adversary G: Beneficiary-Key Loss (testable corollary)
// =================================================================

#[test]
fn adversary_g_lost_key_payout_credits_listed_holder_not_creator() {
    // SFSV v1.0 is single-key — if the future_self key is lost, the
    // payout still goes to that address (the chain has no recovery
    // path; this is by design until the §5.4 threshold variant ships).
    // Pin this so a future refactor doesn't silently redirect payout
    // to the creator on key-loss assumption.
    let mut v = vault(Predicate::EpochReached { release_epoch: 1 }, 999);
    let result = payout(&mut v, 1).expect("payout");
    assert_eq!(
        result.paid_to,
        addr(0x02),
        "payout must go to future_self/holder, NOT creator (addr 0x01)"
    );
    match v.status {
        VaultStatus::Released { paid_to, .. } => assert_eq!(paid_to, addr(0x02)),
        _ => panic!("expected Released"),
    }
}

// =================================================================
// Cross-cutting: predicate purity (re-org / replay / DoS safety)
// =================================================================

#[test]
fn predicate_evaluate_is_referentially_transparent() {
    let p = energy_decays_predicate(500_000);
    let c = ctx(2_048);
    // 1000 evaluations must all return the same answer — no hidden
    // counter, no system-clock leak, no PRNG dependency.
    let first = evaluate(&p, c);
    for _ in 0..1_000 {
        assert_eq!(evaluate(&p, c), first);
    }
}

#[test]
fn predicate_evaluate_does_not_mutate_predicate() {
    // Pin that &Predicate is observably const across many evaluations.
    let p = Predicate::EpochReached { release_epoch: 42 };
    for epoch in 0..200u64 {
        let _ = evaluate(&p, ctx(epoch));
    }
    // Match the predicate again — must be the exact same shape.
    assert_eq!(p, Predicate::EpochReached { release_epoch: 42 });
}

#[test]
fn predicate_evaluate_extreme_values_do_not_panic() {
    // Re-org / DoS safety: u64::MAX, 0, and threshold/release at
    // boundary must never trigger arithmetic panics in debug build.
    let cases = [
        (Predicate::EpochReached { release_epoch: u64::MAX }, 0),
        (Predicate::EpochReached { release_epoch: 0 }, u64::MAX),
        (
            Predicate::EnergyDecaysBelow {
                initial_energy: u64::MAX,
                half_life: HalfLife::from(1u64),
                created_at_epoch: 0,
                threshold: 0,
            },
            u64::MAX,
        ),
        (
            Predicate::EnergyDecaysBelow {
                initial_energy: 1,
                half_life: HalfLife::from(u64::MAX),
                created_at_epoch: 0,
                threshold: u64::MAX,
            },
            u64::MAX,
        ),
    ];
    for (p, epoch_now) in cases {
        let _ = evaluate(&p, ctx(epoch_now));
    }
}

#[test]
fn vault_create_with_zero_predicate_threshold_is_unsatisfiable() {
    // Pin doctrine note: threshold=0 ⇒ never trips even at fully
    // evaporated energy (energy < 0 impossible for u64). An adversary
    // depositing into a threshold=0 vault is depositing into a black
    // hole; this is intentional, not a bug, and must stay testable.
    let p = Predicate::EnergyDecaysBelow {
        initial_energy: 1_000,
        half_life: HalfLife::from(1u64),
        created_at_epoch: 0,
        threshold: 0,
    };
    let mut v = vault(p, 1_000);
    for epoch in [9_999u64, 99_999, u64::MAX / 2] {
        let err = payout(&mut v, epoch).expect_err("never trips");
        assert!(matches!(err, PayoutError::PredicateNotSatisfied { .. }));
    }
    assert!(v.is_locked());
}

//! SFSV adversarial test suite — direct mapping of the eight-row threat
//! model from `research/SFSV_ARCHITECTURE.md` §8 to integration tests
//! against the substrate-crate API.
//!
//! **Reconciled 2026-05-16 to model (a)** (the `.es`-faithful predicate):
//! `EnergyDecaysBelow` is now a *pure comparison over engine-supplied
//! live energy* — the predicate no longer re-derives decay from frozen
//! `(initial_energy, half_life, created_at)` params (invariant #1) and is
//! refresh-aware. Every test name, §8 mapping, and intent below is
//! preserved from the original suite; only the API mechanics were
//! adapted (`PredicateContext{epoch_now,contract_energy}`,
//! `Predicate::EnergyDecaysBelow{threshold}`, `payout(v,epoch,energy)`),
//! plus one new refresh-aware case the old frozen-formula API could not
//! express (`adversary_a_refresh_then_reclaim_is_rejected`).
//!
//! Chain-level-only adversaries (B censor, C re-org, H quantum) are NOT
//! here — they belong to the consensus / cluster soak harness. This file
//! covers the substrate-crate-testable adversaries:
//!   §8.1 A: Present-Self Reneger · §8.4 D: Dust Spammer
//!   §8.5 E: Replay/Double-Reclaim · §8.6 F: MEV/Transfer-Claim Race
//!   §8.7 G: Beneficiary-Key Loss (testable corollary)
//! plus predicate-purity invariants (re-org/replay/DoS safety).

use evaporchain_sfsv::payout::{payout, PayoutError};
use evaporchain_sfsv::predicate::{evaluate, Predicate, PredicateContext};
use evaporchain_sfsv::vault::{Vault, VaultError, VaultStatus};
use evaporchain_types::{AccountAddress, Energy, Epoch};

// ── helpers ──────────────────────────────────────────────────────

fn addr(b: u8) -> AccountAddress {
    let mut x = [0u8; 32];
    x[0] = b;
    x
}

/// Model (a): the caller supplies the vault's live engine-tracked energy.
fn ctx(epoch_now: Epoch, contract_energy: Energy) -> PredicateContext {
    PredicateContext {
        epoch_now,
        contract_energy,
    }
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
    Predicate::EnergyDecaysBelow { threshold }
}

// EpochReached vaults ignore live energy; pass a stand-in to prove it.
const IGNORED: Energy = 1_000;

// =================================================================
// §8.1 — Adversary A: Present-Self Reneger
// =================================================================

#[test]
fn adversary_a_payout_before_epoch_trips_is_rejected() {
    let mut v = vault(Predicate::EpochReached { release_epoch: 100 }, 1_000);
    let err = payout(&mut v, 50, IGNORED).expect_err("predicate not yet satisfied");
    assert_eq!(err, PayoutError::PredicateNotSatisfied { epoch_now: 50 });
    assert!(
        v.is_locked(),
        "vault must remain Locked after failed payout"
    );
}

#[test]
fn adversary_a_payout_before_energy_decays_is_rejected() {
    // Live energy 1_000_000 ≥ threshold 500_000 ⇒ predicate false.
    let mut v = vault(energy_decays_predicate(500_000), 1_000);
    let err = payout(&mut v, 0, 1_000_000).expect_err("predicate not yet satisfied");
    assert_eq!(err, PayoutError::PredicateNotSatisfied { epoch_now: 0 });
    assert!(v.is_locked());
}

#[test]
fn adversary_a_repeated_premature_attempts_do_not_drain_or_corrupt() {
    let mut v = vault(
        Predicate::EpochReached {
            release_epoch: 1_000,
        },
        42,
    );
    for epoch in [0u64, 1, 50, 500, 999] {
        let err = payout(&mut v, epoch, IGNORED).expect_err("must remain unsatisfied");
        assert!(matches!(err, PayoutError::PredicateNotSatisfied { .. }));
    }
    assert_eq!(v.deposit, 42);
    assert!(v.is_locked());
    let r = payout(&mut v, 1_000, IGNORED).expect("trip epoch");
    assert_eq!(r.amount, 42);
}

#[test]
fn adversary_a_refresh_then_reclaim_is_rejected() {
    // NEW (model-(a)-only): live energy decayed below threshold, but the
    // reneger pays gas to refresh it back above before calling payout.
    // The predicate is refresh-aware ⇒ payout must be rejected. The old
    // frozen-formula predicate structurally could not express this.
    let mut v = vault(energy_decays_predicate(500), 1_000);
    // A refresh lifted live energy back to 900 ≥ 500 ⇒ payout rejected
    // even though epochs elapsed (refresh-aware; the old frozen-formula
    // predicate could not express this).
    let err = payout(&mut v, 2_100, 900).expect_err("refreshed ⇒ not satisfied");
    assert!(matches!(err, PayoutError::PredicateNotSatisfied { .. }));
    assert!(v.is_locked());
    // Later it decays again to 100 < 500 ⇒ legitimately releases.
    let r = payout(&mut v, 3_000, 100).expect("decayed again");
    assert_eq!(r.amount, 1_000);
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
    // deposit=1 is valid at the crate API — dust-spam mitigation is a
    // chain-level admission rule (§8.4), not this layer's job. Pin it.
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
    let first = payout(&mut v, 10, IGNORED).expect("first payout");
    assert_eq!(first.amount, 1_000);
    let err = payout(&mut v, 10, IGNORED).expect_err("second payout must fail");
    assert_eq!(err, PayoutError::AlreadyReleased);
    assert!(!v.is_locked(), "vault must remain Released, not relocked");
}

#[test]
fn adversary_e_replay_with_different_epoch_after_release_still_rejected() {
    let mut v = vault(Predicate::EpochReached { release_epoch: 10 }, 100);
    payout(&mut v, 10, IGNORED).expect("first payout");
    for replay_epoch in [11u64, 12, 100, u64::MAX] {
        let err = payout(&mut v, replay_epoch, IGNORED).expect_err("replay rejected");
        assert_eq!(err, PayoutError::AlreadyReleased);
    }
}

#[test]
fn adversary_e_replay_after_energy_release_still_rejected() {
    // Live energy already decayed below threshold ⇒ first payout
    // releases; any replay is terminal-rejected regardless of energy.
    let p = energy_decays_predicate(1);
    let mut v = vault(p, 100);
    payout(&mut v, 50, 0).expect("first payout (live energy 0 < 1)");
    let err = payout(&mut v, 51, 0).expect_err("replay rejected");
    assert_eq!(err, PayoutError::AlreadyReleased);
}

// =================================================================
// §8.6 — Adversary F: MEV / Transfer-Claim Race
// =================================================================

#[test]
fn adversary_f_non_holder_cannot_transfer_claim() {
    let mut v = vault(
        Predicate::EpochReached {
            release_epoch: 1_000,
        },
        1,
    );
    let mallory = addr(0x99);
    let bob = addr(0x42);
    let err = v
        .transfer_claim(mallory, bob)
        .expect_err("non-holder transfer rejected");
    assert!(matches!(err, VaultError::NotCurrentHolder { .. }));
    assert_eq!(v.current_holder(), Some(addr(0x02)));
}

#[test]
fn adversary_f_transfer_chain_preserves_holder_lineage() {
    let mut v = vault(
        Predicate::EpochReached {
            release_epoch: 1_000,
        },
        1,
    );
    let initial_holder = addr(0x02);
    let bob = addr(0x42);
    let carol = addr(0x43);

    v.transfer_claim(initial_holder, bob)
        .expect("initial → bob");
    assert_eq!(v.current_holder(), Some(bob));

    let stale = v
        .transfer_claim(initial_holder, carol)
        .expect_err("stale holder rejected");
    assert!(matches!(stale, VaultError::NotCurrentHolder { .. }));

    v.transfer_claim(bob, carol).expect("bob → carol");
    assert_eq!(v.current_holder(), Some(carol));

    let bob_stale = v
        .transfer_claim(bob, addr(0x44))
        .expect_err("bob is now stale");
    assert!(matches!(bob_stale, VaultError::NotCurrentHolder { .. }));
}

#[test]
fn adversary_f_transfer_after_release_is_rejected() {
    let mut v = vault(Predicate::EpochReached { release_epoch: 1 }, 1);
    payout(&mut v, 1, IGNORED).expect("payout");
    assert_eq!(v.current_holder(), None);
    let err = v
        .transfer_claim(addr(0x02), addr(0x99))
        .expect_err("transfer after release rejected");
    assert_eq!(err, VaultError::NotLocked);
}

#[test]
fn adversary_f_self_referential_transfer_succeeds_but_changes_nothing() {
    let mut v = vault(
        Predicate::EpochReached {
            release_epoch: 1_000,
        },
        1,
    );
    v.transfer_claim(addr(0x02), addr(0x02))
        .expect("self-transfer");
    assert_eq!(v.current_holder(), Some(addr(0x02)));
}

// §8.6 — on-chain listing state machine adversaries (decision A:
// the `.es` record_sale/listing guards are now crate-testable).

#[test]
fn adversary_f_cannot_list_someone_elses_claim() {
    let mut v = vault(
        Predicate::EpochReached {
            release_epoch: 1_000,
        },
        1,
    );
    // mallory (not holder) tries to open a listing on 0x02's vault.
    let err = v
        .list_for_sale(addr(0x99), 1_000, 100, 0, 50)
        .expect_err("non-holder list rejected");
    assert!(matches!(err, VaultError::NotCurrentHolder { .. }));
    assert!(!v.is_listed());
}

#[test]
fn adversary_f_cannot_record_sale_without_a_listing() {
    // Front-run: attacker calls record_sale hoping to grab the claim
    // before any listing exists. Must be rejected (no open listing).
    let mut v = vault(
        Predicate::EpochReached {
            release_epoch: 1_000,
        },
        1,
    );
    let err = v
        .record_sale(addr(0x99), 10)
        .expect_err("record_sale with no listing rejected");
    assert_eq!(err, VaultError::NotListed);
    assert_eq!(v.current_holder(), Some(addr(0x02)));
}

#[test]
fn adversary_f_cannot_record_sale_after_listing_expired() {
    // Holder lists; attacker waits past expiry then tries to settle a
    // stale listing to themselves. `.es` expiry guard must reject.
    let mut v = vault(
        Predicate::EpochReached {
            release_epoch: 10_000,
        },
        1,
    );
    v.list_for_sale(addr(0x02), 1_000, 100, 0, 100)
        .expect("holder lists");
    let err = v
        .record_sale(addr(0x99), 101) // 101 > opened_at(0)+duration(100)
        .expect_err("expired listing cannot be settled");
    assert_eq!(err, VaultError::ListingExpired);
    assert_eq!(v.current_holder(), Some(addr(0x02)), "claim not stolen");
}

#[test]
fn adversary_f_double_list_is_rejected() {
    let mut v = vault(
        Predicate::EpochReached {
            release_epoch: 1_000,
        },
        1,
    );
    v.list_for_sale(addr(0x02), 1_000, 100, 0, 50)
        .expect("first list");
    let err = v
        .list_for_sale(addr(0x02), 900, 90, 0, 50)
        .expect_err("second concurrent listing rejected");
    assert_eq!(err, VaultError::AlreadyListed);
}

// =================================================================
// §8.7 — Adversary G: Beneficiary-Key Loss (testable corollary)
// =================================================================

#[test]
fn adversary_g_lost_key_payout_credits_listed_holder_not_creator() {
    // Single-key v1.0: payout still goes to future_self/holder, never
    // silently redirected to creator. Pin against future refactors.
    let mut v = vault(Predicate::EpochReached { release_epoch: 1 }, 999);
    let result = payout(&mut v, 1, IGNORED).expect("payout");
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
    let c = ctx(2_048, 700_000);
    let first = evaluate(&p, c);
    for _ in 0..1_000 {
        assert_eq!(evaluate(&p, c), first);
    }
}

#[test]
fn predicate_evaluate_does_not_mutate_predicate() {
    let p = Predicate::EpochReached { release_epoch: 42 };
    for epoch in 0..200u64 {
        let _ = evaluate(&p, ctx(epoch, IGNORED));
    }
    assert_eq!(p, Predicate::EpochReached { release_epoch: 42 });
}

#[test]
fn predicate_evaluate_extreme_values_do_not_panic() {
    // Re-org / DoS safety: boundary inputs must never panic in debug.
    let cases = [
        (
            Predicate::EpochReached {
                release_epoch: u64::MAX,
            },
            0u64,
            0u64,
        ),
        (
            Predicate::EpochReached { release_epoch: 0 },
            u64::MAX,
            u64::MAX,
        ),
        (
            Predicate::EnergyDecaysBelow { threshold: 0 },
            u64::MAX,
            u64::MAX,
        ),
        (
            Predicate::EnergyDecaysBelow {
                threshold: u64::MAX,
            },
            u64::MAX,
            0,
        ),
    ];
    for (p, epoch_now, contract_energy) in cases {
        let _ = evaluate(&p, ctx(epoch_now, contract_energy));
    }
}

#[test]
fn vault_create_with_zero_predicate_threshold_is_unsatisfiable() {
    // Doctrine note preserved: threshold=0 ⇒ never trips (energy < 0
    // impossible for u64), even at fully-evaporated live energy 0.
    let p = energy_decays_predicate(0);
    let mut v = vault(p, 1_000);
    for epoch in [9_999u64, 99_999, u64::MAX / 2] {
        let err = payout(&mut v, epoch, 0).expect_err("never trips");
        assert!(matches!(err, PayoutError::PredicateNotSatisfied { .. }));
    }
    assert!(v.is_locked());
}

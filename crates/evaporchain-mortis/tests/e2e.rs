//! §A2.5 — Mortis: final-death act e2e — EvaporChain Testnet 0 shutdown arc
//!
//! Scenario: "EvaporChain Testnet 0" (ectn0) ran healthy for 10_000 epochs.
//! Its refresh pool gradually drained over 3 final epochs (floor=1_000,
//! sustained=3). Mortis fired at epoch 10_003. The singleton death-certificate
//! NFT was minted — unowned, immutable, verifiable forever.
//!
//! The suite proves: the latch is irreversible; partial breaches reset on
//! recovery; the witness cryptographically binds all certificate fields.

use evaporchain_mortis::{
    certificate::{verify_certificate, CertificateError},
    mint_certificate, MortisCondition, MortisMonitor, TickOutcome,
};

const FLOOR: u64 = 1_000;
const SUSTAINED: u64 = 3;
const STATE_ROOT: [u8; 32] = [0xAA; 32];
const EULOGY_ROOT: [u8; 32] = [0xBB; 32];

fn cond() -> MortisCondition {
    MortisCondition::new(FLOOR, SUSTAINED)
}
fn fresh() -> MortisMonitor {
    MortisMonitor::new(cond())
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn healthy_chain_never_triggers() {
    // Pool stays well above floor for 10_000 epochs — monitor stays Healthy.
    let mut m = fresh();
    for epoch in 1u64..=10_000 {
        let o = m.tick(epoch, FLOOR * 1_000);
        assert_eq!(o, TickOutcome::Healthy, "epoch {epoch} should be Healthy");
    }
    assert!(!m.is_triggered());
    assert_eq!(m.consecutive_below, 0);
}

#[test]
fn gradual_drain_counting_increments() {
    // Pool drops below floor for SUSTAINED-1 epochs → counter increments but no fire.
    let mut m = fresh();
    for k in 1..SUSTAINED {
        let o = m.tick(k, FLOOR / 2);
        assert_eq!(
            o,
            TickOutcome::Counting {
                consecutive_below: k
            },
            "tick {k} must be Counting at {k}"
        );
        assert!(!m.is_triggered());
    }
    assert_eq!(m.consecutive_below, SUSTAINED - 1);
}

#[test]
fn partial_drain_then_recovery_resets_counter() {
    // N-1 epochs below floor, then one healthy tick → consecutive_below resets to 0.
    let mut m = fresh();
    for k in 1..SUSTAINED {
        m.tick(k, FLOOR / 2);
    }
    assert_eq!(
        m.consecutive_below,
        SUSTAINED - 1,
        "pre-reset count must be N-1"
    );

    let o = m.tick(SUSTAINED, FLOOR * 10); // healthy recovery
    assert_eq!(o, TickOutcome::Healthy);
    assert_eq!(m.consecutive_below, 0, "recovery must reset counter to 0");
    assert!(!m.is_triggered());
}

#[test]
fn sustained_breach_fires_just_triggered() {
    // Exactly SUSTAINED consecutive low ticks → JustTriggered on the last.
    let mut m = fresh();
    for k in 1..SUSTAINED {
        m.tick(k, FLOOR / 2); // Counting
    }
    let o = m.tick(SUSTAINED, FLOOR / 2);
    assert_eq!(o, TickOutcome::JustTriggered, "sustained breach must fire");
    assert!(m.is_triggered());
}

#[test]
fn trigger_latches_subsequent_ticks_are_noop() {
    // After JustTriggered, every further tick returns AlreadyTriggered —
    // even if the pool floods back to astronomical levels.
    let mut m = fresh();
    for k in 1..=SUSTAINED {
        m.tick(k, 0);
    }
    assert!(m.is_triggered());

    for epoch in SUSTAINED + 1..=SUSTAINED + 20 {
        let o = m.tick(epoch, u64::MAX);
        assert_eq!(
            o,
            TickOutcome::AlreadyTriggered,
            "epoch {epoch}: latch must hold"
        );
        assert!(m.is_triggered());
    }
}

#[test]
fn pool_at_exactly_floor_counts_as_below() {
    // The condition is `pool > floor` (strict). Pool == floor is not above → Counting.
    let mut m = fresh();
    let o = m.tick(1, FLOOR);
    assert!(
        matches!(
            o,
            TickOutcome::Counting {
                consecutive_below: 1
            }
        ),
        "pool == floor must count, not be Healthy; got {:?}",
        o
    );
}

#[test]
fn partial_then_refill_then_second_full_run_fires() {
    // N-1 below → 1 healthy → N below → JustTriggered. Two separate runs.
    let mut m = fresh();
    // First partial run (N-1 ticks below)
    for k in 1..SUSTAINED {
        m.tick(k, 0);
    }
    m.tick(SUSTAINED, FLOOR * 100); // recovery resets

    // Second full run (N ticks below) must fire from scratch
    let base = SUSTAINED + 1;
    for k in 1..SUSTAINED {
        let o = m.tick(base + k - 1, 0);
        assert!(matches!(o, TickOutcome::Counting { .. }));
        assert!(!m.is_triggered());
    }
    let o = m.tick(base + SUSTAINED - 1, 0);
    assert_eq!(o, TickOutcome::JustTriggered);
    assert!(m.is_triggered());
}

#[test]
fn death_certificate_mint_verify_round_trip() {
    // After Mortis fires, the chain mints the singleton certificate.
    // verify_certificate must pass on the just-minted certificate.
    let epoch_of_death = 10_003u64;
    let final_pool = 42u64; // ≤ floor

    let cert = mint_certificate(STATE_ROOT, EULOGY_ROOT, epoch_of_death, final_pool);
    assert_eq!(cert.final_state_root, STATE_ROOT);
    assert_eq!(cert.eulogy_trie_root, EULOGY_ROOT);
    assert_eq!(cert.epoch_of_death, epoch_of_death);
    assert_eq!(cert.final_refresh_pool, final_pool);

    verify_certificate(&cert).expect("freshly minted certificate must verify");
}

#[test]
fn tampered_certificate_rejected_on_any_field() {
    // Any mutation to any field must break the witness.
    let base = mint_certificate(STATE_ROOT, EULOGY_ROOT, 10_003, 42);

    // Tamper final_state_root
    let mut c = base;
    c.final_state_root[0] ^= 0xFF;
    assert!(matches!(
        verify_certificate(&c),
        Err(CertificateError::WitnessMismatch { .. })
    ));

    // Tamper eulogy_trie_root
    let mut c = base;
    c.eulogy_trie_root[0] ^= 0xFF;
    assert!(matches!(
        verify_certificate(&c),
        Err(CertificateError::WitnessMismatch { .. })
    ));

    // Tamper epoch_of_death
    let mut c = base;
    c.epoch_of_death += 1;
    assert!(matches!(
        verify_certificate(&c),
        Err(CertificateError::WitnessMismatch { .. })
    ));

    // Tamper final_refresh_pool
    let mut c = base;
    c.final_refresh_pool += 1;
    assert!(matches!(
        verify_certificate(&c),
        Err(CertificateError::WitnessMismatch { .. })
    ));

    // Tamper witness directly
    let mut c = base;
    c.witness[0] ^= 0xFF;
    assert!(matches!(
        verify_certificate(&c),
        Err(CertificateError::WitnessMismatch { .. })
    ));
}

#[test]
fn certificate_witness_deterministic() {
    // Same inputs always produce the same certificate — validators agree.
    let a = mint_certificate(STATE_ROOT, EULOGY_ROOT, 10_003, 42);
    let b = mint_certificate(STATE_ROOT, EULOGY_ROOT, 10_003, 42);
    assert_eq!(a, b, "certificate must be deterministic");
}

#[test]
fn two_monitors_with_different_conditions_are_independent() {
    // Tight monitor (floor=10_000, sustained=1) fires immediately on breach.
    // Loose monitor (floor=100, sustained=10) needs 10 epochs below 100.
    let mut tight = MortisMonitor::new(MortisCondition::new(10_000, 1));
    let mut loose = MortisMonitor::new(MortisCondition::new(100, 10));

    // Pool at 5_000 → tight fires (5_000 < 10_000, and sustained=1); loose is healthy (5_000 > 100)
    let o_tight = tight.tick(1, 5_000);
    let o_loose = loose.tick(1, 5_000);
    assert_eq!(
        o_tight,
        TickOutcome::JustTriggered,
        "tight monitor fires immediately"
    );
    assert_eq!(
        o_loose,
        TickOutcome::Healthy,
        "loose monitor unaffected at 5_000"
    );

    // Tight is latched; loose is still counting at 0
    assert!(tight.is_triggered());
    assert!(!loose.is_triggered());
}

#[test]
fn ectn0_shutdown_doctrine_full_arc() {
    // EvaporChain Testnet 0 full lifecycle:
    // epochs 1..10_000 → healthy; epochs 10_001..10_003 → dying; epoch 10_003 → death.
    let mut m = MortisMonitor::new(MortisCondition::new(FLOOR, SUSTAINED));

    // Healthy era
    for ep in 1u64..=10_000 {
        let o = m.tick(ep, FLOOR * 100);
        assert_eq!(o, TickOutcome::Healthy);
    }
    assert!(!m.is_triggered());

    // Dying era: pool drains to 0 for SUSTAINED consecutive epochs
    for k in 1..SUSTAINED {
        let o = m.tick(10_000 + k, 0);
        assert!(matches!(o, TickOutcome::Counting { .. }));
    }
    let o = m.tick(10_000 + SUSTAINED, 0);
    assert_eq!(
        o,
        TickOutcome::JustTriggered,
        "ectn0 must fire at epoch 10_003"
    );

    // Mint and verify the death certificate
    let cert = mint_certificate(
        STATE_ROOT,
        EULOGY_ROOT,
        10_000 + SUSTAINED,
        0, // refresh pool exhausted
    );
    verify_certificate(&cert).expect("ectn0 death certificate must be valid");
    assert_eq!(cert.epoch_of_death, 10_000 + SUSTAINED);
    assert_eq!(cert.final_refresh_pool, 0);

    // The latch is permanent
    assert_eq!(m.tick(99_999, u64::MAX), TickOutcome::AlreadyTriggered);
}

//! Coverage tests for Mortis — chain's final-death act (per
//! `INVENTION_STACK.md Amendment 2 §A2.5`).

use evaporchain_mortis::certificate::{verify_certificate, CertificateError};
use evaporchain_mortis::{
    mint_certificate, MortisCertificate, MortisCondition, MortisMonitor, TickOutcome,
};

// =================================================================
// MortisCondition
// =================================================================

#[test]
fn condition_default_genesis_values() {
    let c = MortisCondition::default_genesis();
    assert_eq!(c.refresh_pool_floor, 1_000);
    assert_eq!(c.sustained_epochs, 4_096);
}

#[test]
fn condition_serde_round_trips() {
    let c = MortisCondition::new(5_000, 100);
    let json = serde_json::to_string(&c).unwrap();
    let back: MortisCondition = serde_json::from_str(&json).unwrap();
    assert_eq!(back, c);
}

// =================================================================
// MortisMonitor — TickOutcome variants
// =================================================================

#[test]
fn monitor_starts_healthy_on_above_floor() {
    let cond = MortisCondition::new(1_000, 3);
    let mut m = MortisMonitor::new(cond);
    let out = m.tick(1, 5_000);
    assert!(matches!(out, TickOutcome::Healthy));
    assert!(!m.is_triggered());
    assert_eq!(m.consecutive_below, 0);
}

#[test]
fn monitor_counting_outcome_carries_count() {
    let cond = MortisCondition::new(1_000, 5);
    let mut m = MortisMonitor::new(cond);
    let o1 = m.tick(1, 500);
    let o2 = m.tick(2, 500);
    let o3 = m.tick(3, 500);
    assert!(matches!(
        o1,
        TickOutcome::Counting {
            consecutive_below: 1
        }
    ));
    assert!(matches!(
        o2,
        TickOutcome::Counting {
            consecutive_below: 2
        }
    ));
    assert!(matches!(
        o3,
        TickOutcome::Counting {
            consecutive_below: 3
        }
    ));
}

#[test]
fn monitor_reset_on_recovery() {
    let cond = MortisCondition::new(1_000, 5);
    let mut m = MortisMonitor::new(cond);
    let _ = m.tick(1, 500);
    let _ = m.tick(2, 500);
    let out = m.tick(3, 9_999);
    assert!(matches!(out, TickOutcome::Healthy));
    assert_eq!(m.consecutive_below, 0, "recovery resets the counter");
}

#[test]
fn monitor_triggers_at_exact_threshold() {
    let cond = MortisCondition::new(1_000, 3);
    let mut m = MortisMonitor::new(cond);
    let _ = m.tick(1, 500);
    let _ = m.tick(2, 500);
    let out = m.tick(3, 500);
    assert!(matches!(out, TickOutcome::JustTriggered));
    assert!(m.is_triggered());
}

#[test]
fn monitor_latched_after_trigger() {
    let cond = MortisCondition::new(1_000, 2);
    let mut m = MortisMonitor::new(cond);
    let _ = m.tick(1, 500);
    let _ = m.tick(2, 500);
    let out = m.tick(3, 999_999_999);
    assert!(matches!(out, TickOutcome::AlreadyTriggered));
    assert!(
        m.is_triggered(),
        "trigger must remain latched even on full recovery"
    );
}

#[test]
fn monitor_treats_exact_floor_as_below() {
    // Pool == floor → below path (consecutive_below increments).
    let cond = MortisCondition::new(1_000, 5);
    let mut m = MortisMonitor::new(cond);
    let out = m.tick(1, 1_000);
    assert!(matches!(
        out,
        TickOutcome::Counting {
            consecutive_below: 1
        }
    ));
}

#[test]
fn monitor_past_tick_ignored() {
    let cond = MortisCondition::new(1_000, 5);
    let mut m = MortisMonitor::new(cond);
    let _ = m.tick(10, 5_000);
    let out = m.tick(5, 500); // past epoch — must be ignored
    assert!(matches!(out, TickOutcome::Healthy));
    assert_eq!(m.consecutive_below, 0);
    assert_eq!(m.latest_epoch, 10);
}

#[test]
fn monitor_serde_round_trips() {
    let cond = MortisCondition::new(1_000, 3);
    let mut m = MortisMonitor::new(cond);
    let _ = m.tick(1, 500);
    let _ = m.tick(2, 500);
    let json = serde_json::to_string(&m).unwrap();
    let back: MortisMonitor = serde_json::from_str(&json).unwrap();
    assert_eq!(back.consecutive_below, m.consecutive_below);
    assert_eq!(back.latest_epoch, m.latest_epoch);
    assert_eq!(back.triggered, m.triggered);
}

// =================================================================
// MortisCertificate — mint + verify
// =================================================================

#[test]
fn certificate_mint_and_verify_round_trip() {
    let cert = mint_certificate([1u8; 32], [2u8; 32], 12_345, 999);
    verify_certificate(&cert).expect("freshly-minted certificate must verify");
}

#[test]
fn certificate_tamper_state_root_rejected() {
    let mut cert = mint_certificate([1u8; 32], [2u8; 32], 100, 100);
    cert.final_state_root[0] ^= 0xff;
    let err = verify_certificate(&cert).expect_err("must reject");
    assert!(matches!(err, CertificateError::WitnessMismatch { .. }));
}

#[test]
fn certificate_tamper_eulogy_root_rejected() {
    let mut cert = mint_certificate([1u8; 32], [2u8; 32], 100, 100);
    cert.eulogy_trie_root[0] ^= 0xff;
    assert!(matches!(
        verify_certificate(&cert),
        Err(CertificateError::WitnessMismatch { .. })
    ));
}

#[test]
fn certificate_tamper_epoch_rejected() {
    let mut cert = mint_certificate([1u8; 32], [2u8; 32], 100, 100);
    cert.epoch_of_death += 1;
    assert!(matches!(
        verify_certificate(&cert),
        Err(CertificateError::WitnessMismatch { .. })
    ));
}

#[test]
fn certificate_tamper_refresh_pool_rejected() {
    let mut cert = mint_certificate([1u8; 32], [2u8; 32], 100, 100);
    cert.final_refresh_pool += 1;
    assert!(matches!(
        verify_certificate(&cert),
        Err(CertificateError::WitnessMismatch { .. })
    ));
}

#[test]
fn certificate_serde_round_trips() {
    let cert = mint_certificate([7u8; 32], [8u8; 32], 42, 99);
    let json = serde_json::to_string(&cert).unwrap();
    let back: MortisCertificate = serde_json::from_str(&json).unwrap();
    assert_eq!(back, cert);
    verify_certificate(&back).expect("must verify after serde");
}

#[test]
fn certificate_error_displays() {
    let e = CertificateError::WitnessMismatch {
        derived: [0u8; 32],
        claimed: [1u8; 32],
    };
    assert!(e.to_string().to_lowercase().contains("mismatch"));
}

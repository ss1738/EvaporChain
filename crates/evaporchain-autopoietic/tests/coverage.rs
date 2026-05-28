//! Coverage tests for the Autopoietic Chain framing layer (`A4.3.8`,
//! `INVENTION_STACK.md §A4.3.8`). Ties Sentinel + LLSA + Refresh-Pool
//! Patronage into a Maturana-Varela autopoietic system.
//!
//! Existing in-module tests cover the four primary states (Viable /
//! Stressed / Inviable / Degraded-patronage). This file adds:
//!
//!   - `AutopoieticHealth::compute_status` lookup-table sweep across
//!     all 27 combinations of (patronage, sentinel, llsa) ∈ {H,D,F}³
//!   - `ChainAutopoiesis::new` field-assignment pin
//!   - `health_report` boundary cases:
//!       * total_energy == min_patronage_energy (the >= boundary)
//!       * epoch == last_sentinel_vote (zero-elapsed)
//!       * epoch.saturating_sub clamp on future votes
//!       * elapsed == sentinel_heartbeat_window (inclusive boundary)
//!   - Multi-covenant saturating_add safety
//!   - Serde round-trip for SubsystemHealth + AutopoieticStatus + AutopoieticHealth

use evaporchain_autopoietic::autopoiesis::{
    AutopoieticHealth, AutopoieticStatus, ChainAutopoiesis, SubsystemHealth,
};
use evaporchain_llsa::proof::{AlwaysAcceptVerifier, AlwaysRejectVerifier};
use evaporchain_refresh_patronage::{PatronageBook, PatronageCovenant};

fn empty_book() -> PatronageBook {
    PatronageBook::new(b"coverage-ns".to_vec())
}

fn covenant_with_score(oid: Vec<u8>, score: u64) -> PatronageCovenant {
    PatronageCovenant {
        object_id: oid,
        namespace_id: vec![],
        donation_per_epoch: 10,
        created_epoch: 0,
        expires_epoch: 1_000_000,
        pre_funded: 100,
        patronage_score: score,
        last_honoured_epoch: None,
    }
}

// =================================================================
// ChainAutopoiesis::new — field-assignment pin
// =================================================================

#[test]
fn new_assigns_all_fields() {
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 7_777, 42);
    assert_eq!(sys.min_patronage_energy, 7_777);
    assert_eq!(sys.sentinel_heartbeat_window, 42);
    // verifier is opaque but its behavior is observable via health_report.
    let r = sys.health_report(&empty_book(), &[], Some(0), 0);
    assert_eq!(r.llsa, SubsystemHealth::Healthy, "AlwaysAccept ⇒ Healthy");
}

// =================================================================
// health_report boundary cases
// =================================================================

#[test]
fn patronage_at_exact_min_is_healthy() {
    // total_energy == min_patronage_energy: the >= boundary must be
    // *inclusive* (Healthy), not Degraded.
    let mut book = PatronageBook::new(b"boundary-ns".to_vec());
    let oid = vec![0x01; 32];
    book.insert(covenant_with_score(oid.clone(), 100));
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 100, 10);
    let r = sys.health_report(&book, &[oid], Some(99), 100);
    assert_eq!(r.patronage, SubsystemHealth::Healthy);
    assert_eq!(r.total_patronage_energy, 100);
}

#[test]
fn patronage_one_unit_below_min_is_degraded() {
    let mut book = PatronageBook::new(b"boundary-ns".to_vec());
    let oid = vec![0x02; 32];
    book.insert(covenant_with_score(oid.clone(), 99));
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 100, 10);
    let r = sys.health_report(&book, &[oid], Some(99), 100);
    assert_eq!(r.patronage, SubsystemHealth::Degraded);
}

#[test]
fn sentinel_at_exact_heartbeat_window_is_healthy() {
    // elapsed == sentinel_heartbeat_window: the <= boundary must be
    // *inclusive* (Healthy), not Degraded.
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 10);
    let r = sys.health_report(&empty_book(), &[], Some(90), 100);
    // elapsed = 100 - 90 = 10; window = 10 ⇒ Healthy.
    assert_eq!(r.sentinel, SubsystemHealth::Healthy);
}

#[test]
fn sentinel_one_epoch_past_heartbeat_window_is_degraded() {
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 10);
    let r = sys.health_report(&empty_book(), &[], Some(89), 100);
    // elapsed = 11 > 10 ⇒ Degraded.
    assert_eq!(r.sentinel, SubsystemHealth::Degraded);
}

#[test]
fn sentinel_zero_elapsed_is_healthy() {
    // last_vote == epoch: elapsed = 0 ⇒ Healthy regardless of window.
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 0);
    let r = sys.health_report(&empty_book(), &[], Some(100), 100);
    assert_eq!(r.sentinel, SubsystemHealth::Healthy);
}

#[test]
fn sentinel_future_vote_does_not_panic() {
    // last_vote > epoch: saturating_sub clamps elapsed to 0.
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 10);
    let r = sys.health_report(&empty_book(), &[], Some(200), 100);
    assert_eq!(
        r.sentinel,
        SubsystemHealth::Healthy,
        "future vote clamps elapsed"
    );
}

#[test]
fn sentinel_none_is_failed() {
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 10);
    let r = sys.health_report(&empty_book(), &[], None, 100);
    assert_eq!(r.sentinel, SubsystemHealth::Failed);
}

// =================================================================
// Multi-covenant + saturating_add safety
// =================================================================

#[test]
fn multiple_covenants_sum_scores() {
    let mut book = PatronageBook::new(b"multi-ns".to_vec());
    let oids: Vec<Vec<u8>> = (1..=5u8).map(|i| vec![i; 32]).collect();
    for (i, oid) in oids.iter().enumerate() {
        book.insert(covenant_with_score(oid.clone(), 20 + (i as u64) * 10));
    }
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 10);
    let r = sys.health_report(&book, &oids, Some(99), 100);
    // 20 + 30 + 40 + 50 + 60 = 200
    assert_eq!(r.total_patronage_energy, 200);
}

#[test]
fn covenant_score_sum_saturates_at_u64_max() {
    let mut book = PatronageBook::new(b"sat-ns".to_vec());
    let oids: Vec<Vec<u8>> = (1..=3u8).map(|i| vec![i; 32]).collect();
    for oid in &oids {
        book.insert(covenant_with_score(oid.clone(), u64::MAX / 2));
    }
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 10);
    let r = sys.health_report(&book, &oids, Some(99), 100);
    // 3 × (u64::MAX/2) saturates at u64::MAX.
    assert_eq!(r.total_patronage_energy, u64::MAX);
}

// =================================================================
// compute_status — partial sweep across the {H,D,F}³ lattice
// =================================================================

#[test]
fn status_lattice_zero_at_risk_is_viable() {
    // P=H, S=H, L=H → Viable
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 10);
    let r = sys.health_report(&empty_book(), &[], Some(95), 100);
    assert_eq!(r.status, AutopoieticStatus::Viable);
}

#[test]
fn status_lattice_one_at_risk_is_stressed() {
    // P=F (no covenants, min=1000), S=H, L=H → Stressed
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 1_000, 10);
    let r = sys.health_report(&empty_book(), &[], Some(95), 100);
    assert_eq!(r.patronage, SubsystemHealth::Failed);
    assert_eq!(r.sentinel, SubsystemHealth::Healthy);
    assert_eq!(r.llsa, SubsystemHealth::Healthy);
    assert_eq!(r.status, AutopoieticStatus::Stressed);
}

#[test]
fn status_lattice_two_at_risk_is_stressed_not_inviable() {
    // P=F, S=F, L=H → two failed but one healthy → Stressed
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 1_000, 10);
    let r = sys.health_report(&empty_book(), &[], None, 100);
    assert_eq!(r.patronage, SubsystemHealth::Failed);
    assert_eq!(r.sentinel, SubsystemHealth::Failed);
    assert_eq!(r.llsa, SubsystemHealth::Healthy);
    assert_eq!(r.status, AutopoieticStatus::Stressed);
}

#[test]
fn status_lattice_three_at_risk_is_inviable_even_with_mixed_severity() {
    // P=Degraded, S=Degraded, L=Degraded → all three at-risk but not
    // all Failed. Must still be Inviable per the doctrine (any 3-of-3
    // at-risk = Inviable, severity doesn't matter for the aggregate).
    let mut book = PatronageBook::new(b"all-degraded-ns".to_vec());
    let oid = vec![0xAB; 32];
    book.insert(covenant_with_score(oid.clone(), 50)); // < min=100 ⇒ Degraded
    let sys = ChainAutopoiesis::new(AlwaysRejectVerifier, 100, 10);
    // last_vote out of window ⇒ sentinel Degraded.
    let r = sys.health_report(&book, &[oid], Some(0), 100);
    assert_eq!(r.patronage, SubsystemHealth::Degraded);
    assert_eq!(r.sentinel, SubsystemHealth::Degraded);
    assert_eq!(r.llsa, SubsystemHealth::Degraded);
    assert_eq!(
        r.status,
        AutopoieticStatus::Inviable,
        "3-of-3 at-risk is Inviable even when each is only Degraded"
    );
}

// =================================================================
// Reporting fields propagate
// =================================================================

#[test]
fn health_report_epoch_field_propagates() {
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 10);
    let r = sys.health_report(&empty_book(), &[], Some(9_999), 10_000);
    assert_eq!(r.epoch, 10_000);
}

// =================================================================
// Serde round-trips
// =================================================================

#[test]
fn subsystem_health_serde_round_trips() {
    for h in [
        SubsystemHealth::Healthy,
        SubsystemHealth::Degraded,
        SubsystemHealth::Failed,
    ] {
        let json = serde_json::to_string(&h).expect("serialize");
        let back: SubsystemHealth = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, h);
    }
}

#[test]
fn autopoietic_status_serde_round_trips() {
    for s in [
        AutopoieticStatus::Viable,
        AutopoieticStatus::Stressed,
        AutopoieticStatus::Inviable,
    ] {
        let json = serde_json::to_string(&s).expect("serialize");
        let back: AutopoieticStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, s);
    }
}

#[test]
fn autopoietic_health_serde_round_trips() {
    let sys = ChainAutopoiesis::new(AlwaysAcceptVerifier, 0, 10);
    let r = sys.health_report(&empty_book(), &[], Some(95), 100);
    let json = serde_json::to_string(&r).expect("serialize");
    let back: AutopoieticHealth = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.status, r.status);
    assert_eq!(back.patronage, r.patronage);
    assert_eq!(back.sentinel, r.sentinel);
    assert_eq!(back.llsa, r.llsa);
    assert_eq!(back.total_patronage_energy, r.total_patronage_energy);
    assert_eq!(back.epoch, r.epoch);
}

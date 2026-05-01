//! End-to-end tests for the four-act narrative spine wired into
//! `SimpleExecutor`.
//!
//! These tests prove the chain integration of:
//! - **Small deaths**: tombstone insertion when `collect_storage_rent`
//!   zeros an account.
//! - **Final death**: `tick_mortis` firing the certificate when the
//!   refresh pool stays below ε for N epochs.
//! - **Conservation loop**: storage rent flowing into the
//!   protocol-owned `refresh_pool`.
//! - **Singh-Lyapunov fee state**: per-block tick + drift report.

#![cfg(test)]

use crate::SimpleExecutor;
use crate::SYSTEM_REFRESH_NAMESPACE;
use evaporchain_mortis::{MortisCondition, MortisMonitor};
use evaporchain_state::db::InMemoryStateDB;
use evaporchain_state::db::StateDB;
use evaporchain_types::AccountAddress;

fn addr(b: u8) -> AccountAddress {
    [b; 32]
}

#[test]
fn collect_storage_rent_partial_debit_accrues_into_refresh_pool() {
    let mut exec = SimpleExecutor::new(0);
    let mut db = InMemoryStateDB::new();
    {
        let a = db.get_or_create_account(&addr(1));
        a.balance = 1_000_000;
        a.storage_bytes = 100;
    }
    let pre_pool = exec.refresh_pool.total_accrued();
    // Drive collect_storage_rent through the public path —
    // SimpleExecutor::collect_storage_rent is private; exercise via
    // the per-block tick so we go through the gating logic.
    // Simplest exposure: snapshot db state before / after by calling
    // a tiny helper that mirrors what apply_block does.
    exec.run_storage_rent_for_test(&mut db, 5);
    // Account still has positive balance (rent < balance).
    let new_balance = db.get_account(&addr(1)).unwrap().balance;
    assert!(new_balance < 1_000_000);
    let debited = 1_000_000 - new_balance;
    // Refresh pool accrued exactly the debited amount.
    let post_pool = exec.refresh_pool.total_accrued();
    assert_eq!(post_pool - pre_pool, debited);
}

#[test]
fn account_zeroed_by_rent_is_tombstoned_and_pool_takes_residual() {
    let mut exec = SimpleExecutor::new(0);
    let mut db = InMemoryStateDB::new();
    {
        let a = db.get_or_create_account(&addr(2));
        a.balance = 5; // small balance, large storage → rent > balance
        a.storage_bytes = 1_000_000;
    }
    exec.run_storage_rent_for_test(&mut db, 7);
    // Account zeroed.
    let acct = db.get_account(&addr(2)).unwrap();
    assert_eq!(acct.balance, 0);
    assert_eq!(acct.storage_bytes, 0);
    // Tombstone present in the eulogy trie.
    assert!(exec.eulogy_trie.contains(&addr(2)));
    // Refresh pool got the residual 5 units.
    assert_eq!(exec.refresh_pool.total_accrued(), 5);
}

#[test]
fn mortis_fires_when_pool_stays_below_floor_for_sustained_epochs() {
    let mut exec = SimpleExecutor::new(0);
    // Tighten Mortis condition for the test: floor=10, sustained=3.
    exec.mortis_monitor = MortisMonitor::new(MortisCondition::new(10, 3));
    // Refresh pool starts empty (0), so every tick is below floor=10.
    let s_root = [0xFEu8; 32];
    assert!(exec.tick_mortis(1, s_root).is_none());
    assert!(exec.tick_mortis(2, s_root).is_none());
    let cert = *exec
        .tick_mortis(3, s_root)
        .expect("Mortis must fire on 3rd consecutive tick");
    assert_eq!(cert.epoch_of_death, 3);
    assert_eq!(cert.final_refresh_pool, 0);
    assert_eq!(cert.final_state_root, s_root);
    // Subsequent ticks are no-ops (latched).
    assert!(exec.tick_mortis(4, s_root).is_none());
    // Certificate is preserved.
    assert!(exec.mortis_certificate.is_some());
}

#[test]
fn lyapunov_fee_tick_at_equilibrium_is_a_fixed_point() {
    let mut exec = SimpleExecutor::new(0);
    let target_gas = exec.lyapunov_fee_params.target_gas;
    let target_e = exec.lyapunov_fee_params.target_energy;
    let (_fee, drift) = exec.tick_lyapunov_fee_state(target_gas, 1).unwrap();
    assert_eq!(exec.lyapunov_fee_state.energy, target_e);
    assert_eq!(drift.delta, 0);
}

#[test]
fn lyapunov_fee_tick_above_equilibrium_decays() {
    let mut exec = SimpleExecutor::new(0);
    let target_e = exec.lyapunov_fee_params.target_energy;
    exec.lyapunov_fee_state = evaporchain_fee_controller::FeeState::new(target_e + 100_000);
    let target_gas = exec.lyapunov_fee_params.target_gas;
    let (_, drift) = exec.tick_lyapunov_fee_state(target_gas, 1).unwrap();
    assert!(exec.lyapunov_fee_state.energy < target_e + 100_000);
    assert!(drift.delta <= 0, "empty-block drift must be non-positive");
}

#[test]
fn refresh_pool_namespace_is_well_known() {
    // Sanity: the system namespace is the expected byte string.
    assert_eq!(SYSTEM_REFRESH_NAMESPACE, b"evaporchain-system-refresh");
}

#[test]
fn cmu_observation_within_bound_records_ok() {
    let mut exec = SimpleExecutor::new(0);
    let v = exec.record_cmu_observation(100, 100, 200);
    assert!(matches!(v, evaporchain_cmu_gate::Verdict::Ok { .. }));
    assert!(exec.last_cmu_verdict.is_some());
}

#[test]
fn cmu_observation_above_bound_records_violation() {
    let mut exec = SimpleExecutor::new(0);
    let v = exec.record_cmu_observation(500, 100, 200);
    assert!(matches!(v, evaporchain_cmu_gate::Verdict::Violation { .. }));
}

#[test]
fn tur_observation_constants_record_violation() {
    let mut exec = SimpleExecutor::new(0);
    let v = exec.record_tur_observation(&[10, 10, 10, 10, 10], 100);
    assert!(matches!(
        v,
        evaporchain_tur_liveness::Verdict::Violation { .. }
    ));
    assert!(exec.last_tur_verdict.is_some());
}

#[test]
fn tur_observation_high_variance_records_ok() {
    let mut exec = SimpleExecutor::new(0);
    let v = exec.record_tur_observation(&[1, 100, 1, 100, 1, 100], 100);
    assert!(matches!(v, evaporchain_tur_liveness::Verdict::Ok { .. }));
}

#[test]
fn full_death_flow_zero_account_then_mortis_fires() {
    let mut exec = SimpleExecutor::new(0);
    // Tight Mortis so 3 ticks past trigger it.
    exec.mortis_monitor = MortisMonitor::new(MortisCondition::new(1_000, 3));
    let mut db = InMemoryStateDB::new();
    // Pre-fund an account just enough that one rent tick will zero it.
    {
        let a = db.get_or_create_account(&addr(9));
        a.balance = 50;
        a.storage_bytes = 100_000_000;
    }
    exec.run_storage_rent_for_test(&mut db, 1);
    // Account is dead, tombstoned, and pool got the residual 50.
    assert!(exec.eulogy_trie.contains(&addr(9)));
    assert_eq!(exec.refresh_pool.total_accrued(), 50);
    // Pool=50 < floor=1000 → mortis ticks consecutive_below.
    let s_root = [0u8; 32];
    assert!(exec.tick_mortis(2, s_root).is_none());
    assert!(exec.tick_mortis(3, s_root).is_none());
    let cert = exec.tick_mortis(4, s_root);
    assert!(
        cert.is_some(),
        "after 3 sustained ticks below floor, Mortis fires"
    );
}

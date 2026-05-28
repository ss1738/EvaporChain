//! §State layer e2e
//!
//! Scenario: "LENA validator state transition arc" — LENA runs a
//! validator node. She initialises a fresh state DB, registers accounts
//! for three participants, verifies state-root determinism and
//! commutativity, detects a tampered balance via root change, double-spend
//! nullifiers via the one-shot gate, and processes one epoch of energy
//! decay through the EvaporationEngine (Active → Grace → Ghost).
//!
//! The suite proves: `compute_state_root` is pure and commutative;
//! balance changes produce a different root; `spend_nullifier` is a
//! one-shot gate; the EvaporationEngine correctly transitions objects;
//! decay curves (Exponential, Linear, Stepped) yield the right values.

use evaporchain_state::decay_curves::compute_energy;
use evaporchain_state::{EvaporationEngine, InMemoryStateDB, StateDB};
use evaporchain_types::{Account, AccountAddress, DecayCurve, ObjectState, StateObject};

fn addr(b: u8) -> AccountAddress {
    [b; 32]
}

fn account(address: AccountAddress, balance: u64) -> Account {
    Account {
        address,
        balance,
        nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        vesting: None,
    }
}

fn object(id: [u8; 32], energy: u64, state: ObjectState) -> StateObject {
    StateObject {
        id,
        owner: addr(0xAA),
        energy,
        half_life: 100,
        created_at: 0,
        last_refreshed: 0,
        state,
        grace_epoch: None,
        data: vec![],
        decay_curve: None,
        lad_mode: None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn fresh_db_has_no_accounts() {
    let db = InMemoryStateDB::new();
    assert!(db.get_account(&addr(0x01)).is_none());
}

#[test]
fn put_then_get_account_round_trips() {
    let mut db = InMemoryStateDB::new();
    db.put_account(account(addr(0x01), 1_000));
    let got = db.get_account(&addr(0x01)).unwrap();
    assert_eq!(got.balance, 1_000);
}

#[test]
fn state_root_is_deterministic() {
    let mut db = InMemoryStateDB::new();
    db.put_account(account(addr(0x01), 100));
    db.put_account(account(addr(0x02), 200));
    let r1 = db.compute_state_root();
    let r2 = db.compute_state_root();
    assert_eq!(r1, r2, "compute_state_root must be deterministic");
}

#[test]
fn state_root_is_commutative() {
    let mut db1 = InMemoryStateDB::new();
    db1.put_account(account(addr(0x01), 100));
    db1.put_account(account(addr(0x02), 200));

    let mut db2 = InMemoryStateDB::new();
    db2.put_account(account(addr(0x02), 200));
    db2.put_account(account(addr(0x01), 100));

    assert_eq!(
        db1.compute_state_root(),
        db2.compute_state_root(),
        "insertion order must not affect state root"
    );
}

#[test]
fn balance_change_changes_state_root() {
    let mut db = InMemoryStateDB::new();
    db.put_account(account(addr(0x01), 100));
    let root_before = db.compute_state_root();

    db.put_account(account(addr(0x01), 999));
    let root_after = db.compute_state_root();

    assert_ne!(
        root_before, root_after,
        "balance change must produce different state root"
    );
}

#[test]
fn adding_account_changes_state_root() {
    let mut db = InMemoryStateDB::new();
    db.put_account(account(addr(0x01), 100));
    let r1 = db.compute_state_root();

    db.put_account(account(addr(0x02), 200));
    let r2 = db.compute_state_root();

    assert_ne!(r1, r2, "adding an account must change the state root");
}

#[test]
fn nullifier_spend_is_one_shot() {
    let mut db = InMemoryStateDB::new();
    let n = [0xBB; 32];

    assert!(
        !db.is_nullifier_spent(&n),
        "fresh nullifier must not be spent"
    );
    assert!(db.spend_nullifier(&n), "first spend must succeed");
    assert!(!db.spend_nullifier(&n), "double-spend must be rejected");
    assert!(db.is_nullifier_spent(&n));
}

#[test]
fn different_nullifiers_are_independent() {
    let mut db = InMemoryStateDB::new();
    let n1 = [0x01; 32];
    let n2 = [0x02; 32];

    db.spend_nullifier(&n1);
    assert!(db.is_nullifier_spent(&n1));
    assert!(!db.is_nullifier_spent(&n2));
}

#[test]
fn evaporation_engine_active_to_grace() {
    let engine = EvaporationEngine::new(10);
    let mut db = InMemoryStateDB::new();

    let id = [0x10; 32];
    let mut obj = object(id, 0, ObjectState::Active);
    obj.energy = 0;
    db.put_object(obj);

    let result = engine.process_epoch(&mut db, 1);
    assert_eq!(
        result.entered_grace.len(),
        1,
        "zero-energy object must enter grace"
    );
    assert_eq!(result.entered_grace[0], id);
}

#[test]
fn evaporation_engine_grace_to_ghost() {
    let engine = EvaporationEngine::new(0); // zero grace → immediate evaporation
    let mut db = InMemoryStateDB::new();

    let id = [0x20; 32];
    let mut obj = object(id, 0, ObjectState::Grace);
    obj.grace_epoch = Some(0);
    db.put_object(obj);

    let result = engine.process_epoch(&mut db, 1);
    assert_eq!(
        result.evaporated.len(),
        1,
        "grace-expired object must evaporate"
    );
    assert_eq!(result.evaporated[0], id);
}

#[test]
fn decay_curve_exponential() {
    let curve = DecayCurve::Exponential { half_life: 100 };
    let after_100 = compute_energy(&curve, 1_000_000, 100, None);
    assert!(
        after_100 >= 495_000 && after_100 <= 505_000,
        "exponential after 1 half-life: expected ~500_000, got {after_100}"
    );
}

#[test]
fn decay_curve_linear_reaches_zero() {
    let curve = DecayCurve::Linear { rate_per_epoch: 10 };
    assert_eq!(compute_energy(&curve, 1_000, 100, None), 0);
    assert_eq!(compute_energy(&curve, 1_000, 50, None), 500);
}

#[test]
fn decay_curve_stepped() {
    let curve = DecayCurve::Stepped {
        thresholds: vec![(10, 500), (20, 100)],
    };
    assert_eq!(compute_energy(&curve, 1_000, 5, None), 1_000);
    assert_eq!(compute_energy(&curve, 1_000, 15, None), 500);
    assert_eq!(compute_energy(&curve, 1_000, 25, None), 100);
}

#[test]
fn decay_zero_elapsed_returns_initial() {
    let curve = DecayCurve::Exponential { half_life: 100 };
    assert_eq!(compute_energy(&curve, 5_000, 0, None), 5_000);
}

#[test]
fn lena_state_transition_full_arc() {
    let mut db = InMemoryStateDB::new();

    db.put_account(account(addr(0xA0), 10_000));
    db.put_account(account(addr(0xB0), 20_000));
    db.put_account(account(addr(0xC0), 5_000));
    let root_initial = db.compute_state_root();

    // Commutativity: different insertion order → same root.
    let mut db2 = InMemoryStateDB::new();
    db2.put_account(account(addr(0xC0), 5_000));
    db2.put_account(account(addr(0xA0), 10_000));
    db2.put_account(account(addr(0xB0), 20_000));
    assert_eq!(root_initial, db2.compute_state_root());

    // Tamper detection.
    db2.put_account(account(addr(0xA0), 99_999));
    assert_ne!(root_initial, db2.compute_state_root());

    // Nullifier double-spend gate.
    let n = [0xAB; 32];
    assert!(db.spend_nullifier(&n));
    assert!(!db.spend_nullifier(&n));

    // Evaporation: grace-period-zero object evaporates.
    let engine = EvaporationEngine::new(0);
    let mut obj = object([0xDE; 32], 0, ObjectState::Grace);
    obj.grace_epoch = Some(0);
    db.put_object(obj);
    let result = engine.process_epoch(&mut db, 1);
    assert!(!result.evaporated.is_empty());
}

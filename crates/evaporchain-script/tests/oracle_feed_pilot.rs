//! Pilot — drive `contracts/evaporscript/oracle_feed.es` through the
//! full parse → compile → VM execution pipeline.
//!
//! Fourth worked-example behavioural pilot for the seed-12 stdlib. Where
//! standard oracles publish data with a timestamp and let consumers
//! decide freshness, OracleFeed makes the feed itself a decaying
//! contract — stale data physically can't exist on-chain after
//! evaporation. This pilot pins the doctrine: latest() reverts before
//! any value, latest() returns the correct value after update, and the
//! is_fresh / age telemetry tracks correctly.
//!
//! Pins the documented invariants:
//!   1. set_feed is deployer-only and one-shot.
//!   2. update is deployer-only; updates increment update_count and
//!      stamp updated_at_epoch.
//!   3. latest() reverts before value_set, returns the latest value
//!      after.
//!   4. age() returns 0 pre-set, then `current_epoch -
//!      updated_at_epoch` after each update.
//!   5. is_fresh() compares age vs max_age; flips false past max_age.
//!   6. dispute is open and bumps the counter; doesn't mutate value.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/oracle_feed.es");

fn ctx(caller: [u8; 32], owner: [u8; 32], epoch: u64, energy: u64) -> ExecutionContext {
    ExecutionContext {
        caller,
        owner,
        epoch,
        energy,
        vrf_randomness: [0u8; 32],
        call_depth: 0,
    }
}

fn compile_pilot() -> EvaporBytecode {
    let ast = parser::parse(SOURCE).unwrap_or_else(|e| panic!("OracleFeed failed to parse: {e:?}"));
    compiler::compile(&ast).unwrap_or_else(|e| panic!("OracleFeed failed to compile: {e:?}"))
}

fn initial_state(bc: &EvaporBytecode) -> HashMap<String, Value> {
    let mut state = HashMap::new();
    for f in &bc.state_schema.fields {
        if let Some(default) = &f.default {
            state.insert(f.name.clone(), default.clone());
        }
    }
    state
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "OracleFeed");
    let public = [
        "set_feed",
        "update",
        "latest",
        "age",
        "dispute",
        "feed_label",
        "updates_total",
        "disputes_total",
        "last_updated",
        "is_fresh",
    ];
    for m in &public {
        assert!(
            bc.methods.contains_key(*m),
            "method `{m}` missing from compiled bytecode"
        );
    }
    for hook in ["on_grace", "on_refresh", "on_evaporate"] {
        assert!(
            bc.methods.contains_key(hook),
            "lifecycle hook `{hook}` missing"
        );
    }
}

#[test]
fn set_feed_seals_metadata_and_blocks_double_set() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let r = EvaporVM::execute(
        &bc,
        "set_feed",
        vec![Value::Str("BTC-USD".to_string()), Value::U64(100)],
        initial_state(&bc),
        &ctx(operator, operator, 100, 10_000),
    )
    .expect("set_feed must succeed");

    let label = EvaporVM::execute(
        &bc,
        "feed_label",
        vec![],
        r.state_changes.clone(),
        &ctx(operator, operator, 101, 10_000),
    )
    .unwrap();
    assert_eq!(label.return_value, Value::Str("BTC-USD".to_string()));

    let err = EvaporVM::execute(
        &bc,
        "set_feed",
        vec![Value::Str("ETH-USD".to_string()), Value::U64(50)],
        r.state_changes,
        &ctx(operator, operator, 102, 10_000),
    )
    .expect_err("re-set_feed must reject");
    assert!(
        format!("{err:?}").contains("already configured"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_operator_set_feed_rejects() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let attacker = [0xCCu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "set_feed",
        vec![Value::Str("X".to_string()), Value::U64(100)],
        initial_state(&bc),
        &ctx(attacker, operator, 100, 10_000),
    )
    .expect_err("non-operator set_feed must reject");
    assert!(
        format!("{err:?}").contains("only operator"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn latest_before_any_update_reverts() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let configured = EvaporVM::execute(
        &bc,
        "set_feed",
        vec![Value::Str("BTC-USD".to_string()), Value::U64(100)],
        initial_state(&bc),
        &ctx(operator, operator, 100, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "latest",
        vec![],
        configured.state_changes,
        &ctx(operator, operator, 101, 10_000),
    )
    .expect_err("latest pre-update must revert");
    assert!(
        format!("{err:?}").contains("no value published yet"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn update_then_latest_round_trip() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let configured = EvaporVM::execute(
        &bc,
        "set_feed",
        vec![Value::Str("BTC-USD".to_string()), Value::U64(100)],
        initial_state(&bc),
        &ctx(operator, operator, 100, 10_000),
    )
    .unwrap();

    let r1 = EvaporVM::execute(
        &bc,
        "update",
        vec![Value::U64(67_500)],
        configured.state_changes,
        &ctx(operator, operator, 200, 10_000),
    )
    .expect("update must succeed");

    let v = EvaporVM::execute(
        &bc,
        "latest",
        vec![],
        r1.state_changes.clone(),
        &ctx(operator, operator, 201, 10_000),
    )
    .unwrap();
    assert_eq!(v.return_value, Value::U64(67_500));

    let n = EvaporVM::execute(
        &bc,
        "updates_total",
        vec![],
        r1.state_changes.clone(),
        &ctx(operator, operator, 202, 10_000),
    )
    .unwrap();
    assert_eq!(n.return_value, Value::U64(1));

    let last = EvaporVM::execute(
        &bc,
        "last_updated",
        vec![],
        r1.state_changes,
        &ctx(operator, operator, 203, 10_000),
    )
    .unwrap();
    assert_eq!(last.return_value, Value::U64(200));
}

#[test]
fn non_operator_update_rejects() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let attacker = [0xCCu8; 32];
    let configured = EvaporVM::execute(
        &bc,
        "set_feed",
        vec![Value::Str("X".to_string()), Value::U64(100)],
        initial_state(&bc),
        &ctx(operator, operator, 100, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "update",
        vec![Value::U64(42)],
        configured.state_changes,
        &ctx(attacker, operator, 200, 10_000),
    )
    .expect_err("non-operator update must reject");
    assert!(
        format!("{err:?}").contains("only operator"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn age_tracks_epoch_delta_after_each_update() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let configured = EvaporVM::execute(
        &bc,
        "set_feed",
        vec![Value::Str("X".to_string()), Value::U64(100)],
        initial_state(&bc),
        &ctx(operator, operator, 100, 10_000),
    )
    .unwrap();

    // Pre-update: age == 0 (no value_set yet).
    let age0 = EvaporVM::execute(
        &bc,
        "age",
        vec![],
        configured.state_changes.clone(),
        &ctx(operator, operator, 150, 10_000),
    )
    .unwrap();
    assert_eq!(age0.return_value, Value::U64(0));

    // After update at epoch 200, age at epoch 250 = 50.
    let r1 = EvaporVM::execute(
        &bc,
        "update",
        vec![Value::U64(1)],
        configured.state_changes,
        &ctx(operator, operator, 200, 10_000),
    )
    .unwrap();
    let age1 = EvaporVM::execute(
        &bc,
        "age",
        vec![],
        r1.state_changes,
        &ctx(operator, operator, 250, 10_000),
    )
    .unwrap();
    assert_eq!(age1.return_value, Value::U64(50));
}

#[test]
fn is_fresh_compares_against_max_age() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    // max_age = 100 epochs.
    let configured = EvaporVM::execute(
        &bc,
        "set_feed",
        vec![Value::Str("X".to_string()), Value::U64(100)],
        initial_state(&bc),
        &ctx(operator, operator, 100, 10_000),
    )
    .unwrap();

    // Pre-update: not fresh.
    let pre = EvaporVM::execute(
        &bc,
        "is_fresh",
        vec![],
        configured.state_changes.clone(),
        &ctx(operator, operator, 150, 10_000),
    )
    .unwrap();
    assert_eq!(pre.return_value, Value::Bool(false));

    // After update at 200: fresh at 250 (age=50<=100), stale at 350 (age=150>100).
    let r1 = EvaporVM::execute(
        &bc,
        "update",
        vec![Value::U64(1)],
        configured.state_changes,
        &ctx(operator, operator, 200, 10_000),
    )
    .unwrap();
    let fresh = EvaporVM::execute(
        &bc,
        "is_fresh",
        vec![],
        r1.state_changes.clone(),
        &ctx(operator, operator, 250, 10_000),
    )
    .unwrap();
    assert_eq!(fresh.return_value, Value::Bool(true));

    let stale = EvaporVM::execute(
        &bc,
        "is_fresh",
        vec![],
        r1.state_changes,
        &ctx(operator, operator, 350, 10_000),
    )
    .unwrap();
    assert_eq!(stale.return_value, Value::Bool(false));
}

#[test]
fn dispute_increments_counter_open_to_anyone() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let observer1 = [0xB1u8; 32];
    let observer2 = [0xB2u8; 32];
    let configured = EvaporVM::execute(
        &bc,
        "set_feed",
        vec![Value::Str("X".to_string()), Value::U64(100)],
        initial_state(&bc),
        &ctx(operator, operator, 100, 10_000),
    )
    .unwrap();
    let updated = EvaporVM::execute(
        &bc,
        "update",
        vec![Value::U64(42)],
        configured.state_changes,
        &ctx(operator, operator, 200, 10_000),
    )
    .unwrap();

    let d1 = EvaporVM::execute(
        &bc,
        "dispute",
        vec![],
        updated.state_changes,
        &ctx(observer1, operator, 210, 10_000),
    )
    .expect("observer can dispute");
    let d2 = EvaporVM::execute(
        &bc,
        "dispute",
        vec![],
        d1.state_changes,
        &ctx(observer2, operator, 211, 10_000),
    )
    .expect("second observer can dispute");

    let total = EvaporVM::execute(
        &bc,
        "disputes_total",
        vec![],
        d2.state_changes,
        &ctx(operator, operator, 212, 10_000),
    )
    .unwrap();
    assert_eq!(total.return_value, Value::U64(2));
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let configured = EvaporVM::execute(
        &bc,
        "set_feed",
        vec![Value::Str("X".to_string()), Value::U64(100)],
        initial_state(&bc),
        &ctx(operator, operator, 100, 10_000),
    )
    .unwrap();
    for hook in &["on_grace", "on_refresh", "on_evaporate"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            configured.state_changes.clone(),
            &ctx(operator, operator, 500, 100),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        let _ = r.events;
    }
}

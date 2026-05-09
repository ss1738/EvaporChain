//! Pilot — drive `contracts/evaporscript/dead_man_switch.es` through the
//! full parse → compile → VM execution pipeline.
//!
//! Acts as the worked-example behavioural test for the seed-12 stdlib.
//! The other 11 contracts get their own pilots modelled on this file
//! incrementally.
//!
//! Pins the documented invariants:
//!   1. `set_switch` is one-shot and principal-only (caller == builtin
//!      owner). First check-in is implicit in the arm.
//!   2. `check_in` is principal-only and requires `sealed && !disarmed
//!      && !released`. Each call bumps `checkin_count` and refreshes
//!      `last_checkin_epoch`.
//!   3. `claim` reverts before release; only the beneficiary can call;
//!      double-claim reverts.
//!   4. `on_evaporate` releases the switch IFF not disarmed.
//!   5. `disarm` blocks subsequent release; on_evaporate after disarm
//!      is a no-op.
//!   6. Lifecycle hooks emit cleanly under all reachable arm states.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/dead_man_switch.es");

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
    let ast = parser::parse(SOURCE)
        .unwrap_or_else(|e| panic!("DeadManSwitch failed to parse: {e:?}"));
    compiler::compile(&ast)
        .unwrap_or_else(|e| panic!("DeadManSwitch failed to compile: {e:?}"))
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
    assert_eq!(bc.name, "DeadManSwitch");
    let public = [
        "set_switch",
        "check_in",
        "disarm",
        "claim",
        "principal_of",
        "beneficiary_of",
        "last_checkin",
        "checkins_total",
        "is_released",
        "is_claimed",
        "is_disarmed",
        "silence_age",
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
fn arm_seals_and_records_first_checkin() {
    let bc = compile_pilot();
    let principal = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];

    let armed = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(beneficiary),
            Value::Str("encrypted-payload-blob".to_string()),
        ],
        initial_state(&bc),
        &ctx(principal, principal, 100, 10_000),
    )
    .expect("arm must succeed for principal");
    assert!(
        armed.events.iter().any(|e| e.contains("armed")),
        "arm must emit armed event"
    );

    let checkins = EvaporVM::execute(
        &bc,
        "checkins_total",
        vec![],
        armed.state_changes.clone(),
        &ctx(principal, principal, 101, 9_900),
    )
    .unwrap();
    assert_eq!(
        checkins.return_value,
        Value::U64(1),
        "first check-in is implicit in arm"
    );

    let last = EvaporVM::execute(
        &bc,
        "last_checkin",
        vec![],
        armed.state_changes,
        &ctx(principal, principal, 102, 9_900),
    )
    .unwrap();
    assert_eq!(
        last.return_value,
        Value::U64(100),
        "last_checkin must equal arm-time epoch"
    );
}

#[test]
fn non_principal_cannot_arm() {
    let bc = compile_pilot();
    let principal = [0xAAu8; 32];
    let attacker = [0xCCu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(attacker),
            Value::Str("hostile-payload".to_string()),
        ],
        initial_state(&bc),
        &ctx(attacker, principal, 100, 10_000),
    )
    .expect_err("non-principal arm must revert");
    assert!(
        format!("{err:?}").contains("only principal"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn double_arm_reverts() {
    let bc = compile_pilot();
    let principal = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(beneficiary),
            Value::Str("p".to_string()),
        ],
        initial_state(&bc),
        &ctx(principal, principal, 100, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(beneficiary),
            Value::Str("p2".to_string()),
        ],
        armed.state_changes,
        &ctx(principal, principal, 200, 9_500),
    )
    .expect_err("re-arm must revert");
    assert!(
        format!("{err:?}").contains("already armed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn check_in_bumps_count_and_epoch() {
    let bc = compile_pilot();
    let principal = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(beneficiary),
            Value::Str("p".to_string()),
        ],
        initial_state(&bc),
        &ctx(principal, principal, 100, 10_000),
    )
    .unwrap();

    let after_checkin = EvaporVM::execute(
        &bc,
        "check_in",
        vec![],
        armed.state_changes,
        &ctx(principal, principal, 250, 9_000),
    )
    .expect("principal check-in must succeed");

    let count = EvaporVM::execute(
        &bc,
        "checkins_total",
        vec![],
        after_checkin.state_changes.clone(),
        &ctx(principal, principal, 251, 9_000),
    )
    .unwrap();
    assert_eq!(
        count.return_value,
        Value::U64(2),
        "checkin_count must increment past the implicit-first"
    );

    let last = EvaporVM::execute(
        &bc,
        "last_checkin",
        vec![],
        after_checkin.state_changes,
        &ctx(principal, principal, 252, 9_000),
    )
    .unwrap();
    assert_eq!(
        last.return_value,
        Value::U64(250),
        "last_checkin must update to most recent"
    );
}

#[test]
fn non_principal_cannot_check_in() {
    let bc = compile_pilot();
    let principal = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let attacker = [0xCCu8; 32];
    let armed = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(beneficiary),
            Value::Str("p".to_string()),
        ],
        initial_state(&bc),
        &ctx(principal, principal, 100, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "check_in",
        vec![],
        armed.state_changes,
        &ctx(attacker, principal, 200, 9_500),
    )
    .expect_err("non-principal check-in must revert");
    assert!(
        format!("{err:?}").contains("only principal"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn claim_before_release_reverts() {
    let bc = compile_pilot();
    let principal = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(beneficiary),
            Value::Str("p".to_string()),
        ],
        initial_state(&bc),
        &ctx(principal, principal, 100, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        armed.state_changes,
        &ctx(beneficiary, principal, 200, 9_000),
    )
    .expect_err("claim before release must revert");
    assert!(
        format!("{err:?}").contains("not yet released"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn on_evaporate_releases_payload_when_armed() {
    let bc = compile_pilot();
    let principal = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(beneficiary),
            Value::Str("the-secret".to_string()),
        ],
        initial_state(&bc),
        &ctx(principal, principal, 100, 10_000),
    )
    .unwrap();

    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        armed.state_changes,
        &ctx(principal, principal, 5_000, 0),
    )
    .expect("on_evaporate must execute");

    let released = EvaporVM::execute(
        &bc,
        "is_released",
        vec![],
        evap.state_changes.clone(),
        &ctx(principal, principal, 5_001, 0),
    )
    .unwrap();
    assert_eq!(
        released.return_value,
        Value::Bool(true),
        "on_evaporate must flip released to true on armed switch"
    );

    let payload = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        evap.state_changes,
        &ctx(beneficiary, principal, 5_002, 0),
    )
    .expect("beneficiary must be able to claim post-release");
    assert_eq!(
        payload.return_value,
        Value::Str("the-secret".to_string()),
        "claim must return the released payload"
    );
}

#[test]
fn disarm_blocks_release_on_evaporate() {
    let bc = compile_pilot();
    let principal = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(beneficiary),
            Value::Str("the-secret".to_string()),
        ],
        initial_state(&bc),
        &ctx(principal, principal, 100, 10_000),
    )
    .unwrap();

    let disarmed = EvaporVM::execute(
        &bc,
        "disarm",
        vec![],
        armed.state_changes,
        &ctx(principal, principal, 200, 9_500),
    )
    .expect("principal disarm must succeed");

    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        disarmed.state_changes,
        &ctx(principal, principal, 5_000, 0),
    )
    .expect("on_evaporate must execute even when disarmed");

    let released = EvaporVM::execute(
        &bc,
        "is_released",
        vec![],
        evap.state_changes.clone(),
        &ctx(principal, principal, 5_001, 0),
    )
    .unwrap();
    assert_eq!(
        released.return_value,
        Value::Bool(false),
        "disarmed switch must NOT release on evaporate"
    );

    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        evap.state_changes,
        &ctx(beneficiary, principal, 5_002, 0),
    )
    .expect_err("claim against disarmed-then-evaporated switch must revert");
    assert!(
        format!("{err:?}").contains("not yet released"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn only_beneficiary_can_claim() {
    let bc = compile_pilot();
    let principal = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let attacker = [0xCCu8; 32];
    let armed = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(beneficiary),
            Value::Str("p".to_string()),
        ],
        initial_state(&bc),
        &ctx(principal, principal, 100, 10_000),
    )
    .unwrap();
    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        armed.state_changes,
        &ctx(principal, principal, 5_000, 0),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        evap.state_changes.clone(),
        &ctx(attacker, principal, 5_001, 0),
    )
    .expect_err("non-beneficiary claim must revert");
    assert!(
        format!("{err:?}").contains("only beneficiary"),
        "wrong revert: {err:?}"
    );

    // The principal also cannot claim — this is the beneficiary's right.
    let err2 = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        evap.state_changes,
        &ctx(principal, principal, 5_001, 0),
    )
    .expect_err("principal claim must revert");
    assert!(
        format!("{err2:?}").contains("only beneficiary"),
        "wrong revert: {err2:?}"
    );
}

#[test]
fn double_claim_reverts() {
    let bc = compile_pilot();
    let principal = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(beneficiary),
            Value::Str("p".to_string()),
        ],
        initial_state(&bc),
        &ctx(principal, principal, 100, 10_000),
    )
    .unwrap();
    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        armed.state_changes,
        &ctx(principal, principal, 5_000, 0),
    )
    .unwrap();
    let claimed = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        evap.state_changes,
        &ctx(beneficiary, principal, 5_001, 0),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        claimed.state_changes,
        &ctx(beneficiary, principal, 5_002, 0),
    )
    .expect_err("double claim must revert");
    assert!(
        format!("{err:?}").contains("already claimed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn lifecycle_hooks_emit_cleanly() {
    let bc = compile_pilot();
    let principal = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(beneficiary),
            Value::Str("p".to_string()),
        ],
        initial_state(&bc),
        &ctx(principal, principal, 100, 10_000),
    )
    .unwrap();

    // on_grace + on_refresh on an armed-not-yet-released switch.
    for hook in &["on_grace", "on_refresh"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            armed.state_changes.clone(),
            &ctx(principal, principal, 500, 100),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        // Hooks may emit conditionally — just verify the call path is clean.
        let _ = r.events;
    }
}

#[test]
fn silence_age_tracks_epoch_delta() {
    let bc = compile_pilot();
    let principal = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = EvaporVM::execute(
        &bc,
        "set_switch",
        vec![
            Value::Address(beneficiary),
            Value::Str("p".to_string()),
        ],
        initial_state(&bc),
        &ctx(principal, principal, 100, 10_000),
    )
    .unwrap();
    // 850 epochs after arm, no further check-ins.
    let age = EvaporVM::execute(
        &bc,
        "silence_age",
        vec![],
        armed.state_changes,
        &ctx(beneficiary, principal, 950, 1_000),
    )
    .unwrap();
    assert_eq!(
        age.return_value,
        Value::U64(850),
        "silence_age must equal current_epoch - last_checkin_epoch"
    );
}

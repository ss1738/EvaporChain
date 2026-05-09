//! Pilot — drive `contracts/evaporscript/vesting_schedule.es` through the
//! full parse → compile → VM execution pipeline.
//!
//! Seventh worked-example behavioural pilot. VestingSchedule's doctrine
//! moment: vested-but-unclaimed amount forfeits at evaporation. The
//! cliff + linear vest math runs normally; the *post-vest claim window*
//! is bounded by the contract's own energy.
//!
//! Pins:
//!   1. set_terms is one-shot + grantor-only; cliff <= duration enforced;
//!      grant > 0 + duration > 0.
//!   2. vested_now: 0 pre-cliff, total_grant post-duration, linear in
//!      between. Pre-seal returns 0.
//!   3. claim is beneficiary-only; updates claimed_amount monotonically;
//!      returns delta.
//!   4. cancel is grantor-only; allowed only if claimed_amount == 0.
//!   5. on_evaporate stamps vested_at_evaporate + flips forfeit_signaled.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/vesting_schedule.es");

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
        .unwrap_or_else(|e| panic!("VestingSchedule failed to parse: {e:?}"));
    compiler::compile(&ast)
        .unwrap_or_else(|e| panic!("VestingSchedule failed to compile: {e:?}"))
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

fn arm(
    bc: &EvaporBytecode,
    grantor: [u8; 32],
    beneficiary: [u8; 32],
    grant: u64,
    cliff: u64,
    duration: u64,
    seal_epoch: u64,
) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "set_terms",
        vec![
            Value::Address(beneficiary),
            Value::U64(grant),
            Value::U64(cliff),
            Value::U64(duration),
        ],
        initial_state(bc),
        &ctx(grantor, grantor, seal_epoch, 10_000),
    )
    .expect("set_terms must succeed");
    r.state_changes
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "VestingSchedule");
    let public = [
        "set_terms",
        "claim",
        "vested_now",
        "cancel",
        "vested_amount",
        "pending_amount",
        "beneficiary_of",
        "grant_total",
        "cliff_at",
        "fully_vested_at",
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
fn set_terms_validates_inputs() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];

    // grant == 0 rejects.
    let err = EvaporVM::execute(
        &bc,
        "set_terms",
        vec![
            Value::Address(beneficiary),
            Value::U64(0),
            Value::U64(10),
            Value::U64(100),
        ],
        initial_state(&bc),
        &ctx(grantor, grantor, 100, 10_000),
    )
    .expect_err("zero grant must reject");
    assert!(
        format!("{err:?}").contains("must be positive"),
        "wrong revert: {err:?}"
    );

    // duration == 0 rejects.
    let err = EvaporVM::execute(
        &bc,
        "set_terms",
        vec![
            Value::Address(beneficiary),
            Value::U64(1000),
            Value::U64(10),
            Value::U64(0),
        ],
        initial_state(&bc),
        &ctx(grantor, grantor, 100, 10_000),
    )
    .expect_err("zero duration must reject");
    assert!(
        format!("{err:?}").contains("duration must be positive"),
        "wrong revert: {err:?}"
    );

    // cliff > duration rejects.
    let err = EvaporVM::execute(
        &bc,
        "set_terms",
        vec![
            Value::Address(beneficiary),
            Value::U64(1000),
            Value::U64(200),
            Value::U64(100),
        ],
        initial_state(&bc),
        &ctx(grantor, grantor, 100, 10_000),
    )
    .expect_err("cliff > duration must reject");
    assert!(
        format!("{err:?}").contains("cliff cannot exceed duration"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn vesting_math_pre_cliff_post_cliff_post_full() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    // Grant 1000, cliff 100, duration 1000. Sealed at epoch 100.
    let armed = arm(&bc, grantor, beneficiary, 1000, 100, 1000, 100);

    // Pre-cliff: epoch 150 → elapsed 50 < cliff 100 → 0.
    let pre = EvaporVM::execute(
        &bc,
        "vested_now",
        vec![],
        armed.clone(),
        &ctx(beneficiary, grantor, 150, 10_000),
    )
    .unwrap();
    assert_eq!(pre.return_value, Value::U64(0));

    // Post-cliff: epoch 600 → elapsed 500 → 1000 * 500 / 1000 = 500.
    let mid = EvaporVM::execute(
        &bc,
        "vested_now",
        vec![],
        armed.clone(),
        &ctx(beneficiary, grantor, 600, 10_000),
    )
    .unwrap();
    assert_eq!(mid.return_value, Value::U64(500));

    // Post-duration: epoch 2000 → elapsed 1900 >= 1000 → total_grant.
    let full = EvaporVM::execute(
        &bc,
        "vested_now",
        vec![],
        armed,
        &ctx(beneficiary, grantor, 2000, 10_000),
    )
    .unwrap();
    assert_eq!(full.return_value, Value::U64(1000));
}

#[test]
fn claim_returns_only_delta_post_first_claim() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 0, 1000, 100);

    // Half-way through vesting: epoch 600 → 500 vested.
    let claim1 = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        armed,
        &ctx(beneficiary, grantor, 600, 10_000),
    )
    .expect("first claim must succeed");
    assert_eq!(claim1.return_value, Value::U64(500));

    // Later: epoch 800 → 700 vested cumulatively. Already claimed 500 → delta 200.
    let claim2 = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        claim1.state_changes,
        &ctx(beneficiary, grantor, 800, 10_000),
    )
    .expect("second claim must succeed");
    assert_eq!(claim2.return_value, Value::U64(200));

    // Same epoch re-claim: nothing to claim.
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        claim2.state_changes,
        &ctx(beneficiary, grantor, 800, 10_000),
    )
    .expect_err("re-claim same epoch must revert");
    assert!(
        format!("{err:?}").contains("nothing to claim"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_beneficiary_claim_rejects() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let attacker = [0xCCu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 0, 1000, 100);
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        armed,
        &ctx(attacker, grantor, 600, 10_000),
    )
    .expect_err("non-beneficiary claim must reject");
    assert!(
        format!("{err:?}").contains("only beneficiary"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn pre_cliff_claim_reverts() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 200, 1000, 100);
    // epoch 150 → elapsed 50 < cliff 200 → vested_now = 0 → claim reverts.
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        armed,
        &ctx(beneficiary, grantor, 150, 10_000),
    )
    .expect_err("pre-cliff claim must reject");
    assert!(
        format!("{err:?}").contains("nothing to claim"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn cancel_pre_claim_succeeds() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 0, 1000, 100);
    let cancelled = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        armed,
        &ctx(grantor, grantor, 200, 10_000),
    )
    .expect("cancel pre-claim must succeed");
    // Subsequent claim must reject.
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        cancelled.state_changes,
        &ctx(beneficiary, grantor, 600, 10_000),
    )
    .expect_err("post-cancel claim must reject");
    assert!(
        format!("{err:?}").contains("cancelled"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn cancel_post_claim_rejects_irrevocably_vested() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 0, 1000, 100);
    let claimed = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        armed,
        &ctx(beneficiary, grantor, 500, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        claimed.state_changes,
        &ctx(grantor, grantor, 600, 10_000),
    )
    .expect_err("post-claim cancel must reject");
    assert!(
        format!("{err:?}").contains("immutable"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_grantor_cancel_rejects() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 0, 1000, 100);
    let err = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        armed,
        &ctx(beneficiary, grantor, 200, 10_000),
    )
    .expect_err("beneficiary cancel must reject");
    assert!(
        format!("{err:?}").contains("only grantor"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn on_evaporate_stamps_forfeit() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 0, 1000, 100);
    // 600 vested at epoch 700, none claimed.
    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        armed,
        &ctx(grantor, grantor, 700, 0),
    )
    .expect("on_evaporate must execute");

    if let Some(Value::U64(v)) = evap.state_changes.get("vested_at_evaporate") {
        assert_eq!(
            *v, 600,
            "vested_at_evaporate must capture vested_now() at death"
        );
    } else {
        panic!("vested_at_evaporate not set as U64");
    }
    if let Some(Value::Bool(f)) = evap.state_changes.get("forfeit_signaled") {
        assert!(*f, "forfeit_signaled must flip true");
    }
}

#[test]
fn cliff_at_and_fully_vested_at_views() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 50, 500, 100);

    let cliff = EvaporVM::execute(
        &bc,
        "cliff_at",
        vec![],
        armed.clone(),
        &ctx(beneficiary, grantor, 101, 10_000),
    )
    .unwrap();
    // start_epoch 100 + cliff_epochs 50 = 150.
    assert_eq!(cliff.return_value, Value::U64(150));

    let full = EvaporVM::execute(
        &bc,
        "fully_vested_at",
        vec![],
        armed,
        &ctx(beneficiary, grantor, 102, 10_000),
    )
    .unwrap();
    // start_epoch 100 + duration_epochs 500 = 600.
    assert_eq!(full.return_value, Value::U64(600));
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 50, 500, 100);
    for hook in &["on_grace", "on_refresh"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            armed.clone(),
            &ctx(grantor, grantor, 500, 100),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        let _ = r.events;
    }
}

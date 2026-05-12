//! Pilot — drive `contracts/evaporscript/time_lock.es` through the full
//! parse → compile → VM execution pipeline.
//!
//! Eighth worked-example behavioural pilot. TimeLock keeps the
//! unlock_epoch (calendar deadline) but bounds the *claim window* by
//! the contract's energy lifetime — no second timer, no off-chain
//! reaper. An unclaimed lock at evaporation time forfeits.
//!
//! Pins:
//!   1. set_terms is one-shot + grantor-only; unlock must be in the
//!      future at setup; amount > 0.
//!   2. claim is beneficiary-only + requires epoch >= unlock_epoch +
//!      not revoked + not yet claimed.
//!   3. revoke is grantor-only + pre-unlock only; blocked post-claim.
//!   4. is_unlocked / is_claimed views reflect state correctly.
//!   5. on_evaporate flips forfeit_signaled iff !claimed && !revoked.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/time_lock.es");

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
        .unwrap_or_else(|e| panic!("TimeLock failed to parse: {e:?}"));
    compiler::compile(&ast)
        .unwrap_or_else(|e| panic!("TimeLock failed to compile: {e:?}"))
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
    amount: u64,
    unlock: u64,
    seal_epoch: u64,
) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "set_terms",
        vec![
            Value::Address(beneficiary),
            Value::U64(amount),
            Value::U64(unlock),
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
    assert_eq!(bc.name, "TimeLock");
    let public = [
        "set_terms",
        "claim",
        "revoke",
        "beneficiary_of",
        "locked",
        "unlock_at",
        "is_unlocked",
        "is_claimed",
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
fn set_terms_unlock_must_be_future() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    // unlock <= current epoch must reject.
    let err = EvaporVM::execute(
        &bc,
        "set_terms",
        vec![
            Value::Address(beneficiary),
            Value::U64(1000),
            Value::U64(100),
        ],
        initial_state(&bc),
        &ctx(grantor, grantor, 100, 10_000),
    )
    .expect_err("unlock at-or-before now must reject");
    assert!(
        format!("{err:?}").contains("unlock must be in the future"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn set_terms_zero_amount_rejects() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "set_terms",
        vec![Value::Address(beneficiary), Value::U64(0), Value::U64(500)],
        initial_state(&bc),
        &ctx(grantor, grantor, 100, 10_000),
    )
    .expect_err("zero amount must reject");
    assert!(
        format!("{err:?}").contains("must be positive"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn double_set_terms_rejects() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 500, 100);
    let err = EvaporVM::execute(
        &bc,
        "set_terms",
        vec![Value::Address(beneficiary), Value::U64(2000), Value::U64(800)],
        armed,
        &ctx(grantor, grantor, 200, 10_000),
    )
    .expect_err("re-set_terms must reject");
    assert!(
        format!("{err:?}").contains("already set"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn claim_pre_unlock_rejects() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 500, 100);
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        armed,
        &ctx(beneficiary, grantor, 400, 10_000),
    )
    .expect_err("pre-unlock claim must reject");
    assert!(
        format!("{err:?}").contains("still locked"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn claim_at_unlock_succeeds() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 500, 100);
    // Exactly at unlock_epoch — boundary case.
    let r = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        armed,
        &ctx(beneficiary, grantor, 500, 10_000),
    )
    .expect("claim at unlock_epoch must succeed");
    assert_eq!(r.return_value, Value::U64(1000));
}

#[test]
fn claim_post_unlock_succeeds_and_blocks_double_claim() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 500, 100);
    let claimed = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        armed,
        &ctx(beneficiary, grantor, 600, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        claimed.state_changes,
        &ctx(beneficiary, grantor, 700, 10_000),
    )
    .expect_err("double claim must reject");
    assert!(
        format!("{err:?}").contains("already claimed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_beneficiary_claim_rejects() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let attacker = [0xCCu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 500, 100);
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
fn revoke_pre_unlock_blocks_subsequent_claim() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 500, 100);
    let revoked = EvaporVM::execute(
        &bc,
        "revoke",
        vec![],
        armed,
        &ctx(grantor, grantor, 200, 10_000),
    )
    .expect("pre-unlock revoke must succeed");
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        revoked.state_changes,
        &ctx(beneficiary, grantor, 600, 10_000),
    )
    .expect_err("post-revoke claim must reject");
    assert!(
        format!("{err:?}").contains("revoked"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn revoke_post_unlock_rejects_irrevocable_after_maturity() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 500, 100);
    let err = EvaporVM::execute(
        &bc,
        "revoke",
        vec![],
        armed,
        &ctx(grantor, grantor, 600, 10_000),
    )
    .expect_err("post-unlock revoke must reject");
    assert!(
        format!("{err:?}").contains("cannot revoke after unlock"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_grantor_revoke_rejects() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 500, 100);
    let err = EvaporVM::execute(
        &bc,
        "revoke",
        vec![],
        armed,
        &ctx(beneficiary, grantor, 200, 10_000),
    )
    .expect_err("beneficiary revoke must reject");
    assert!(
        format!("{err:?}").contains("only grantor"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn views_reflect_state() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 500, 100);

    let bof = EvaporVM::execute(
        &bc,
        "beneficiary_of",
        vec![],
        armed.clone(),
        &ctx(grantor, grantor, 101, 10_000),
    )
    .unwrap();
    assert_eq!(bof.return_value, Value::Address(beneficiary));

    let unlock_at = EvaporVM::execute(
        &bc,
        "unlock_at",
        vec![],
        armed.clone(),
        &ctx(grantor, grantor, 102, 10_000),
    )
    .unwrap();
    assert_eq!(unlock_at.return_value, Value::U64(500));

    // Pre-unlock: locked = amount, is_unlocked = false.
    let locked = EvaporVM::execute(
        &bc,
        "locked",
        vec![],
        armed.clone(),
        &ctx(grantor, grantor, 200, 10_000),
    )
    .unwrap();
    assert_eq!(locked.return_value, Value::U64(1000));

    let unlocked = EvaporVM::execute(
        &bc,
        "is_unlocked",
        vec![],
        armed.clone(),
        &ctx(grantor, grantor, 201, 10_000),
    )
    .unwrap();
    assert_eq!(unlocked.return_value, Value::Bool(false));

    // Post-unlock epoch: is_unlocked = true.
    let unlocked2 = EvaporVM::execute(
        &bc,
        "is_unlocked",
        vec![],
        armed,
        &ctx(grantor, grantor, 600, 10_000),
    )
    .unwrap();
    assert_eq!(unlocked2.return_value, Value::Bool(true));
}

#[test]
fn on_evaporate_unclaimed_active_lock_signals_forfeit() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 500, 100);
    // Evaporate post-unlock, never claimed.
    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        armed,
        &ctx(grantor, grantor, 9_000, 0),
    )
    .expect("on_evaporate must execute");
    if let Some(Value::Bool(f)) = evap.state_changes.get("forfeit_signaled") {
        assert!(*f, "unclaimed active lock must signal forfeit");
    } else {
        panic!("forfeit_signaled not set");
    }
    if let Some(Value::U64(v)) = evap.state_changes.get("unclaimed_at_evaporate") {
        assert_eq!(*v, 1000, "unclaimed_at_evaporate must capture full amount");
    }
}

#[test]
fn on_evaporate_post_claim_does_not_resignal_forfeit() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 500, 100);
    let claimed = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        armed,
        &ctx(beneficiary, grantor, 600, 10_000),
    )
    .unwrap();
    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        claimed.state_changes,
        &ctx(grantor, grantor, 9_000, 0),
    )
    .expect("on_evaporate post-claim must execute");
    if let Some(Value::Bool(f)) = evap.state_changes.get("forfeit_signaled") {
        assert!(
            !*f,
            "claimed lock must NOT signal forfeit on later evaporation"
        );
    }
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let grantor = [0xAAu8; 32];
    let beneficiary = [0xBBu8; 32];
    let armed = arm(&bc, grantor, beneficiary, 1000, 500, 100);
    for hook in &["on_grace", "on_refresh"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            armed.clone(),
            &ctx(grantor, grantor, 300, 100),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        let _ = r.events;
    }
}

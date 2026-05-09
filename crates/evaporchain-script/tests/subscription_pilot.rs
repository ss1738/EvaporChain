//! Pilot — drive `contracts/evaporscript/subscription.es` through the
//! full parse → compile → VM execution pipeline.
//!
//! Fifth worked-example behavioural pilot for the seed-12 stdlib.
//! Subscription's doctrine moment: every other chain needs an off-chain
//! reaper to detect non-payment and cancel. EvaporChain doesn't —
//! pay() refreshes the contract, skipping payments lets it evaporate,
//! on_evaporate flips lapsed=true. No reaper, no who-watches-the-watcher.
//!
//! Pins the documented invariants:
//!   1. set_terms is one-shot and subscriber-only (caller==owner).
//!   2. pay is subscriber-only and requires sealed && !cancelled.
//!   3. paid_periods + cumulative_paid + last_payment_epoch track per pay.
//!   4. cancel can be called by either subscriber OR provider; blocks
//!      future pay calls; recorded with cancelled_by + epoch.
//!   5. on_evaporate flips lapsed=true ONLY if not cancelled.
//!   6. is_active reflects sealed && !cancelled && !lapsed.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/subscription.es");

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
        .unwrap_or_else(|e| panic!("Subscription failed to parse: {e:?}"));
    compiler::compile(&ast)
        .unwrap_or_else(|e| panic!("Subscription failed to compile: {e:?}"))
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
    subscriber: [u8; 32],
    provider: [u8; 32],
    amount: u64,
    period: u64,
) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "set_terms",
        vec![
            Value::Address(provider),
            Value::U64(amount),
            Value::U64(period),
        ],
        initial_state(bc),
        &ctx(subscriber, subscriber, 100, 10_000),
    )
    .expect("set_terms must succeed");
    r.state_changes
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "Subscription");
    let public = [
        "set_terms",
        "pay",
        "cancel",
        "provider_of",
        "subscriber_of",
        "amount_per_period",
        "period_length",
        "periods_paid",
        "total_paid",
        "last_payment",
        "is_active",
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
fn set_terms_seals_and_blocks_double_set() {
    let bc = compile_pilot();
    let subscriber = [0xAAu8; 32];
    let provider = [0xBBu8; 32];

    let armed = arm(&bc, subscriber, provider, 100, 30);

    let prov = EvaporVM::execute(
        &bc,
        "provider_of",
        vec![],
        armed.clone(),
        &ctx(subscriber, subscriber, 101, 10_000),
    )
    .unwrap();
    assert_eq!(prov.return_value, Value::Address(provider));

    let amt = EvaporVM::execute(
        &bc,
        "amount_per_period",
        vec![],
        armed.clone(),
        &ctx(subscriber, subscriber, 102, 10_000),
    )
    .unwrap();
    assert_eq!(amt.return_value, Value::U64(100));

    let err = EvaporVM::execute(
        &bc,
        "set_terms",
        vec![Value::Address(provider), Value::U64(50), Value::U64(15)],
        armed,
        &ctx(subscriber, subscriber, 103, 10_000),
    )
    .expect_err("re-set_terms must reject");
    assert!(
        format!("{err:?}").contains("already set"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_subscriber_set_terms_rejects() {
    let bc = compile_pilot();
    let subscriber = [0xAAu8; 32];
    let provider = [0xBBu8; 32];
    let attacker = [0xCCu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "set_terms",
        vec![Value::Address(provider), Value::U64(100), Value::U64(30)],
        initial_state(&bc),
        &ctx(attacker, subscriber, 100, 10_000),
    )
    .expect_err("non-subscriber set_terms must reject");
    assert!(
        format!("{err:?}").contains("only subscriber"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn pay_increments_counters() {
    let bc = compile_pilot();
    let subscriber = [0xAAu8; 32];
    let provider = [0xBBu8; 32];
    let armed = arm(&bc, subscriber, provider, 100, 30);

    let p1 = EvaporVM::execute(
        &bc,
        "pay",
        vec![],
        armed,
        &ctx(subscriber, subscriber, 200, 10_000),
    )
    .expect("first pay must succeed");
    assert_eq!(p1.return_value, Value::U64(100), "pay returns period_amount");

    let p2 = EvaporVM::execute(
        &bc,
        "pay",
        vec![],
        p1.state_changes,
        &ctx(subscriber, subscriber, 230, 10_000),
    )
    .expect("second pay must succeed");

    let periods = EvaporVM::execute(
        &bc,
        "periods_paid",
        vec![],
        p2.state_changes.clone(),
        &ctx(subscriber, subscriber, 231, 10_000),
    )
    .unwrap();
    assert_eq!(periods.return_value, Value::U64(2));

    let total = EvaporVM::execute(
        &bc,
        "total_paid",
        vec![],
        p2.state_changes.clone(),
        &ctx(subscriber, subscriber, 232, 10_000),
    )
    .unwrap();
    assert_eq!(total.return_value, Value::U64(200));

    let last = EvaporVM::execute(
        &bc,
        "last_payment",
        vec![],
        p2.state_changes,
        &ctx(subscriber, subscriber, 233, 10_000),
    )
    .unwrap();
    assert_eq!(last.return_value, Value::U64(230));
}

#[test]
fn non_subscriber_pay_rejects() {
    let bc = compile_pilot();
    let subscriber = [0xAAu8; 32];
    let provider = [0xBBu8; 32];
    let attacker = [0xCCu8; 32];
    let armed = arm(&bc, subscriber, provider, 100, 30);
    let err = EvaporVM::execute(
        &bc,
        "pay",
        vec![],
        armed,
        &ctx(attacker, subscriber, 200, 10_000),
    )
    .expect_err("non-subscriber pay must reject");
    assert!(
        format!("{err:?}").contains("only subscriber"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn either_party_can_cancel_subscriber() {
    let bc = compile_pilot();
    let subscriber = [0xAAu8; 32];
    let provider = [0xBBu8; 32];
    let armed = arm(&bc, subscriber, provider, 100, 30);

    let cancelled = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        armed,
        &ctx(subscriber, subscriber, 200, 10_000),
    )
    .expect("subscriber cancel must succeed");

    let active = EvaporVM::execute(
        &bc,
        "is_active",
        vec![],
        cancelled.state_changes,
        &ctx(subscriber, subscriber, 201, 10_000),
    )
    .unwrap();
    assert_eq!(active.return_value, Value::Bool(false));
}

#[test]
fn either_party_can_cancel_provider() {
    let bc = compile_pilot();
    let subscriber = [0xAAu8; 32];
    let provider = [0xBBu8; 32];
    let armed = arm(&bc, subscriber, provider, 100, 30);

    let cancelled = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        armed,
        &ctx(provider, subscriber, 200, 10_000),
    )
    .expect("provider cancel must succeed");

    let active = EvaporVM::execute(
        &bc,
        "is_active",
        vec![],
        cancelled.state_changes,
        &ctx(provider, subscriber, 201, 10_000),
    )
    .unwrap();
    assert_eq!(active.return_value, Value::Bool(false));
}

#[test]
fn third_party_cancel_rejects() {
    let bc = compile_pilot();
    let subscriber = [0xAAu8; 32];
    let provider = [0xBBu8; 32];
    let attacker = [0xCCu8; 32];
    let armed = arm(&bc, subscriber, provider, 100, 30);

    let err = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        armed,
        &ctx(attacker, subscriber, 200, 10_000),
    )
    .expect_err("third-party cancel must revert");
    assert!(
        format!("{err:?}").contains("not authorized"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn pay_after_cancel_rejects() {
    let bc = compile_pilot();
    let subscriber = [0xAAu8; 32];
    let provider = [0xBBu8; 32];
    let armed = arm(&bc, subscriber, provider, 100, 30);
    let cancelled = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        armed,
        &ctx(subscriber, subscriber, 200, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "pay",
        vec![],
        cancelled.state_changes,
        &ctx(subscriber, subscriber, 230, 10_000),
    )
    .expect_err("post-cancel pay must reject");
    assert!(
        format!("{err:?}").contains("cancelled"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn double_cancel_rejects() {
    let bc = compile_pilot();
    let subscriber = [0xAAu8; 32];
    let provider = [0xBBu8; 32];
    let armed = arm(&bc, subscriber, provider, 100, 30);
    let cancelled = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        armed,
        &ctx(subscriber, subscriber, 200, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        cancelled.state_changes,
        &ctx(provider, subscriber, 230, 10_000),
    )
    .expect_err("dup cancel must reject");
    assert!(
        format!("{err:?}").contains("already cancelled"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn on_evaporate_lapses_uncancelled_subscription() {
    let bc = compile_pilot();
    let subscriber = [0xAAu8; 32];
    let provider = [0xBBu8; 32];
    let armed = arm(&bc, subscriber, provider, 100, 30);

    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        armed,
        &ctx(subscriber, subscriber, 9_000, 0),
    )
    .expect("on_evaporate must execute");

    if let Some(Value::Bool(lapsed)) = evap.state_changes.get("lapsed") {
        assert!(*lapsed, "uncancelled evap must flip lapsed=true");
    } else {
        panic!("lapsed field not set");
    }
    assert!(
        evap.events.iter().any(|e| e.contains("evaporated") || e.contains("ends")),
        "evap must emit lapse event"
    );
}

#[test]
fn on_evaporate_after_cancel_does_not_relapse() {
    let bc = compile_pilot();
    let subscriber = [0xAAu8; 32];
    let provider = [0xBBu8; 32];
    let armed = arm(&bc, subscriber, provider, 100, 30);
    let cancelled = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        armed,
        &ctx(subscriber, subscriber, 200, 10_000),
    )
    .unwrap();

    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        cancelled.state_changes,
        &ctx(subscriber, subscriber, 9_000, 0),
    )
    .expect("on_evaporate must execute even post-cancel");

    if let Some(Value::Bool(lapsed)) = evap.state_changes.get("lapsed") {
        assert!(!*lapsed, "cancelled evap must NOT flip lapsed=true");
    }
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let subscriber = [0xAAu8; 32];
    let provider = [0xBBu8; 32];
    let armed = arm(&bc, subscriber, provider, 100, 30);
    for hook in &["on_grace", "on_refresh"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            armed.clone(),
            &ctx(subscriber, subscriber, 500, 100),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        let _ = r.events;
    }
}

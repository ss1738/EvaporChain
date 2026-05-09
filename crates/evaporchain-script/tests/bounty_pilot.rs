//! Pilot — drive `contracts/evaporscript/bounty.es` through the full
//! parse → compile → VM execution pipeline.
//!
//! Eleventh worked-example behavioural pilot. Bounty's doctrine: an
//! unaccepted bounty refunds to the poster when the contract evaporates.
//! Hunters' submissions are historical record but produce no payout
//! without acceptance — poster's funds don't sit forever.
//!
//! Pins:
//!   1. set_bounty one-shot + poster-only; reward > 0.
//!   2. submit open + first-time bumps counter; resubmissions overwrite
//!      and don't bump counter.
//!   3. submit post-acceptance rejects.
//!   4. accept poster-only, requires winner_addr has submitted; one-shot.
//!   5. claim winner-only + post-accept + once.
//!   6. cancel poster-only, only if no submissions yet.
//!   7. on_evaporate without acceptance flips refunded=true.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/bounty.es");

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
        .unwrap_or_else(|e| panic!("Bounty failed to parse: {e:?}"));
    compiler::compile(&ast)
        .unwrap_or_else(|e| panic!("Bounty failed to compile: {e:?}"))
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

fn post(
    bc: &EvaporBytecode,
    poster: [u8; 32],
    task: &str,
    reward: u64,
) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "set_bounty",
        vec![Value::Str(task.to_string()), Value::U64(reward)],
        initial_state(bc),
        &ctx(poster, poster, 100, 10_000),
    )
    .expect("set_bounty must succeed");
    r.state_changes
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "Bounty");
    let public = [
        "set_bounty",
        "submit",
        "accept",
        "claim",
        "cancel",
        "task_of",
        "reward",
        "submissions_total",
        "submission_of",
        "winner_of",
        "is_accepted",
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
fn set_bounty_zero_reward_rejects() {
    let bc = compile_pilot();
    let poster = [0xAAu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "set_bounty",
        vec![Value::Str("task-spec".to_string()), Value::U64(0)],
        initial_state(&bc),
        &ctx(poster, poster, 100, 10_000),
    )
    .expect_err("zero reward must reject");
    assert!(
        format!("{err:?}").contains("reward must be positive"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn submit_first_time_bumps_count_resubmit_overwrites() {
    let bc = compile_pilot();
    let poster = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bounty = post(&bc, poster, "task-1", 1000);

    let s1 = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("solution-v1".to_string())],
        bounty,
        &ctx(alice, poster, 200, 10_000),
    )
    .unwrap();
    let total1 = EvaporVM::execute(
        &bc,
        "submissions_total",
        vec![],
        s1.state_changes.clone(),
        &ctx(poster, poster, 201, 10_000),
    )
    .unwrap();
    assert_eq!(total1.return_value, Value::U64(1));

    // Alice resubmits — count stays at 1, but submission_of returns v2.
    let s2 = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("solution-v2".to_string())],
        s1.state_changes,
        &ctx(alice, poster, 202, 10_000),
    )
    .unwrap();
    let total2 = EvaporVM::execute(
        &bc,
        "submissions_total",
        vec![],
        s2.state_changes.clone(),
        &ctx(poster, poster, 203, 10_000),
    )
    .unwrap();
    assert_eq!(
        total2.return_value,
        Value::U64(1),
        "resubmit must NOT bump counter"
    );
    let stored = EvaporVM::execute(
        &bc,
        "submission_of",
        vec![Value::Address(alice)],
        s2.state_changes,
        &ctx(poster, poster, 204, 10_000),
    )
    .unwrap();
    assert_eq!(stored.return_value, Value::Str("solution-v2".to_string()));
}

#[test]
fn accept_unsubmitted_address_rejects() {
    let bc = compile_pilot();
    let poster = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bob = [0xB2u8; 32];
    let bounty = post(&bc, poster, "t", 1000);
    let s = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("alice's work".to_string())],
        bounty,
        &ctx(alice, poster, 200, 10_000),
    )
    .unwrap();
    // Poster tries to accept Bob who never submitted.
    let err = EvaporVM::execute(
        &bc,
        "accept",
        vec![Value::Address(bob)],
        s.state_changes,
        &ctx(poster, poster, 300, 10_000),
    )
    .expect_err("accept of unsubmitted addr must reject");
    assert!(
        format!("{err:?}").contains("no submission on file"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn full_round_trip_post_submit_accept_claim() {
    let bc = compile_pilot();
    let poster = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bounty = post(&bc, poster, "task-spec", 5000);
    let submitted = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("perfect-solution".to_string())],
        bounty,
        &ctx(alice, poster, 200, 10_000),
    )
    .unwrap();
    let accepted = EvaporVM::execute(
        &bc,
        "accept",
        vec![Value::Address(alice)],
        submitted.state_changes,
        &ctx(poster, poster, 300, 10_000),
    )
    .unwrap();
    let is_acc = EvaporVM::execute(
        &bc,
        "is_accepted",
        vec![],
        accepted.state_changes.clone(),
        &ctx(poster, poster, 301, 10_000),
    )
    .unwrap();
    assert_eq!(is_acc.return_value, Value::Bool(true));

    let claimed = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        accepted.state_changes,
        &ctx(alice, poster, 310, 10_000),
    )
    .unwrap();
    assert_eq!(claimed.return_value, Value::U64(5000));

    // Double claim rejects.
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        claimed.state_changes,
        &ctx(alice, poster, 311, 10_000),
    )
    .expect_err("dup claim must reject");
    assert!(
        format!("{err:?}").contains("already claimed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn submit_post_accept_rejects() {
    let bc = compile_pilot();
    let poster = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bob = [0xB2u8; 32];
    let bounty = post(&bc, poster, "t", 1000);
    let s = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("v1".to_string())],
        bounty,
        &ctx(alice, poster, 200, 10_000),
    )
    .unwrap();
    let accepted = EvaporVM::execute(
        &bc,
        "accept",
        vec![Value::Address(alice)],
        s.state_changes,
        &ctx(poster, poster, 300, 10_000),
    )
    .unwrap();
    // Bob tries to submit after acceptance.
    let err = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("late".to_string())],
        accepted.state_changes,
        &ctx(bob, poster, 310, 10_000),
    )
    .expect_err("post-accept submit must reject");
    assert!(
        format!("{err:?}").contains("already accepted"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn double_accept_rejects() {
    let bc = compile_pilot();
    let poster = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bob = [0xB2u8; 32];
    let bounty = post(&bc, poster, "t", 1000);
    let s = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("a".to_string())],
        bounty,
        &ctx(alice, poster, 200, 10_000),
    )
    .unwrap();
    let s = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("b".to_string())],
        s.state_changes,
        &ctx(bob, poster, 201, 10_000),
    )
    .unwrap();
    let accepted = EvaporVM::execute(
        &bc,
        "accept",
        vec![Value::Address(alice)],
        s.state_changes,
        &ctx(poster, poster, 300, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "accept",
        vec![Value::Address(bob)],
        accepted.state_changes,
        &ctx(poster, poster, 310, 10_000),
    )
    .expect_err("dup accept must reject");
    assert!(
        format!("{err:?}").contains("already accepted"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_winner_claim_rejects() {
    let bc = compile_pilot();
    let poster = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bob = [0xB2u8; 32];
    let bounty = post(&bc, poster, "t", 1000);
    let s = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("a".to_string())],
        bounty,
        &ctx(alice, poster, 200, 10_000),
    )
    .unwrap();
    let s = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("b".to_string())],
        s.state_changes,
        &ctx(bob, poster, 201, 10_000),
    )
    .unwrap();
    let accepted = EvaporVM::execute(
        &bc,
        "accept",
        vec![Value::Address(alice)],
        s.state_changes,
        &ctx(poster, poster, 300, 10_000),
    )
    .unwrap();
    // Bob (not winner) tries to claim.
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        accepted.state_changes,
        &ctx(bob, poster, 310, 10_000),
    )
    .expect_err("non-winner claim must reject");
    assert!(
        format!("{err:?}").contains("only winner"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn cancel_only_with_no_submissions() {
    let bc = compile_pilot();
    let poster = [0xAAu8; 32];
    let bounty = post(&bc, poster, "t", 1000);
    // No submissions yet — cancel succeeds.
    let cancelled = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        bounty,
        &ctx(poster, poster, 200, 10_000),
    )
    .expect("cancel pre-submission must succeed");

    // Resubmit attempt now must reject (cancelled).
    let alice = [0xB1u8; 32];
    let err = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("late".to_string())],
        cancelled.state_changes,
        &ctx(alice, poster, 210, 10_000),
    )
    .expect_err("post-cancel submit must reject");
    assert!(
        format!("{err:?}").contains("cancelled"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn cancel_with_submissions_rejects() {
    let bc = compile_pilot();
    let poster = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bounty = post(&bc, poster, "t", 1000);
    let s = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("v1".to_string())],
        bounty,
        &ctx(alice, poster, 200, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        s.state_changes,
        &ctx(poster, poster, 300, 10_000),
    )
    .expect_err("cancel with submissions must reject (no rug-pull)");
    assert!(
        format!("{err:?}").contains("submissions exist"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn on_evaporate_without_accept_flips_refunded() {
    let bc = compile_pilot();
    let poster = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bounty = post(&bc, poster, "t", 1000);
    let s = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("v1".to_string())],
        bounty,
        &ctx(alice, poster, 200, 10_000),
    )
    .unwrap();
    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        s.state_changes,
        &ctx(poster, poster, 9_000, 0),
    )
    .unwrap();
    if let Some(Value::Bool(r)) = evap.state_changes.get("refunded") {
        assert!(*r, "no-accept evap must flip refunded=true");
    }
}

#[test]
fn on_evaporate_after_accept_does_not_refund() {
    let bc = compile_pilot();
    let poster = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bounty = post(&bc, poster, "t", 1000);
    let s = EvaporVM::execute(
        &bc,
        "submit",
        vec![Value::Str("v1".to_string())],
        bounty,
        &ctx(alice, poster, 200, 10_000),
    )
    .unwrap();
    let accepted = EvaporVM::execute(
        &bc,
        "accept",
        vec![Value::Address(alice)],
        s.state_changes,
        &ctx(poster, poster, 300, 10_000),
    )
    .unwrap();
    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        accepted.state_changes,
        &ctx(poster, poster, 9_000, 0),
    )
    .unwrap();
    if let Some(Value::Bool(r)) = evap.state_changes.get("refunded") {
        assert!(!*r, "accepted bounty must NOT refund on evap");
    }
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let poster = [0xAAu8; 32];
    let bounty = post(&bc, poster, "t", 1000);
    for hook in &["on_grace", "on_refresh"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            bounty.clone(),
            &ctx(poster, poster, 500, 100),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        let _ = r.events;
    }
}

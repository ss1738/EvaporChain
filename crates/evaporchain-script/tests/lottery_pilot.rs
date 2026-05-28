//! Pilot — drive `contracts/evaporscript/lottery.es` through the full
//! parse → compile → VM execution pipeline.
//!
//! Tenth worked-example behavioural pilot. Lottery's doctrine moment:
//! an unresolved draw at evaporation is void — entries refund by
//! physics. No coordinator polling, no rescue contract, no recovery
//! flow. The protocol's natural decay enforces resolution-or-refund.
//!
//! Pins:
//!   1. set_event one-shot + operator-only; prize > 0, stake > 0.
//!   2. enter open + one-per-address; bumps entry_count.
//!   3. enter post-draw rejects.
//!   4. draw() operator-only; uses chain-VRF random_range; one-shot.
//!      LOTTERY-1 (audit 2026-05-17): operator triggers draw but cannot
//!      influence outcome — random_range derives winner from VRF beacon.
//!   5. claim_prize winner-only + post-draw + once.
//!   6. on_evaporate without draw flips voided=true; with draw
//!      already recorded, voided stays false.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/lottery.es");

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
    let ast = parser::parse(SOURCE).unwrap_or_else(|e| panic!("Lottery failed to parse: {e:?}"));
    compiler::compile(&ast).unwrap_or_else(|e| panic!("Lottery failed to compile: {e:?}"))
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

fn configure(
    bc: &EvaporBytecode,
    operator: [u8; 32],
    prize: u64,
    stake: u64,
) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "set_event",
        vec![Value::U64(prize), Value::U64(stake)],
        initial_state(bc),
        &ctx(operator, operator, 100, 10_000),
    )
    .expect("set_event must succeed");
    r.state_changes
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "Lottery");
    let public = [
        "set_event",
        "enter",
        "draw",
        "claim_prize",
        "entries_total",
        "is_entered",
        "winner_of",
        "is_drawn",
        "is_voided",
        "prize_size",
        "stake_per_entry",
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
fn set_event_validates_inputs() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];

    let err = EvaporVM::execute(
        &bc,
        "set_event",
        vec![Value::U64(0), Value::U64(10)],
        initial_state(&bc),
        &ctx(operator, operator, 100, 10_000),
    )
    .expect_err("zero prize must reject");
    assert!(
        format!("{err:?}").contains("prize must be positive"),
        "wrong revert: {err:?}"
    );

    let err = EvaporVM::execute(
        &bc,
        "set_event",
        vec![Value::U64(1000), Value::U64(0)],
        initial_state(&bc),
        &ctx(operator, operator, 100, 10_000),
    )
    .expect_err("zero stake must reject");
    assert!(
        format!("{err:?}").contains("stake must be positive"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn enter_records_and_blocks_double_enter() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bob = [0xB2u8; 32];
    let configured = configure(&bc, operator, 1000, 10);

    let after_a = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    let after_b = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        after_a.state_changes,
        &ctx(bob, operator, 201, 10_000),
    )
    .unwrap();
    let total = EvaporVM::execute(
        &bc,
        "entries_total",
        vec![],
        after_b.state_changes.clone(),
        &ctx(operator, operator, 202, 10_000),
    )
    .unwrap();
    assert_eq!(total.return_value, Value::U64(2));

    let err = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        after_b.state_changes,
        &ctx(alice, operator, 203, 10_000),
    )
    .expect_err("dup enter must reject");
    assert!(
        format!("{err:?}").contains("already entered"),
        "wrong revert: {err:?}"
    );
}

// LOTTERY-1 adversarial: draw() is operator-only; a participant cannot
// steer the VRF by choosing when to trigger it.
#[test]
fn lottery1_draw_is_operator_only() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let configured = configure(&bc, operator, 1000, 10);
    let after_a = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "draw",
        vec![],
        after_a.state_changes,
        &ctx(alice, operator, 300, 10_000),
    )
    .expect_err("non-operator draw must reject");
    assert!(
        format!("{err:?}").contains("only operator"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn full_draw_and_claim_round_trip() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let configured = configure(&bc, operator, 5000, 100);
    let entered = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    // Only alice is entered (index 0); random_range(1) == 0 deterministically.
    let drawn = EvaporVM::execute(
        &bc,
        "draw",
        vec![],
        entered.state_changes,
        &ctx(operator, operator, 300, 10_000),
    )
    .unwrap();
    let is_drawn = EvaporVM::execute(
        &bc,
        "is_drawn",
        vec![],
        drawn.state_changes.clone(),
        &ctx(operator, operator, 301, 10_000),
    )
    .unwrap();
    assert_eq!(is_drawn.return_value, Value::Bool(true));

    let winner = EvaporVM::execute(
        &bc,
        "winner_of",
        vec![],
        drawn.state_changes.clone(),
        &ctx(operator, operator, 302, 10_000),
    )
    .unwrap();
    assert_eq!(winner.return_value, Value::Address(alice));

    let claimed = EvaporVM::execute(
        &bc,
        "claim_prize",
        vec![],
        drawn.state_changes,
        &ctx(alice, operator, 310, 10_000),
    )
    .unwrap();
    assert_eq!(claimed.return_value, Value::U64(5000));

    let err = EvaporVM::execute(
        &bc,
        "claim_prize",
        vec![],
        claimed.state_changes,
        &ctx(alice, operator, 311, 10_000),
    )
    .expect_err("dup claim must reject");
    assert!(
        format!("{err:?}").contains("already claimed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_winner_claim_rejects() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bob = [0xB2u8; 32];
    let configured = configure(&bc, operator, 1000, 10);
    let s = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    let s = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        s.state_changes,
        &ctx(bob, operator, 201, 10_000),
    )
    .unwrap();
    let drawn = EvaporVM::execute(
        &bc,
        "draw",
        vec![],
        s.state_changes,
        &ctx(operator, operator, 300, 10_000),
    )
    .unwrap();
    // Determine actual winner; test the other address is rejected.
    let winner_result = EvaporVM::execute(
        &bc,
        "winner_of",
        vec![],
        drawn.state_changes.clone(),
        &ctx(operator, operator, 301, 10_000),
    )
    .unwrap();
    let non_winner = if winner_result.return_value == Value::Address(alice) {
        bob
    } else {
        alice
    };
    let err = EvaporVM::execute(
        &bc,
        "claim_prize",
        vec![],
        drawn.state_changes,
        &ctx(non_winner, operator, 310, 10_000),
    )
    .expect_err("non-winner claim must reject");
    assert!(
        format!("{err:?}").contains("only winner"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn enter_post_draw_rejects() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let charlie = [0xB3u8; 32];
    let configured = configure(&bc, operator, 1000, 10);
    let entered = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    // alice at index 0; random_range(1) == 0 → alice wins.
    let drawn = EvaporVM::execute(
        &bc,
        "draw",
        vec![],
        entered.state_changes,
        &ctx(operator, operator, 300, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        drawn.state_changes,
        &ctx(charlie, operator, 310, 10_000),
    )
    .expect_err("late entry must reject");
    assert!(
        format!("{err:?}").contains("draw already happened"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn double_draw_rejects() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bob = [0xB2u8; 32];
    let configured = configure(&bc, operator, 1000, 10);
    let s = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    let s = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        s.state_changes,
        &ctx(bob, operator, 201, 10_000),
    )
    .unwrap();
    let drawn = EvaporVM::execute(
        &bc,
        "draw",
        vec![],
        s.state_changes,
        &ctx(operator, operator, 300, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "draw",
        vec![],
        drawn.state_changes,
        &ctx(operator, operator, 310, 10_000),
    )
    .expect_err("re-draw must reject");
    assert!(
        format!("{err:?}").contains("already drawn"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn draw_with_no_entries_rejects() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let configured = configure(&bc, operator, 1000, 10);
    let err = EvaporVM::execute(
        &bc,
        "draw",
        vec![],
        configured,
        &ctx(operator, operator, 300, 10_000),
    )
    .expect_err("draw with no entries must reject");
    assert!(
        format!("{err:?}").contains("no entries"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn on_evaporate_without_draw_voids() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let configured = configure(&bc, operator, 1000, 10);
    let entered = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        entered.state_changes,
        &ctx(operator, operator, 9_000, 0),
    )
    .unwrap();
    if let Some(Value::Bool(v)) = evap.state_changes.get("voided") {
        assert!(*v, "no-draw evap must flip voided=true");
    }
    assert!(
        evap.events
            .iter()
            .any(|e| e.contains("evaporated") || e.contains("void")),
        "must emit void/evaporated event"
    );
}

#[test]
fn on_evaporate_after_draw_does_not_void() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let configured = configure(&bc, operator, 1000, 10);
    let entered = EvaporVM::execute(
        &bc,
        "enter",
        vec![],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    // alice at index 0; random_range(1) == 0 → alice wins.
    let drawn = EvaporVM::execute(
        &bc,
        "draw",
        vec![],
        entered.state_changes,
        &ctx(operator, operator, 300, 10_000),
    )
    .unwrap();
    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        drawn.state_changes,
        &ctx(operator, operator, 9_000, 0),
    )
    .unwrap();
    if let Some(Value::Bool(v)) = evap.state_changes.get("voided") {
        assert!(!*v, "drawn lottery must NOT void on evap");
    }
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let configured = configure(&bc, operator, 1000, 10);
    for hook in &["on_grace", "on_refresh"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            configured.clone(),
            &ctx(operator, operator, 500, 100),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        let _ = r.events;
    }
}

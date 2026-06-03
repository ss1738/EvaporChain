//! Pilot — drive `contracts/evaporscript/evaporcash_note.es` through
//! the full parse → compile → VM execution pipeline.
//!
//! 14th worked-example behavioural pilot. EvaporCashNote's doctrine
//! moment: native demurrage money. ONE note = ONE contract instance;
//! the note's own `energy` builtin IS its spendable value, so a
//! hoarded note loses value by chain physics (the evaporation engine)
//! with no keeper bot, no in-contract decay formula, and no
//! off-chain timer. The Wörgl/Gesell "money rots if you hoard it"
//! incentive, native. `on_evaporate` with `spent == false` is the
//! demurrage taken to its physical limit — value lost to hoarding.
//!
//! Pins:
//!   1. issue() one-shot + owner-only; face > 0.
//!   2. spend() holder-only + once; transitions holder + flips spent.
//!   3. spend after evaporation premise: `live_value()` reads the
//!      `energy` builtin (NOT a stored snapshot), so the spendable
//!      value moves with the chain's evaporation engine.
//!   4. face_value() returns the issue-time snapshot, NOT the live
//!      value (the two-value separation is the doctrine's whole
//!      point — face for accounting, energy for what you can spend).
//!   5. on_evaporate emits the "value lost to hoarding" event only
//!      when spent == false; a spent note's evaporation is silent
//!      because the value was already preserved off-chain.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/evaporcash_note.es");

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
    let ast =
        parser::parse(SOURCE).unwrap_or_else(|e| panic!("EvaporCashNote failed to parse: {e:?}"));
    compiler::compile(&ast)
        .unwrap_or_else(|e| panic!("EvaporCashNote failed to compile: {e:?}"))
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

fn issue(
    bc: &EvaporBytecode,
    issuer: [u8; 32],
    holder: [u8; 32],
    face: u64,
    energy_at_issue: u64,
) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "issue",
        vec![Value::Address(holder), Value::U64(face)],
        initial_state(bc),
        &ctx(issuer, issuer, 100, energy_at_issue),
    )
    .expect("issue must succeed");
    r.state_changes
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "EvaporCashNote");
    let public = [
        "issue",
        "spend",
        "current_holder",
        "is_spent",
        "face_value",
        "live_value",
        "issued_epoch",
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
fn issue_validates_inputs() {
    let bc = compile_pilot();
    let issuer = [0xAAu8; 32];
    let alice = [0xB1u8; 32];

    // Non-owner cannot issue.
    let stranger = [0xCCu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "issue",
        vec![Value::Address(alice), Value::U64(1000)],
        initial_state(&bc),
        &ctx(stranger, issuer, 100, 10_000),
    )
    .expect_err("non-issuer must reject");
    assert!(
        format!("{err:?}").contains("only issuer can issue this note"),
        "wrong revert: {err:?}"
    );

    // Zero face is rejected.
    let err = EvaporVM::execute(
        &bc,
        "issue",
        vec![Value::Address(alice), Value::U64(0)],
        initial_state(&bc),
        &ctx(issuer, issuer, 100, 10_000),
    )
    .expect_err("zero face must reject");
    assert!(
        format!("{err:?}").contains("face value must be positive"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn issue_is_one_shot() {
    let bc = compile_pilot();
    let issuer = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bob = [0xB2u8; 32];
    let after_first = issue(&bc, issuer, alice, 1000, 10_000);

    let err = EvaporVM::execute(
        &bc,
        "issue",
        vec![Value::Address(bob), Value::U64(500)],
        after_first,
        &ctx(issuer, issuer, 200, 10_000),
    )
    .expect_err("second issue must reject");
    assert!(
        format!("{err:?}").contains("note already issued"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn spend_holder_only_and_one_shot() {
    let bc = compile_pilot();
    let issuer = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bob = [0xB2u8; 32];
    let issued = issue(&bc, issuer, alice, 1000, 10_000);

    // Non-holder cannot spend.
    let err = EvaporVM::execute(
        &bc,
        "spend",
        vec![Value::Address(bob)],
        issued.clone(),
        &ctx(bob, issuer, 150, 10_000),
    )
    .expect_err("non-holder must reject");
    assert!(
        format!("{err:?}").contains("only the holder can spend"),
        "wrong revert: {err:?}"
    );

    // Holder can spend — transfers to recipient and flips spent.
    let after_spend =
        EvaporVM::execute(&bc, "spend", vec![Value::Address(bob)], issued, &ctx(alice, issuer, 150, 10_000))
            .expect("holder spend must succeed");
    let holder_q = EvaporVM::execute(
        &bc,
        "current_holder",
        vec![],
        after_spend.state_changes.clone(),
        &ctx(alice, issuer, 151, 10_000),
    )
    .unwrap();
    assert_eq!(holder_q.return_value, Value::Address(bob));
    let spent_q = EvaporVM::execute(
        &bc,
        "is_spent",
        vec![],
        after_spend.state_changes.clone(),
        &ctx(alice, issuer, 151, 10_000),
    )
    .unwrap();
    assert_eq!(spent_q.return_value, Value::Bool(true));

    // Second spend rejects.
    let err = EvaporVM::execute(
        &bc,
        "spend",
        vec![Value::Address(alice)],
        after_spend.state_changes,
        &ctx(bob, issuer, 152, 10_000),
    )
    .expect_err("double spend must reject");
    assert!(
        format!("{err:?}").contains("note already spent"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn live_value_reads_energy_builtin_not_stored_snapshot() {
    // The doctrine claim — the note's spendable value IS the chain's
    // physical energy, never re-derived — depends on live_value()
    // reading the `energy` builtin at call time. Issue with face=5000
    // and energy_at_issue=10000; later read at a different energy
    // and confirm live_value tracks the CURRENT energy, not the
    // issue-time snapshot.
    let bc = compile_pilot();
    let issuer = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let issued = issue(&bc, issuer, alice, 5000, 10_000);

    // live_value at issue-time energy → 10_000 (the current energy,
    // NOT the face value 5000).
    let live_q = EvaporVM::execute(
        &bc,
        "live_value",
        vec![],
        issued.clone(),
        &ctx(alice, issuer, 100, 10_000),
    )
    .unwrap();
    assert_eq!(live_q.return_value, Value::U64(10_000));

    // live_value queried later at decayed energy → reflects the
    // current energy (3000), NOT the face (5000) and NOT the
    // issue-time energy (10_000). This is the demurrage premise:
    // hoarding the note bleeds live_value by physics.
    let live_q2 = EvaporVM::execute(
        &bc,
        "live_value",
        vec![],
        issued.clone(),
        &ctx(alice, issuer, 500, 3_000),
    )
    .unwrap();
    assert_eq!(live_q2.return_value, Value::U64(3_000));

    // face_value is the issue-time snapshot — must NOT track current
    // energy. The two-value separation is the doctrine's whole point.
    let face_q = EvaporVM::execute(
        &bc,
        "face_value",
        vec![],
        issued,
        &ctx(alice, issuer, 500, 3_000),
    )
    .unwrap();
    assert_eq!(face_q.return_value, Value::U64(5000));
}

#[test]
fn issued_epoch_records_issue_time() {
    let bc = compile_pilot();
    let issuer = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let issued = issue(&bc, issuer, alice, 1000, 10_000);
    let q = EvaporVM::execute(
        &bc,
        "issued_epoch",
        vec![],
        issued,
        &ctx(alice, issuer, 250, 5_000),
    )
    .unwrap();
    // issue() ran at epoch 100; issued_at must be 100 regardless of
    // current epoch at read time.
    assert_eq!(q.return_value, Value::U64(100));
}

#[test]
fn on_evaporate_emits_hoarding_loss_only_when_unspent() {
    let bc = compile_pilot();
    let issuer = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bob = [0xB2u8; 32];

    // Unspent note: on_evaporate emits the hoarding-loss event.
    let issued = issue(&bc, issuer, alice, 1000, 10_000);
    let evap_unspent = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        issued.clone(),
        &ctx(alice, issuer, 9999, 0),
    )
    .expect("on_evaporate must succeed");
    let saw_hoarding_emit = evap_unspent
        .events
        .iter()
        .any(|e| format!("{e:?}").contains("value lost to hoarding"));
    assert!(
        saw_hoarding_emit,
        "unspent on_evaporate must emit hoarding-loss event; events: {:?}",
        evap_unspent.events
    );

    // Spent note: on_evaporate must NOT emit the hoarding event —
    // the value was preserved off-chain at spend time.
    let after_spend = EvaporVM::execute(
        &bc,
        "spend",
        vec![Value::Address(bob)],
        issued,
        &ctx(alice, issuer, 150, 10_000),
    )
    .expect("spend must succeed");
    let evap_spent = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        after_spend.state_changes,
        &ctx(bob, issuer, 9999, 0),
    )
    .expect("on_evaporate must succeed");
    let saw_hoarding_emit = evap_spent
        .events
        .iter()
        .any(|e| format!("{e:?}").contains("value lost to hoarding"));
    assert!(
        !saw_hoarding_emit,
        "spent on_evaporate must NOT emit hoarding-loss event; events: {:?}",
        evap_spent.events
    );
}

//! Pilot — drive `contracts/evaporscript/mortal_message.es` through the full
//! parse → compile → VM execution pipeline.
//!
//! This is the B-track reference: the canonical EvaporScript contract for
//! the mortal-messages dapp. Anything broken here means a future TS-to-
//! EvaporScript migration of `dapps/mortal-messages/` would not compile.
//!
//! Asserts the lifecycle invariants the contract is supposed to enforce:
//!   1. Only `owner` (the deployer / sender) can `set_payload`.
//!   2. The contract is one-shot: a second `set_payload` after sealing
//!      reverts.
//!   3. Sender and recipient can `read`; nobody else can.
//!   4. `read` before sealing reverts.
//!   5. The on-chain hooks (`on_grace`, `on_refresh`, `on_evaporate`)
//!      compile + run cleanly with the documented event emissions.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/mortal_message.es");

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
        parser::parse(SOURCE).unwrap_or_else(|e| panic!("MortalMessage failed to parse: {e:?}"));
    compiler::compile(&ast).unwrap_or_else(|e| panic!("MortalMessage failed to compile: {e:?}"))
}

/// Seed initial state with the schema's declared defaults. Without this,
/// missing fields read as `Value::Null` from the VM, which fails the
/// `self.sealed == false` guard in the contract on the very first call.
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
    assert_eq!(bc.name, "MortalMessage");
    // Five public methods + four lifecycle hooks (on_grace/on_refresh/
    // on_evaporate). Method count is the public-callable surface.
    let public = ["set_payload", "read", "record_boost", "inspect"];
    for m in &public {
        assert!(
            bc.methods.contains_key(*m),
            "method `{m}` missing from compiled bytecode (have: {:?})",
            bc.methods.keys().collect::<Vec<_>>()
        );
    }
    // State schema must include all six fields.
    let names: Vec<&str> = bc
        .state_schema
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    for f in &[
        "body",
        "recipient",
        "sender",
        "sealed",
        "boost_count",
        "last_boost_epoch",
    ] {
        assert!(
            names.contains(f),
            "state field `{f}` missing from schema (have: {names:?})"
        );
    }
}

#[test]
fn set_payload_seals_then_read_returns_body() {
    let bc = compile_pilot();
    let sender = [0xAAu8; 32];
    let recipient = [0xBBu8; 32];

    // Step 1: sender deploys + seals. caller == owner == sender.
    let result = EvaporVM::execute(
        &bc,
        "set_payload",
        vec![
            Value::Str("hello, world".to_string()),
            Value::Address(recipient),
        ],
        initial_state(&bc),
        &ctx(sender, sender, 100, 1000),
    )
    .expect("set_payload from owner must succeed");
    assert!(
        result.events.iter().any(|e| e.contains("sealed")),
        "set_payload must emit `message sealed`, events: {:?}",
        result.events
    );

    // Step 2: recipient reads the body. caller == recipient, owner ==
    // sender — read must succeed and return the body.
    let read_result = EvaporVM::execute(
        &bc,
        "read",
        vec![],
        result.state_changes.clone(),
        &ctx(recipient, sender, 105, 950),
    )
    .expect("read from recipient must succeed");
    assert_eq!(
        read_result.return_value,
        Value::Str("hello, world".to_string()),
        "read must return the sealed body verbatim"
    );

    // Step 3: sender (== owner) reading their own message also works.
    let sender_read = EvaporVM::execute(
        &bc,
        "read",
        vec![],
        result.state_changes.clone(),
        &ctx(sender, sender, 105, 950),
    )
    .expect("read from sender (owner) must succeed");
    assert_eq!(
        sender_read.return_value,
        Value::Str("hello, world".to_string())
    );
}

#[test]
fn read_before_seal_reverts() {
    let bc = compile_pilot();
    let sender = [0xAAu8; 32];
    // Empty state — no set_payload ran yet.
    let err = EvaporVM::execute(
        &bc,
        "read",
        vec![],
        initial_state(&bc),
        &ctx(sender, sender, 100, 1000),
    )
    .expect_err("read before seal must revert");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("not yet sealed") || msg.contains("sealed"),
        "revert reason must mention sealing: {msg}"
    );
}

#[test]
fn non_owner_cannot_seal() {
    let bc = compile_pilot();
    let sender = [0xAAu8; 32];
    let attacker = [0xCCu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "set_payload",
        vec![
            Value::Str("malicious body".to_string()),
            Value::Address(attacker),
        ],
        initial_state(&bc),
        // caller != owner: the attacker is calling, but owner is sender.
        &ctx(attacker, sender, 100, 1000),
    )
    .expect_err("only sender (owner) can seal");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("only sender"),
        "revert reason must mention sender: {msg}"
    );
}

#[test]
fn double_seal_reverts() {
    let bc = compile_pilot();
    let sender = [0xAAu8; 32];
    let recipient = [0xBBu8; 32];

    // First seal succeeds.
    let first = EvaporVM::execute(
        &bc,
        "set_payload",
        vec![
            Value::Str("original".to_string()),
            Value::Address(recipient),
        ],
        initial_state(&bc),
        &ctx(sender, sender, 100, 1000),
    )
    .expect("first set_payload must succeed");

    // Second seal must revert — the contract is one-shot.
    let err = EvaporVM::execute(
        &bc,
        "set_payload",
        vec![Value::Str("rewrite".to_string()), Value::Address(recipient)],
        first.state_changes,
        &ctx(sender, sender, 200, 800),
    )
    .expect_err("re-seal must revert");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("already sealed"),
        "revert reason must mention 'already sealed': {msg}"
    );
}

#[test]
fn unauthorized_reader_reverts() {
    let bc = compile_pilot();
    let sender = [0xAAu8; 32];
    let recipient = [0xBBu8; 32];
    let outsider = [0xDDu8; 32];

    let sealed = EvaporVM::execute(
        &bc,
        "set_payload",
        vec![Value::Str("private".to_string()), Value::Address(recipient)],
        initial_state(&bc),
        &ctx(sender, sender, 100, 1000),
    )
    .expect("seal must succeed");

    // Outsider is neither sender nor recipient.
    let err = EvaporVM::execute(
        &bc,
        "read",
        vec![],
        sealed.state_changes,
        &ctx(outsider, sender, 110, 900),
    )
    .expect_err("outsider read must revert");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("not authorized"),
        "revert reason must mention authorization: {msg}"
    );
}

#[test]
fn record_boost_increments_counter() {
    let bc = compile_pilot();
    let sender = [0xAAu8; 32];
    let recipient = [0xBBu8; 32];

    let sealed = EvaporVM::execute(
        &bc,
        "set_payload",
        vec![Value::Str("body".to_string()), Value::Address(recipient)],
        initial_state(&bc),
        &ctx(sender, sender, 100, 1000),
    )
    .expect("seal must succeed");

    let boosted = EvaporVM::execute(
        &bc,
        "record_boost",
        vec![],
        sealed.state_changes,
        &ctx(sender, sender, 200, 1200),
    )
    .expect("record_boost must succeed after seal");

    let inspected = EvaporVM::execute(
        &bc,
        "inspect",
        vec![],
        boosted.state_changes,
        &ctx(sender, sender, 201, 1200),
    )
    .expect("inspect must succeed");
    assert_eq!(
        inspected.return_value,
        Value::U64(1),
        "boost_count must be 1 after a single record_boost"
    );
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    // The chain runtime invokes on_grace / on_refresh / on_evaporate
    // directly — no caller / no args. We just verify the hooks compile,
    // run without runtime errors, and emit the documented events.
    let bc = compile_pilot();
    let sender = [0xAAu8; 32];
    let recipient = [0xBBu8; 32];

    // Seal first so on_refresh can mutate boost_count.
    let sealed = EvaporVM::execute(
        &bc,
        "set_payload",
        vec![Value::Str("body".to_string()), Value::Address(recipient)],
        initial_state(&bc),
        &ctx(sender, sender, 100, 1000),
    )
    .expect("seal must succeed");

    for hook in &["on_grace", "on_refresh", "on_evaporate"] {
        let result = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            sealed.state_changes.clone(),
            &ctx(sender, sender, 500, 0),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        assert!(
            !result.events.is_empty(),
            "hook {hook} must emit at least one event"
        );
    }
}

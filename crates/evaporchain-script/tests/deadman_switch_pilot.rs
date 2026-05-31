//! Pilot — drive `contracts/evaporscript/deadman_switch.es` through the
//! full parse → compile → VM execution pipeline.
//!
//! Doctrine moment: every other dead-man's switch needs an external
//! keeper (Chainlink, Gelato) to ping the contract and check the
//! deadline. EvaporChain doesn't — the chain's own epoch advancement
//! IS the trigger. After `last_refresh_epoch + refresh_window` epochs
//! lapse, the `release_dead()` guard naturally permits anyone to
//! fire the switch.
//!
//! Pins the documented invariants:
//!   1. arm() is owner-only and one-shot; seeds last_refresh_epoch
//!      + has_refreshed so the countdown starts from arm-time.
//!   2. refresh() is holder-only; bumps refresh_count.
//!   3. trigger_early() is holder-only; sets released + released_by =
//!      holder + revealed_secret = arg.
//!   4. release_dead() is anyone, but ONLY after deadline lapses;
//!      sets released + released_by = caller.
//!   5. transfer_holder() is current-holder-only and post-release
//!      is rejected.
//!   6. View functions agree with state across (alive / releasable
//!      / released) tri-state.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/deadman_switch.es");

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

fn arm(
    bc: &EvaporBytecode,
    owner: [u8; 32],
    holder: [u8; 32],
    payload_hash: &str,
    window: u64,
    epoch: u64,
) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "arm",
        vec![
            Value::Address(holder),
            Value::String(payload_hash.to_string()),
            Value::U64(window),
        ],
        initial_state(bc),
        &ctx(owner, owner, epoch, 10_000),
    )
    .expect("arm must succeed");
    r.state_changes
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "DeadManSwitch");
    let expected = [
        "arm",
        "refresh",
        "trigger_early",
        "release_dead",
        "transfer_holder",
        "is_armed",
        "is_released",
        "is_alive",
        "is_releasable",
        "epochs_until_deadline",
        "secret_hash_view",
        "revealed_secret_view",
        "released_at_view",
        "refresh_count_view",
        "last_refresh_view",
        "holder_view",
        "is_holder",
    ];
    for m in &expected {
        assert!(
            bc.methods.contains_key(*m),
            "method `{m}` missing from compiled bytecode"
        );
    }
}

#[test]
fn arm_is_owner_only_one_shot() {
    let bc = compile_pilot();
    let owner = [1u8; 32];
    let stranger = [2u8; 32];
    let holder = [3u8; 32];

    // Stranger can't arm.
    let bad = EvaporVM::execute(
        &bc,
        "arm",
        vec![
            Value::Address(holder),
            Value::String("0xdeadbeef".to_string()),
            Value::U64(100),
        ],
        initial_state(&bc),
        &ctx(stranger, owner, 50, 10_000),
    );
    assert!(bad.is_err(), "stranger must not be able to arm");

    // Owner arms once.
    let state = arm(&bc, owner, holder, "0xdeadbeef", 100, 50);
    assert_eq!(state.get("sealed"), Some(&Value::Bool(true)));
    assert_eq!(state.get("has_refreshed"), Some(&Value::Bool(true)));
    assert_eq!(state.get("refresh_count"), Some(&Value::U64(1)));
    assert_eq!(state.get("last_refresh_epoch"), Some(&Value::U64(50)));
    assert_eq!(state.get("refresh_window"), Some(&Value::U64(100)));
    assert_eq!(state.get("holder"), Some(&Value::Address(holder)));

    // Second arm fails (already sealed).
    let again = EvaporVM::execute(
        &bc,
        "arm",
        vec![
            Value::Address(holder),
            Value::String("0xbeefcafe".to_string()),
            Value::U64(50),
        ],
        state,
        &ctx(owner, owner, 60, 10_000),
    );
    assert!(again.is_err(), "double-arm must be rejected");
}

#[test]
fn refresh_is_holder_only_and_resets_deadline() {
    let bc = compile_pilot();
    let owner = [1u8; 32];
    let holder = [3u8; 32];
    let stranger = [4u8; 32];

    let state = arm(&bc, owner, holder, "0xdeadbeef", 100, 50);

    // Stranger can't refresh.
    let bad = EvaporVM::execute(
        &bc,
        "refresh",
        vec![],
        state.clone(),
        &ctx(stranger, owner, 80, 10_000),
    );
    assert!(bad.is_err(), "stranger must not be able to refresh");

    // Holder refreshes — deadline + refresh_count advance.
    let r = EvaporVM::execute(
        &bc,
        "refresh",
        vec![],
        state,
        &ctx(holder, owner, 80, 10_000),
    )
    .expect("holder refresh must succeed");
    assert_eq!(r.state_changes.get("last_refresh_epoch"), Some(&Value::U64(80)));
    assert_eq!(r.state_changes.get("refresh_count"), Some(&Value::U64(2)));
}

#[test]
fn release_dead_blocked_before_deadline_open_after() {
    let bc = compile_pilot();
    let owner = [1u8; 32];
    let holder = [3u8; 32];
    let releaser = [5u8; 32];

    let state = arm(&bc, owner, holder, "0xdeadbeef", 100, 50);

    // 50 + 100 = 150 — deadline epoch. Anything before is rejected.
    let before = EvaporVM::execute(
        &bc,
        "release_dead",
        vec![Value::String("".to_string())],
        state.clone(),
        &ctx(releaser, owner, 149, 10_000),
    );
    assert!(before.is_err(), "release_dead before deadline must be rejected");

    // At or past the deadline, anyone may fire.
    let after = EvaporVM::execute(
        &bc,
        "release_dead",
        vec![Value::String("the secret".to_string())],
        state,
        &ctx(releaser, owner, 150, 10_000),
    )
    .expect("release_dead at deadline must succeed");
    assert_eq!(after.state_changes.get("released"), Some(&Value::Bool(true)));
    assert_eq!(after.state_changes.get("released_at_epoch"), Some(&Value::U64(150)));
    assert_eq!(after.state_changes.get("released_by"), Some(&Value::Address(releaser)));
    assert_eq!(
        after.state_changes.get("revealed_secret"),
        Some(&Value::String("the secret".to_string())),
    );
}

#[test]
fn trigger_early_is_holder_only() {
    let bc = compile_pilot();
    let owner = [1u8; 32];
    let holder = [3u8; 32];
    let stranger = [6u8; 32];

    let state = arm(&bc, owner, holder, "0xdeadbeef", 100, 50);

    // Stranger can't trigger early.
    let bad = EvaporVM::execute(
        &bc,
        "trigger_early",
        vec![Value::String("".to_string())],
        state.clone(),
        &ctx(stranger, owner, 75, 10_000),
    );
    assert!(bad.is_err(), "stranger must not be able to trigger early");

    // Holder triggers early — sets released, released_by=holder.
    let r = EvaporVM::execute(
        &bc,
        "trigger_early",
        vec![Value::String("voluntary release".to_string())],
        state,
        &ctx(holder, owner, 75, 10_000),
    )
    .expect("holder trigger_early must succeed");
    assert_eq!(r.state_changes.get("released"), Some(&Value::Bool(true)));
    assert_eq!(r.state_changes.get("released_at_epoch"), Some(&Value::U64(75)));
    assert_eq!(r.state_changes.get("released_by"), Some(&Value::Address(holder)));
}

#[test]
fn double_release_rejected() {
    let bc = compile_pilot();
    let owner = [1u8; 32];
    let holder = [3u8; 32];

    let state = arm(&bc, owner, holder, "0xdeadbeef", 100, 50);

    let r = EvaporVM::execute(
        &bc,
        "trigger_early",
        vec![Value::String("first".to_string())],
        state,
        &ctx(holder, owner, 75, 10_000),
    )
    .expect("first release must succeed");

    let second = EvaporVM::execute(
        &bc,
        "release_dead",
        vec![Value::String("second".to_string())],
        r.state_changes,
        &ctx([7u8; 32], owner, 200, 10_000),
    );
    assert!(second.is_err(), "double-release must be rejected");
}

#[test]
fn transfer_holder_passes_baton_keeps_deadline() {
    let bc = compile_pilot();
    let owner = [1u8; 32];
    let holder1 = [3u8; 32];
    let holder2 = [4u8; 32];

    let state = arm(&bc, owner, holder1, "0xdeadbeef", 100, 50);

    let r = EvaporVM::execute(
        &bc,
        "transfer_holder",
        vec![Value::Address(holder2)],
        state,
        &ctx(holder1, owner, 75, 10_000),
    )
    .expect("transfer_holder must succeed");
    assert_eq!(r.state_changes.get("holder"), Some(&Value::Address(holder2)));
    // Deadline must NOT reset on transfer — the new holder inherits
    // whatever epochs are left.
    assert_eq!(r.state_changes.get("last_refresh_epoch"), Some(&Value::U64(50)));
}

#[test]
fn views_track_alive_releasable_released_tristate() {
    let bc = compile_pilot();
    let owner = [1u8; 32];
    let holder = [3u8; 32];

    let state = arm(&bc, owner, holder, "0xdeadbeef", 100, 50);

    // Epoch 75 — alive.
    let alive = EvaporVM::execute(
        &bc,
        "is_alive",
        vec![],
        state.clone(),
        &ctx(holder, owner, 75, 10_000),
    )
    .expect("is_alive must succeed");
    assert_eq!(alive.return_value, Some(Value::Bool(true)));

    let releasable_pre = EvaporVM::execute(
        &bc,
        "is_releasable",
        vec![],
        state.clone(),
        &ctx(holder, owner, 75, 10_000),
    )
    .expect("is_releasable must succeed");
    assert_eq!(releasable_pre.return_value, Some(Value::Bool(false)));

    // Epoch 200 — deadline lapsed, releasable.
    let releasable_post = EvaporVM::execute(
        &bc,
        "is_releasable",
        vec![],
        state.clone(),
        &ctx(holder, owner, 200, 10_000),
    )
    .expect("is_releasable must succeed");
    assert_eq!(releasable_post.return_value, Some(Value::Bool(true)));

    let alive_after = EvaporVM::execute(
        &bc,
        "is_alive",
        vec![],
        state.clone(),
        &ctx(holder, owner, 200, 10_000),
    )
    .expect("is_alive must succeed");
    assert_eq!(alive_after.return_value, Some(Value::Bool(false)));

    // After release, both is_alive and is_releasable flip to false;
    // is_released flips to true.
    let r = EvaporVM::execute(
        &bc,
        "release_dead",
        vec![Value::String("x".to_string())],
        state,
        &ctx([8u8; 32], owner, 200, 10_000),
    )
    .expect("release_dead must succeed");
    let post = r.state_changes;

    for (m, expected) in &[
        ("is_alive", false),
        ("is_releasable", false),
        ("is_released", true),
    ] {
        let v = EvaporVM::execute(
            &bc,
            m,
            vec![],
            post.clone(),
            &ctx(holder, owner, 250, 10_000),
        )
        .expect("view must succeed");
        assert_eq!(
            v.return_value,
            Some(Value::Bool(*expected)),
            "{m} mismatch after release"
        );
    }
}

#[test]
fn epochs_until_deadline_counts_down() {
    let bc = compile_pilot();
    let owner = [1u8; 32];
    let holder = [3u8; 32];

    let state = arm(&bc, owner, holder, "0xdeadbeef", 100, 50);

    for (epoch, expected) in &[(50u64, 100u64), (100, 50), (149, 1), (150, 0), (200, 0)] {
        let r = EvaporVM::execute(
            &bc,
            "epochs_until_deadline",
            vec![],
            state.clone(),
            &ctx(holder, owner, *epoch, 10_000),
        )
        .expect("epochs_until_deadline must succeed");
        assert_eq!(
            r.return_value,
            Some(Value::U64(*expected)),
            "epochs_until_deadline at epoch {epoch}",
        );
    }
}

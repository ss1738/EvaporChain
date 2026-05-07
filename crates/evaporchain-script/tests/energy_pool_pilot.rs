//! Pilot — drive `contracts/evaporscript/energy_pool.es` through the full
//! parse → compile → VM execution pipeline.
//!
//! Asserts the lifecycle invariants the contract is supposed to enforce:
//!   1. `set_metadata` is one-shot and creator-only.
//!   2. Operations gated on `sealed == true` revert before sealing.
//!   3. Stake increases the caller's balance and the pool total; unstake
//!      decrements the caller's balance only (historical totals
//!      unchanged).
//!   4. Multiple distinct stakers bump `contributor_count`; a re-stake
//!      from the same address does NOT.
//!   5. `record_save` is creator-only and awards one guardian point per
//!      save, recording `last_distribution_epoch`.
//!   6. Lifecycle hooks (`on_grace`, `on_refresh`, `on_evaporate`) emit
//!      cleanly.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/energy_pool.es");

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
    let ast = parser::parse(SOURCE).unwrap_or_else(|e| panic!("EnergyPool failed to parse: {e:?}"));
    compiler::compile(&ast).unwrap_or_else(|e| panic!("EnergyPool failed to compile: {e:?}"))
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
    assert_eq!(bc.name, "EnergyPool");
    let public = [
        "set_metadata",
        "stake",
        "unstake",
        "record_save",
        "balance_of",
        "guardian_points_of",
        "pool_total",
        "pool_contributors",
    ];
    for m in &public {
        assert!(
            bc.methods.contains_key(*m),
            "method `{m}` missing from compiled bytecode"
        );
    }
}

#[test]
fn set_metadata_seals_and_locks() {
    let bc = compile_pilot();
    let creator = [0xAAu8; 32];

    let sealed = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("Forest Pool".to_string()),
            Value::Str("Refresh forest objects".to_string()),
            Value::U64(0),
        ],
        initial_state(&bc),
        &ctx(creator, creator, 100, 10_000),
    )
    .expect("seal must succeed for creator");
    assert!(
        sealed.events.iter().any(|e| e.contains("sealed")),
        "set_metadata must emit sealed event"
    );

    // Re-seal must revert.
    let err = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("Renamed".to_string()),
            Value::Str("rewrite".to_string()),
            Value::U64(1),
        ],
        sealed.state_changes,
        &ctx(creator, creator, 200, 9_000),
    )
    .expect_err("re-seal must revert");
    assert!(
        format!("{err:?}").contains("already sealed"),
        "re-seal revert reason wrong: {err:?}"
    );
}

#[test]
fn non_creator_cannot_seal() {
    let bc = compile_pilot();
    let creator = [0xAAu8; 32];
    let attacker = [0xBBu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("Hostile".to_string()),
            Value::Str("".to_string()),
            Value::U64(0),
        ],
        initial_state(&bc),
        &ctx(attacker, creator, 100, 10_000),
    )
    .expect_err("non-creator must revert");
    assert!(
        format!("{err:?}").contains("only creator"),
        "wrong revert reason: {err:?}"
    );
}

#[test]
fn unknown_strategy_rejected() {
    let bc = compile_pilot();
    let creator = [0xAAu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("X".to_string()),
            Value::Str("".to_string()),
            Value::U64(99),
        ],
        initial_state(&bc),
        &ctx(creator, creator, 100, 10_000),
    )
    .expect_err("strategy=99 must revert");
    assert!(
        format!("{err:?}").contains("unknown strategy"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn stake_before_seal_reverts() {
    let bc = compile_pilot();
    let staker = [0xCCu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "stake",
        vec![Value::U64(500)],
        initial_state(&bc),
        &ctx(staker, [0xAAu8; 32], 100, 10_000),
    )
    .expect_err("stake before seal must revert");
    assert!(
        format!("{err:?}").contains("not yet sealed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn stake_zero_rejected() {
    let bc = compile_pilot();
    let creator = [0xAAu8; 32];
    let staker = [0xCCu8; 32];

    let sealed = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("P".to_string()),
            Value::Str("".to_string()),
            Value::U64(0),
        ],
        initial_state(&bc),
        &ctx(creator, creator, 100, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "stake",
        vec![Value::U64(0)],
        sealed.state_changes,
        &ctx(staker, creator, 101, 10_000),
    )
    .expect_err("stake(0) must revert");
    assert!(
        format!("{err:?}").contains("must be positive"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn stake_unstake_round_trip() {
    let bc = compile_pilot();
    let creator = [0xAAu8; 32];
    let alice = [0xC1u8; 32];

    let sealed = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("P".to_string()),
            Value::Str("".to_string()),
            Value::U64(0),
        ],
        initial_state(&bc),
        &ctx(creator, creator, 100, 10_000),
    )
    .unwrap();

    let staked = EvaporVM::execute(
        &bc,
        "stake",
        vec![Value::U64(700)],
        sealed.state_changes,
        &ctx(alice, creator, 101, 10_000),
    )
    .expect("stake must succeed after seal");

    // Verify alice's balance is 700.
    let bal = EvaporVM::execute(
        &bc,
        "balance_of",
        vec![Value::Address(alice)],
        staked.state_changes.clone(),
        &ctx(alice, creator, 102, 10_000),
    )
    .unwrap();
    assert_eq!(bal.return_value, Value::U64(700));

    // pool_total cumulative.
    let total = EvaporVM::execute(
        &bc,
        "pool_total",
        vec![],
        staked.state_changes.clone(),
        &ctx(alice, creator, 102, 10_000),
    )
    .unwrap();
    assert_eq!(total.return_value, Value::U64(700));

    // Unstake 200, leaving 500. pool_total stays at 700 (cumulative).
    let unstaked = EvaporVM::execute(
        &bc,
        "unstake",
        vec![Value::U64(200)],
        staked.state_changes,
        &ctx(alice, creator, 103, 10_000),
    )
    .expect("unstake within balance must succeed");
    let bal_after = EvaporVM::execute(
        &bc,
        "balance_of",
        vec![Value::Address(alice)],
        unstaked.state_changes.clone(),
        &ctx(alice, creator, 104, 10_000),
    )
    .unwrap();
    assert_eq!(bal_after.return_value, Value::U64(500));

    let total_after = EvaporVM::execute(
        &bc,
        "pool_total",
        vec![],
        unstaked.state_changes.clone(),
        &ctx(alice, creator, 104, 10_000),
    )
    .unwrap();
    assert_eq!(
        total_after.return_value,
        Value::U64(700),
        "pool_total is cumulative; unstake must not decrement it"
    );

    // Over-unstake reverts.
    let err = EvaporVM::execute(
        &bc,
        "unstake",
        vec![Value::U64(9999)],
        unstaked.state_changes,
        &ctx(alice, creator, 105, 10_000),
    )
    .expect_err("over-unstake must revert");
    assert!(
        format!("{err:?}").contains("exceeds staked"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn contributor_count_only_increments_on_first_stake() {
    let bc = compile_pilot();
    let creator = [0xAAu8; 32];
    let alice = [0xC1u8; 32];
    let bob = [0xC2u8; 32];

    let sealed = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("P".to_string()),
            Value::Str("".to_string()),
            Value::U64(0),
        ],
        initial_state(&bc),
        &ctx(creator, creator, 100, 10_000),
    )
    .unwrap();

    let s1 = EvaporVM::execute(
        &bc,
        "stake",
        vec![Value::U64(100)],
        sealed.state_changes,
        &ctx(alice, creator, 101, 10_000),
    )
    .unwrap();

    let count_after_alice = EvaporVM::execute(
        &bc,
        "pool_contributors",
        vec![],
        s1.state_changes.clone(),
        &ctx(alice, creator, 102, 10_000),
    )
    .unwrap();
    assert_eq!(count_after_alice.return_value, Value::U64(1));

    // Alice stakes again — count must NOT increment.
    let s2 = EvaporVM::execute(
        &bc,
        "stake",
        vec![Value::U64(50)],
        s1.state_changes,
        &ctx(alice, creator, 103, 10_000),
    )
    .unwrap();
    let count_after_alice2 = EvaporVM::execute(
        &bc,
        "pool_contributors",
        vec![],
        s2.state_changes.clone(),
        &ctx(alice, creator, 104, 10_000),
    )
    .unwrap();
    assert_eq!(
        count_after_alice2.return_value,
        Value::U64(1),
        "re-stake from same address must not bump contributor_count"
    );

    // Bob's first stake — count goes to 2.
    let s3 = EvaporVM::execute(
        &bc,
        "stake",
        vec![Value::U64(300)],
        s2.state_changes,
        &ctx(bob, creator, 105, 10_000),
    )
    .unwrap();
    let count_after_bob = EvaporVM::execute(
        &bc,
        "pool_contributors",
        vec![],
        s3.state_changes,
        &ctx(bob, creator, 106, 10_000),
    )
    .unwrap();
    assert_eq!(count_after_bob.return_value, Value::U64(2));
}

#[test]
fn record_save_creator_only_and_awards_points() {
    let bc = compile_pilot();
    let creator = [0xAAu8; 32];
    let outsider = [0xBBu8; 32];

    let sealed = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("P".to_string()),
            Value::Str("".to_string()),
            Value::U64(0),
        ],
        initial_state(&bc),
        &ctx(creator, creator, 100, 10_000),
    )
    .unwrap();

    // Outsider attempting record_save reverts.
    let err = EvaporVM::execute(
        &bc,
        "record_save",
        vec![],
        sealed.state_changes.clone(),
        &ctx(outsider, creator, 105, 10_000),
    )
    .expect_err("non-creator record_save must revert");
    assert!(
        format!("{err:?}").contains("only coordinator"),
        "wrong revert: {err:?}"
    );

    // Creator records two saves; guardian_points and objects_saved both
    // climb to 2.
    let after1 = EvaporVM::execute(
        &bc,
        "record_save",
        vec![],
        sealed.state_changes,
        &ctx(creator, creator, 110, 10_000),
    )
    .unwrap();
    let after2 = EvaporVM::execute(
        &bc,
        "record_save",
        vec![],
        after1.state_changes,
        &ctx(creator, creator, 120, 10_000),
    )
    .unwrap();

    let pts = EvaporVM::execute(
        &bc,
        "guardian_points_of",
        vec![Value::Address(creator)],
        after2.state_changes,
        &ctx(creator, creator, 121, 10_000),
    )
    .unwrap();
    assert_eq!(pts.return_value, Value::U64(2));
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let creator = [0xAAu8; 32];
    let sealed = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("P".to_string()),
            Value::Str("".to_string()),
            Value::U64(0),
        ],
        initial_state(&bc),
        &ctx(creator, creator, 100, 10_000),
    )
    .unwrap();
    for hook in &["on_grace", "on_refresh", "on_evaporate"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            sealed.state_changes.clone(),
            &ctx(creator, creator, 500, 0),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        assert!(
            !r.events.is_empty(),
            "hook {hook} must emit at least one event"
        );
    }
}

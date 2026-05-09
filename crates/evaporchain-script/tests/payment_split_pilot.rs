//! Pilot — drive `contracts/evaporscript/payment_split.es` through the
//! full parse → compile → VM execution pipeline.
//!
//! Second worked-example behavioural pilot for the seed-12 stdlib (after
//! `dead_man_switch_pilot.rs`). PaymentSplit is the math-heavy contract
//! — every claim runs `(total_deposited * share_bps) / 10000`, so this
//! pilot doubles as a regression on EvaporScript's arithmetic ops under
//! the pull-payment pattern.
//!
//! Pins the documented invariants:
//!   1. `add_recipient` is deployer-only, accumulates `total_bps`,
//!      blocks duplicates, blocks overshoots past 10_000.
//!   2. `seal` requires `total_bps == 10_000` exactly (no dust).
//!   3. `deposit` is open and bumps cumulative `total_deposited`.
//!   4. `claim` is recipient-only and uses the cumulative-basis math:
//!      `owed = total_deposited * bps / 10_000 - already_claimed`.
//!   5. Consecutive claims after additional deposits return only the
//!      delta — the cumulative tracker is monotonic and never refunds.
//!   6. `pending_of` and `entitlement_of` views compute the same math
//!      without mutating state.
//!   7. `on_evaporate` flips the forfeit_signaled flag and stamps
//!      `unclaimed_at_evaporate`.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/payment_split.es");

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
        .unwrap_or_else(|e| panic!("PaymentSplit failed to parse: {e:?}"));
    compiler::compile(&ast)
        .unwrap_or_else(|e| panic!("PaymentSplit failed to compile: {e:?}"))
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

/// Helper — deploy + add 3 recipients with given bps + seal. Returns the
/// final post-seal state map for downstream tests.
fn deploy_and_seal_three(
    bc: &EvaporBytecode,
    deployer: [u8; 32],
    a: [u8; 32],
    b: [u8; 32],
    c: [u8; 32],
    bps: (u64, u64, u64),
) -> HashMap<String, Value> {
    let mut s = initial_state(bc);
    let r = EvaporVM::execute(
        bc,
        "add_recipient",
        vec![Value::Address(a), Value::U64(bps.0)],
        s.clone(),
        &ctx(deployer, deployer, 100, 10_000),
    )
    .expect("add A");
    s = r.state_changes;
    let r = EvaporVM::execute(
        bc,
        "add_recipient",
        vec![Value::Address(b), Value::U64(bps.1)],
        s.clone(),
        &ctx(deployer, deployer, 101, 10_000),
    )
    .expect("add B");
    s = r.state_changes;
    let r = EvaporVM::execute(
        bc,
        "add_recipient",
        vec![Value::Address(c), Value::U64(bps.2)],
        s.clone(),
        &ctx(deployer, deployer, 102, 10_000),
    )
    .expect("add C");
    s = r.state_changes;
    let r = EvaporVM::execute(
        bc,
        "seal",
        vec![],
        s,
        &ctx(deployer, deployer, 103, 10_000),
    )
    .expect("seal");
    r.state_changes
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "PaymentSplit");
    let public = [
        "add_recipient",
        "seal",
        "deposit",
        "claim",
        "entitlement_of",
        "pending_of",
        "share_of",
        "total_pool",
        "recipients",
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
fn add_recipient_accumulates_and_blocks_overshoot() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let a = [0x11u8; 32];
    let b = [0x22u8; 32];

    // 6000 bps for A.
    let r1 = EvaporVM::execute(
        &bc,
        "add_recipient",
        vec![Value::Address(a), Value::U64(6000)],
        initial_state(&bc),
        &ctx(deployer, deployer, 100, 10_000),
    )
    .expect("first add");

    // 5000 bps for B would push total to 11000 — must reject.
    let err = EvaporVM::execute(
        &bc,
        "add_recipient",
        vec![Value::Address(b), Value::U64(5000)],
        r1.state_changes.clone(),
        &ctx(deployer, deployer, 101, 10_000),
    )
    .expect_err("overshoot must reject");
    assert!(
        format!("{err:?}").contains("total bps would exceed 10000"),
        "wrong revert: {err:?}"
    );

    // 4000 bps for B brings total to exactly 10000.
    let r2 = EvaporVM::execute(
        &bc,
        "add_recipient",
        vec![Value::Address(b), Value::U64(4000)],
        r1.state_changes,
        &ctx(deployer, deployer, 101, 10_000),
    )
    .expect("ok-fit add");
    let count = EvaporVM::execute(
        &bc,
        "recipients",
        vec![],
        r2.state_changes,
        &ctx(deployer, deployer, 102, 10_000),
    )
    .unwrap();
    assert_eq!(count.return_value, Value::U64(2));
}

#[test]
fn duplicate_recipient_rejects() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let a = [0x11u8; 32];
    let r1 = EvaporVM::execute(
        &bc,
        "add_recipient",
        vec![Value::Address(a), Value::U64(5000)],
        initial_state(&bc),
        &ctx(deployer, deployer, 100, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "add_recipient",
        vec![Value::Address(a), Value::U64(3000)],
        r1.state_changes,
        &ctx(deployer, deployer, 101, 10_000),
    )
    .expect_err("dup add must reject");
    assert!(
        format!("{err:?}").contains("recipient already added"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn seal_requires_exact_10000_bps() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let a = [0x11u8; 32];
    // Only add 5000 bps then try to seal — must reject (under-allocation).
    let r = EvaporVM::execute(
        &bc,
        "add_recipient",
        vec![Value::Address(a), Value::U64(5000)],
        initial_state(&bc),
        &ctx(deployer, deployer, 100, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "seal",
        vec![],
        r.state_changes,
        &ctx(deployer, deployer, 101, 10_000),
    )
    .expect_err("under-alloc seal must reject");
    assert!(
        format!("{err:?}").contains("total bps must equal 10000"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn deposit_before_seal_rejects() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let depositor = [0x99u8; 32];
    let err = EvaporVM::execute(
        &bc,
        "deposit",
        vec![Value::U64(1_000)],
        initial_state(&bc),
        &ctx(depositor, deployer, 100, 10_000),
    )
    .expect_err("deposit pre-seal must reject");
    assert!(
        format!("{err:?}").contains("not yet sealed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn deposit_accumulates_total_pool() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let a = [0x11u8; 32];
    let b = [0x22u8; 32];
    let c = [0x33u8; 32];
    let depositor = [0x99u8; 32];

    let s = deploy_and_seal_three(&bc, deployer, a, b, c, (5000, 3000, 2000));

    let r1 = EvaporVM::execute(
        &bc,
        "deposit",
        vec![Value::U64(10_000)],
        s,
        &ctx(depositor, deployer, 200, 10_000),
    )
    .unwrap();
    let r2 = EvaporVM::execute(
        &bc,
        "deposit",
        vec![Value::U64(5_000)],
        r1.state_changes,
        &ctx(depositor, deployer, 201, 10_000),
    )
    .unwrap();
    let total = EvaporVM::execute(
        &bc,
        "total_pool",
        vec![],
        r2.state_changes,
        &ctx(depositor, deployer, 202, 10_000),
    )
    .unwrap();
    assert_eq!(total.return_value, Value::U64(15_000));
}

#[test]
fn claim_returns_proportional_share_math() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let a = [0x11u8; 32]; // 5000 bps = 50%
    let b = [0x22u8; 32]; // 3000 bps = 30%
    let c = [0x33u8; 32]; // 2000 bps = 20%
    let depositor = [0x99u8; 32];

    let s = deploy_and_seal_three(&bc, deployer, a, b, c, (5000, 3000, 2000));

    // Deposit 10_000. A's owed = 5000, B's owed = 3000, C's owed = 2000.
    let after_deposit = EvaporVM::execute(
        &bc,
        "deposit",
        vec![Value::U64(10_000)],
        s,
        &ctx(depositor, deployer, 200, 10_000),
    )
    .unwrap();

    // A claims — must return 5_000.
    let claim_a = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        after_deposit.state_changes.clone(),
        &ctx(a, deployer, 201, 10_000),
    )
    .expect("A claim must succeed");
    assert_eq!(
        claim_a.return_value,
        Value::U64(5_000),
        "A's first claim must equal 50% of pool"
    );

    // B claims from the SAME state — uses the same total_deposited.
    let claim_b = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        after_deposit.state_changes,
        &ctx(b, deployer, 202, 10_000),
    )
    .expect("B claim must succeed");
    assert_eq!(
        claim_b.return_value,
        Value::U64(3_000),
        "B's first claim must equal 30% of pool"
    );
}

#[test]
fn cumulative_claim_returns_only_the_delta() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let a = [0x11u8; 32]; // 5000 bps
    let b = [0x22u8; 32];
    let c = [0x33u8; 32];
    let depositor = [0x99u8; 32];

    let s = deploy_and_seal_three(&bc, deployer, a, b, c, (5000, 3000, 2000));

    // Round 1: deposit 10_000, A claims 5_000.
    let after_d1 = EvaporVM::execute(
        &bc,
        "deposit",
        vec![Value::U64(10_000)],
        s,
        &ctx(depositor, deployer, 200, 10_000),
    )
    .unwrap();
    let claim1 = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        after_d1.state_changes,
        &ctx(a, deployer, 201, 10_000),
    )
    .unwrap();
    assert_eq!(claim1.return_value, Value::U64(5_000));

    // Round 2: deposit ANOTHER 10_000 (total 20_000). A's cumulative
    // entitlement is now 10_000; A has already claimed 5_000; delta = 5_000.
    let after_d2 = EvaporVM::execute(
        &bc,
        "deposit",
        vec![Value::U64(10_000)],
        claim1.state_changes,
        &ctx(depositor, deployer, 210, 10_000),
    )
    .unwrap();
    let claim2 = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        after_d2.state_changes,
        &ctx(a, deployer, 211, 10_000),
    )
    .unwrap();
    assert_eq!(
        claim2.return_value,
        Value::U64(5_000),
        "second claim must return only the delta, not the full cumulative"
    );

    // Round 3: claim again with no new deposit — must revert ("nothing
    // to claim") since cumulative entitlement = already_claimed.
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        claim2.state_changes,
        &ctx(a, deployer, 212, 10_000),
    )
    .expect_err("re-claim with no new deposit must revert");
    assert!(
        format!("{err:?}").contains("nothing to claim"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_recipient_claim_rejects() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let a = [0x11u8; 32];
    let b = [0x22u8; 32];
    let c = [0x33u8; 32];
    let attacker = [0xCCu8; 32];
    let depositor = [0x99u8; 32];

    let s = deploy_and_seal_three(&bc, deployer, a, b, c, (5000, 3000, 2000));
    let after_deposit = EvaporVM::execute(
        &bc,
        "deposit",
        vec![Value::U64(10_000)],
        s,
        &ctx(depositor, deployer, 200, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "claim",
        vec![],
        after_deposit.state_changes,
        &ctx(attacker, deployer, 201, 10_000),
    )
    .expect_err("non-recipient claim must revert");
    assert!(
        format!("{err:?}").contains("not a recipient"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn pending_and_entitlement_views_match_math() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let a = [0x11u8; 32]; // 7500 bps = 75%
    let b = [0x22u8; 32]; // 1500 bps = 15%
    let c = [0x33u8; 32]; // 1000 bps = 10%
    let depositor = [0x99u8; 32];

    let s = deploy_and_seal_three(&bc, deployer, a, b, c, (7500, 1500, 1000));
    let after = EvaporVM::execute(
        &bc,
        "deposit",
        vec![Value::U64(20_000)],
        s,
        &ctx(depositor, deployer, 200, 10_000),
    )
    .unwrap();

    // Entitlement_of is gross (cumulative).
    let ent_a = EvaporVM::execute(
        &bc,
        "entitlement_of",
        vec![Value::Address(a)],
        after.state_changes.clone(),
        &ctx(depositor, deployer, 201, 10_000),
    )
    .unwrap();
    assert_eq!(ent_a.return_value, Value::U64(15_000));

    // Pending_of equals entitlement when nothing has been claimed.
    let pen_a = EvaporVM::execute(
        &bc,
        "pending_of",
        vec![Value::Address(a)],
        after.state_changes.clone(),
        &ctx(depositor, deployer, 202, 10_000),
    )
    .unwrap();
    assert_eq!(pen_a.return_value, Value::U64(15_000));

    // Non-recipient gets 0 from both views (no revert).
    let attacker = [0xCCu8; 32];
    let ent_x = EvaporVM::execute(
        &bc,
        "entitlement_of",
        vec![Value::Address(attacker)],
        after.state_changes.clone(),
        &ctx(depositor, deployer, 203, 10_000),
    )
    .unwrap();
    assert_eq!(ent_x.return_value, Value::U64(0));
    let pen_x = EvaporVM::execute(
        &bc,
        "pending_of",
        vec![Value::Address(attacker)],
        after.state_changes,
        &ctx(depositor, deployer, 204, 10_000),
    )
    .unwrap();
    assert_eq!(pen_x.return_value, Value::U64(0));
}

#[test]
fn on_evaporate_signals_forfeit() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let a = [0x11u8; 32];
    let b = [0x22u8; 32];
    let c = [0x33u8; 32];
    let depositor = [0x99u8; 32];

    let s = deploy_and_seal_three(&bc, deployer, a, b, c, (5000, 3000, 2000));
    let after_deposit = EvaporVM::execute(
        &bc,
        "deposit",
        vec![Value::U64(8_000)],
        s,
        &ctx(depositor, deployer, 200, 10_000),
    )
    .unwrap();

    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        after_deposit.state_changes,
        &ctx(deployer, deployer, 5_000, 0),
    )
    .expect("on_evaporate must execute");
    assert!(
        evap.events.iter().any(|e| e.contains("evaporated")),
        "evaporate must emit forfeit event"
    );
    // Spot-check that `unclaimed_at_evaporate` got stamped.
    if let Some(Value::U64(v)) = evap.state_changes.get("unclaimed_at_evaporate") {
        assert_eq!(
            *v, 8_000,
            "unclaimed_at_evaporate must capture total_deposited at death"
        );
    } else {
        panic!("unclaimed_at_evaporate not set as U64");
    }
    if let Some(Value::Bool(b)) = evap.state_changes.get("forfeit_signaled") {
        assert!(*b, "forfeit_signaled must flip true on evaporate");
    } else {
        panic!("forfeit_signaled not set as Bool");
    }
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let a = [0x11u8; 32];
    let b = [0x22u8; 32];
    let c = [0x33u8; 32];
    let s = deploy_and_seal_three(&bc, deployer, a, b, c, (5000, 3000, 2000));
    for hook in &["on_grace", "on_refresh"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            s.clone(),
            &ctx(deployer, deployer, 500, 100),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        let _ = r.events;
    }
}

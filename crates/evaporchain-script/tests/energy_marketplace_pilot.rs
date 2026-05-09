//! Pilot — drive `contracts/evaporscript/energy_marketplace.es` through
//! the full parse → compile → VM execution pipeline.
//!
//! Twelfth and final worked-example behavioural pilot for the seed-12
//! stdlib. EnergyMarketplace is the meta-marketplace — energy itself is
//! the commodity. Listings expire when the marketplace evaporates;
//! mortal liquidity, no eternal order book.
//!
//! Pins:
//!   1. set_market one-shot + operator-only.
//!   2. list updates in place; first-time-from-seller bumps
//!      listing_count, replacement does not.
//!   3. buy decrements available_units, bumps cumulative_units_sold +
//!      cumulative_evap_volume + trade_count; full-fill removes
//!      listing.
//!   4. buy on insufficient units rejects.
//!   5. buy on inactive seller rejects.
//!   6. cancel pulls the seller's listing; subsequent buy rejects.
//!   7. on_evaporate flips closed=true; subsequent operations rejected.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/energy_marketplace.es");

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
        .unwrap_or_else(|e| panic!("EnergyMarketplace failed to parse: {e:?}"));
    compiler::compile(&ast)
        .unwrap_or_else(|e| panic!("EnergyMarketplace failed to compile: {e:?}"))
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

fn configure(bc: &EvaporBytecode, operator: [u8; 32], name: &str) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "set_market",
        vec![Value::Str(name.to_string())],
        initial_state(bc),
        &ctx(operator, operator, 100, 10_000),
    )
    .expect("set_market must succeed");
    r.state_changes
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "EnergyMarketplace");
    let public = [
        "set_market",
        "list",
        "buy",
        "cancel",
        "market_label",
        "active_listings",
        "units_sold_total",
        "evap_volume_total",
        "trades_total",
        "listing_units_of",
        "listing_price_of",
        "has_listing",
        "is_open",
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
fn set_market_one_shot_and_non_operator_rejects() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let attacker = [0xCCu8; 32];

    let configured = configure(&bc, operator, "EVAP-energy-mkt");

    let label = EvaporVM::execute(
        &bc,
        "market_label",
        vec![],
        configured.clone(),
        &ctx(operator, operator, 101, 10_000),
    )
    .unwrap();
    assert_eq!(label.return_value, Value::Str("EVAP-energy-mkt".to_string()));

    // Re-set rejects.
    let err = EvaporVM::execute(
        &bc,
        "set_market",
        vec![Value::Str("X".to_string())],
        configured,
        &ctx(operator, operator, 102, 10_000),
    )
    .expect_err("re-set must reject");
    assert!(
        format!("{err:?}").contains("already configured"),
        "wrong revert: {err:?}"
    );

    // Non-operator set rejects.
    let err = EvaporVM::execute(
        &bc,
        "set_market",
        vec![Value::Str("X".to_string())],
        initial_state(&bc),
        &ctx(attacker, operator, 102, 10_000),
    )
    .expect_err("non-operator set must reject");
    assert!(
        format!("{err:?}").contains("only operator"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn list_first_time_bumps_count_relist_does_not() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let configured = configure(&bc, operator, "x");

    let l1 = EvaporVM::execute(
        &bc,
        "list",
        vec![Value::U64(100), Value::U64(5)],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    let count1 = EvaporVM::execute(
        &bc,
        "active_listings",
        vec![],
        l1.state_changes.clone(),
        &ctx(alice, operator, 201, 10_000),
    )
    .unwrap();
    assert_eq!(count1.return_value, Value::U64(1));

    // Alice updates her listing — count stays.
    let l2 = EvaporVM::execute(
        &bc,
        "list",
        vec![Value::U64(200), Value::U64(7)],
        l1.state_changes,
        &ctx(alice, operator, 202, 10_000),
    )
    .unwrap();
    let count2 = EvaporVM::execute(
        &bc,
        "active_listings",
        vec![],
        l2.state_changes.clone(),
        &ctx(alice, operator, 203, 10_000),
    )
    .unwrap();
    assert_eq!(
        count2.return_value,
        Value::U64(1),
        "relist must not bump count"
    );
    let units = EvaporVM::execute(
        &bc,
        "listing_units_of",
        vec![Value::Address(alice)],
        l2.state_changes.clone(),
        &ctx(alice, operator, 204, 10_000),
    )
    .unwrap();
    assert_eq!(units.return_value, Value::U64(200));
    let price = EvaporVM::execute(
        &bc,
        "listing_price_of",
        vec![Value::Address(alice)],
        l2.state_changes,
        &ctx(alice, operator, 205, 10_000),
    )
    .unwrap();
    assert_eq!(price.return_value, Value::U64(7));
}

#[test]
fn buy_partial_fill_decrements_units() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let buyer = [0xCCu8; 32];
    let configured = configure(&bc, operator, "x");
    let listed = EvaporVM::execute(
        &bc,
        "list",
        vec![Value::U64(100), Value::U64(5)],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    let bought = EvaporVM::execute(
        &bc,
        "buy",
        vec![Value::Address(alice), Value::U64(30)],
        listed.state_changes,
        &ctx(buyer, operator, 300, 10_000),
    )
    .unwrap();
    // 30 * 5 = 150 EVP cost.
    assert_eq!(bought.return_value, Value::U64(150));

    let units = EvaporVM::execute(
        &bc,
        "listing_units_of",
        vec![Value::Address(alice)],
        bought.state_changes.clone(),
        &ctx(buyer, operator, 301, 10_000),
    )
    .unwrap();
    assert_eq!(units.return_value, Value::U64(70));

    // Listing still active (partial fill).
    let active = EvaporVM::execute(
        &bc,
        "has_listing",
        vec![Value::Address(alice)],
        bought.state_changes.clone(),
        &ctx(buyer, operator, 302, 10_000),
    )
    .unwrap();
    assert_eq!(active.return_value, Value::Bool(true));

    // Aggregate metrics.
    let sold = EvaporVM::execute(
        &bc,
        "units_sold_total",
        vec![],
        bought.state_changes.clone(),
        &ctx(buyer, operator, 303, 10_000),
    )
    .unwrap();
    assert_eq!(sold.return_value, Value::U64(30));
    let vol = EvaporVM::execute(
        &bc,
        "evap_volume_total",
        vec![],
        bought.state_changes.clone(),
        &ctx(buyer, operator, 304, 10_000),
    )
    .unwrap();
    assert_eq!(vol.return_value, Value::U64(150));
    let trades = EvaporVM::execute(
        &bc,
        "trades_total",
        vec![],
        bought.state_changes,
        &ctx(buyer, operator, 305, 10_000),
    )
    .unwrap();
    assert_eq!(trades.return_value, Value::U64(1));
}

#[test]
fn buy_full_fill_removes_listing_and_decrements_count() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let buyer = [0xCCu8; 32];
    let configured = configure(&bc, operator, "x");
    let listed = EvaporVM::execute(
        &bc,
        "list",
        vec![Value::U64(50), Value::U64(2)],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    let bought = EvaporVM::execute(
        &bc,
        "buy",
        vec![Value::Address(alice), Value::U64(50)],
        listed.state_changes,
        &ctx(buyer, operator, 300, 10_000),
    )
    .unwrap();
    let active = EvaporVM::execute(
        &bc,
        "has_listing",
        vec![Value::Address(alice)],
        bought.state_changes.clone(),
        &ctx(buyer, operator, 301, 10_000),
    )
    .unwrap();
    assert_eq!(active.return_value, Value::Bool(false));
    let count = EvaporVM::execute(
        &bc,
        "active_listings",
        vec![],
        bought.state_changes,
        &ctx(buyer, operator, 302, 10_000),
    )
    .unwrap();
    assert_eq!(count.return_value, Value::U64(0));
}

#[test]
fn buy_insufficient_units_rejects() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let buyer = [0xCCu8; 32];
    let configured = configure(&bc, operator, "x");
    let listed = EvaporVM::execute(
        &bc,
        "list",
        vec![Value::U64(50), Value::U64(2)],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "buy",
        vec![Value::Address(alice), Value::U64(100)],
        listed.state_changes,
        &ctx(buyer, operator, 300, 10_000),
    )
    .expect_err("over-buy must reject");
    assert!(
        format!("{err:?}").contains("insufficient units"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn buy_on_inactive_seller_rejects() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let buyer = [0xCCu8; 32];
    let configured = configure(&bc, operator, "x");
    // Alice never listed.
    let err = EvaporVM::execute(
        &bc,
        "buy",
        vec![Value::Address(alice), Value::U64(10)],
        configured,
        &ctx(buyer, operator, 300, 10_000),
    )
    .expect_err("buy from non-lister must reject");
    assert!(
        format!("{err:?}").contains("no active listing"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn cancel_removes_listing_and_blocks_buy() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let buyer = [0xCCu8; 32];
    let configured = configure(&bc, operator, "x");
    let listed = EvaporVM::execute(
        &bc,
        "list",
        vec![Value::U64(100), Value::U64(5)],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();
    let cancelled = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        listed.state_changes,
        &ctx(alice, operator, 210, 10_000),
    )
    .unwrap();
    let active = EvaporVM::execute(
        &bc,
        "has_listing",
        vec![Value::Address(alice)],
        cancelled.state_changes.clone(),
        &ctx(alice, operator, 211, 10_000),
    )
    .unwrap();
    assert_eq!(active.return_value, Value::Bool(false));

    let err = EvaporVM::execute(
        &bc,
        "buy",
        vec![Value::Address(alice), Value::U64(10)],
        cancelled.state_changes,
        &ctx(buyer, operator, 300, 10_000),
    )
    .expect_err("buy on cancelled listing must reject");
    assert!(
        format!("{err:?}").contains("no active listing"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn cancel_without_listing_rejects() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let configured = configure(&bc, operator, "x");
    let err = EvaporVM::execute(
        &bc,
        "cancel",
        vec![],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .expect_err("cancel without listing must reject");
    assert!(
        format!("{err:?}").contains("no active listing"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn on_evaporate_closes_marketplace() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let configured = configure(&bc, operator, "x");
    let listed = EvaporVM::execute(
        &bc,
        "list",
        vec![Value::U64(100), Value::U64(5)],
        configured,
        &ctx(alice, operator, 200, 10_000),
    )
    .unwrap();

    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        listed.state_changes,
        &ctx(operator, operator, 9_000, 0),
    )
    .unwrap();

    let open = EvaporVM::execute(
        &bc,
        "is_open",
        vec![],
        evap.state_changes.clone(),
        &ctx(operator, operator, 9_001, 0),
    )
    .unwrap();
    assert_eq!(open.return_value, Value::Bool(false));

    // Subsequent list rejects (closed).
    let err = EvaporVM::execute(
        &bc,
        "list",
        vec![Value::U64(50), Value::U64(2)],
        evap.state_changes,
        &ctx(alice, operator, 9_010, 0),
    )
    .expect_err("post-evap list must reject");
    assert!(
        format!("{err:?}").contains("marketplace closed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let operator = [0xAAu8; 32];
    let configured = configure(&bc, operator, "x");
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

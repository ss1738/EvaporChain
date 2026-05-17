//! Pilot — drive `contracts/evaporscript/sealed_bid_auction.es` through
//! the full parse → compile → VM execution pipeline.
//!
//! Ninth worked-example behavioural pilot. SealedBidAuction's doctrine
//! moment: bid weight decays during reveal — early reveal wins ties.
//! The contract's energy budget is the entire auction window; an
//! evaporated auction without settlement is void by physics.
//!
//! Pins:
//!   1. Phase machine: 0=COMMIT, 1=REVEAL, 2=SETTLE, 3=CLOSED.
//!      Phases advance monotonically; rewinds reject.
//!   2. set_metadata is one-shot + seller-only.
//!   3. commit only in phase 0; reveal only in phase 1; settle only in 2.
//!   4. reveal requires a prior commit + nominal >= reserve_price +
//!      effective <= nominal; one reveal per address.
//!   5. record_winner is seller-only; verifies winner has revealed +
//!      effective_bid matches.
//!   6. on_evaporate without settle emits void event.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/sealed_bid_auction.es");

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
        .unwrap_or_else(|e| panic!("SealedBidAuction failed to parse: {e:?}"));
    compiler::compile(&ast)
        .unwrap_or_else(|e| panic!("SealedBidAuction failed to compile: {e:?}"))
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

fn seal(
    bc: &EvaporBytecode,
    seller: [u8; 32],
    item: &str,
    reserve: u64,
) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "set_metadata",
        vec![Value::Str(item.to_string()), Value::U64(reserve)],
        initial_state(bc),
        &ctx(seller, seller, 100, 10_000),
    )
    .expect("set_metadata must succeed");
    r.state_changes
}

fn advance(
    bc: &EvaporBytecode,
    state: HashMap<String, Value>,
    seller: [u8; 32],
    next: u64,
    epoch: u64,
) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "set_phase",
        vec![Value::U64(next)],
        state,
        &ctx(seller, seller, epoch, 10_000),
    )
    .expect("set_phase must succeed");
    r.state_changes
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "SealedBidAuction");
    let public = [
        "set_metadata",
        "set_phase",
        "commit",
        "reveal",
        "record_winner",
        "current_phase",
        "nominal_bid_of",
        "effective_bid_of",
        "winner_of",
        "is_settled",
        "commits_received",
        "reveals_received",
        "committed_hash_of",
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
fn full_round_trip_commit_reveal_settle() {
    let bc = compile_pilot();
    let seller = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bob = [0xB2u8; 32];

    let s0 = seal(&bc, seller, "rare-asset-1", 100);

    // Phase 0 (default after seal). Both bidders commit.
    let after_a_commit = EvaporVM::execute(
        &bc,
        "commit",
        vec![Value::Str("alice-commit-hash".to_string())],
        s0,
        &ctx(alice, seller, 200, 10_000),
    )
    .unwrap();
    let after_b_commit = EvaporVM::execute(
        &bc,
        "commit",
        vec![Value::Str("bob-commit-hash".to_string())],
        after_a_commit.state_changes,
        &ctx(bob, seller, 201, 10_000),
    )
    .unwrap();
    let cc = EvaporVM::execute(
        &bc,
        "commits_received",
        vec![],
        after_b_commit.state_changes.clone(),
        &ctx(seller, seller, 202, 10_000),
    )
    .unwrap();
    assert_eq!(cc.return_value, Value::U64(2));

    // Advance to REVEAL.
    let s1 = advance(&bc, after_b_commit.state_changes, seller, 1, 300);

    // Alice reveals nominal=500, effective=500 (full strength).
    // Must supply the exact hash she committed (SBA-1 binding check).
    let after_a_reveal = EvaporVM::execute(
        &bc,
        "reveal",
        vec![Value::U64(500), Value::U64(500), Value::Str("alice-commit-hash".to_string())],
        s1,
        &ctx(alice, seller, 310, 10_000),
    )
    .unwrap();
    // Bob reveals nominal=600, effective=400 (decayed).
    let after_b_reveal = EvaporVM::execute(
        &bc,
        "reveal",
        vec![Value::U64(600), Value::U64(400), Value::Str("bob-commit-hash".to_string())],
        after_a_reveal.state_changes,
        &ctx(bob, seller, 320, 10_000),
    )
    .unwrap();
    let rc = EvaporVM::execute(
        &bc,
        "reveals_received",
        vec![],
        after_b_reveal.state_changes.clone(),
        &ctx(seller, seller, 321, 10_000),
    )
    .unwrap();
    assert_eq!(rc.return_value, Value::U64(2));

    // Advance to SETTLE.
    let s2 = advance(&bc, after_b_reveal.state_changes, seller, 2, 400);

    // Alice's effective (500) > Bob's effective (400) — Alice wins.
    let settled = EvaporVM::execute(
        &bc,
        "record_winner",
        vec![Value::Address(alice), Value::U64(500)],
        s2,
        &ctx(seller, seller, 410, 10_000),
    )
    .unwrap();

    let winner = EvaporVM::execute(
        &bc,
        "winner_of",
        vec![],
        settled.state_changes.clone(),
        &ctx(seller, seller, 411, 10_000),
    )
    .unwrap();
    assert_eq!(winner.return_value, Value::Address(alice));

    let settled_flag = EvaporVM::execute(
        &bc,
        "is_settled",
        vec![],
        settled.state_changes.clone(),
        &ctx(seller, seller, 412, 10_000),
    )
    .unwrap();
    assert_eq!(settled_flag.return_value, Value::Bool(true));

    let phase = EvaporVM::execute(
        &bc,
        "current_phase",
        vec![],
        settled.state_changes,
        &ctx(seller, seller, 413, 10_000),
    )
    .unwrap();
    // record_winner advances to phase 3 (CLOSED).
    assert_eq!(phase.return_value, Value::U64(3));
}

#[test]
fn phases_advance_monotonically_only() {
    let bc = compile_pilot();
    let seller = [0xAAu8; 32];
    let s0 = seal(&bc, seller, "x", 100);
    let s1 = advance(&bc, s0, seller, 1, 200);

    // Try to rewind to phase 0 — must reject.
    let err = EvaporVM::execute(
        &bc,
        "set_phase",
        vec![Value::U64(0)],
        s1.clone(),
        &ctx(seller, seller, 201, 10_000),
    )
    .expect_err("phase rewind must reject");
    assert!(
        format!("{err:?}").contains("phase only advances"),
        "wrong revert: {err:?}"
    );

    // Phase 4 (out of range) rejects.
    let err = EvaporVM::execute(
        &bc,
        "set_phase",
        vec![Value::U64(4)],
        s1,
        &ctx(seller, seller, 202, 10_000),
    )
    .expect_err("phase > 3 must reject");
    assert!(
        format!("{err:?}").contains("max phase is 3"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn commit_outside_phase_zero_rejects() {
    let bc = compile_pilot();
    let seller = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let s0 = seal(&bc, seller, "x", 100);
    let s1 = advance(&bc, s0, seller, 1, 200);
    let err = EvaporVM::execute(
        &bc,
        "commit",
        vec![Value::Str("late-commit".to_string())],
        s1,
        &ctx(alice, seller, 210, 10_000),
    )
    .expect_err("commit in phase 1 must reject");
    assert!(
        format!("{err:?}").contains("not in COMMIT"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn reveal_without_commit_rejects() {
    let bc = compile_pilot();
    let seller = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let s0 = seal(&bc, seller, "x", 100);
    // Skip commit; advance to reveal.
    let s1 = advance(&bc, s0, seller, 1, 200);
    let err = EvaporVM::execute(
        &bc,
        "reveal",
        vec![Value::U64(500), Value::U64(500), Value::Str("any".to_string())],
        s1,
        &ctx(alice, seller, 210, 10_000),
    )
    .expect_err("reveal without commit must reject");
    assert!(
        format!("{err:?}").contains("no commit on file"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn reveal_below_reserve_rejects() {
    let bc = compile_pilot();
    let seller = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let s0 = seal(&bc, seller, "x", 100);
    let s_committed = EvaporVM::execute(
        &bc,
        "commit",
        vec![Value::Str("hash".to_string())],
        s0,
        &ctx(alice, seller, 200, 10_000),
    )
    .unwrap();
    let s1 = advance(&bc, s_committed.state_changes, seller, 1, 300);
    let err = EvaporVM::execute(
        &bc,
        "reveal",
        vec![Value::U64(50), Value::U64(50), Value::Str("hash".to_string())],
        s1,
        &ctx(alice, seller, 310, 10_000),
    )
    .expect_err("reveal below reserve must reject");
    assert!(
        format!("{err:?}").contains("below reserve"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn reveal_effective_exceeds_nominal_rejects() {
    let bc = compile_pilot();
    let seller = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let s0 = seal(&bc, seller, "x", 100);
    let s_committed = EvaporVM::execute(
        &bc,
        "commit",
        vec![Value::Str("hash".to_string())],
        s0,
        &ctx(alice, seller, 200, 10_000),
    )
    .unwrap();
    let s1 = advance(&bc, s_committed.state_changes, seller, 1, 300);
    let err = EvaporVM::execute(
        &bc,
        "reveal",
        vec![Value::U64(500), Value::U64(600), Value::Str("hash".to_string())],
        s1,
        &ctx(alice, seller, 310, 10_000),
    )
    .expect_err("effective > nominal must reject");
    assert!(
        format!("{err:?}").contains("effective cannot exceed nominal"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn double_reveal_rejects() {
    let bc = compile_pilot();
    let seller = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let s0 = seal(&bc, seller, "x", 100);
    let s_committed = EvaporVM::execute(
        &bc,
        "commit",
        vec![Value::Str("hash".to_string())],
        s0,
        &ctx(alice, seller, 200, 10_000),
    )
    .unwrap();
    let s1 = advance(&bc, s_committed.state_changes, seller, 1, 300);
    let s_revealed = EvaporVM::execute(
        &bc,
        "reveal",
        vec![Value::U64(500), Value::U64(500), Value::Str("hash".to_string())],
        s1,
        &ctx(alice, seller, 310, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "reveal",
        vec![Value::U64(600), Value::U64(600), Value::Str("hash".to_string())],
        s_revealed.state_changes,
        &ctx(alice, seller, 311, 10_000),
    )
    .expect_err("double reveal must reject");
    assert!(
        format!("{err:?}").contains("already revealed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn settle_without_revealed_winner_rejects() {
    let bc = compile_pilot();
    let seller = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let s0 = seal(&bc, seller, "x", 100);
    let s1 = advance(&bc, s0, seller, 1, 300);
    let s2 = advance(&bc, s1, seller, 2, 400);
    // Alice never revealed.
    let err = EvaporVM::execute(
        &bc,
        "record_winner",
        vec![Value::Address(alice), Value::U64(500)],
        s2,
        &ctx(seller, seller, 410, 10_000),
    )
    .expect_err("settle on never-revealed winner must reject");
    assert!(
        format!("{err:?}").contains("did not reveal"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn settle_with_mismatched_effective_rejects() {
    let bc = compile_pilot();
    let seller = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let s0 = seal(&bc, seller, "x", 100);
    let s_committed = EvaporVM::execute(
        &bc,
        "commit",
        vec![Value::Str("hash".to_string())],
        s0,
        &ctx(alice, seller, 200, 10_000),
    )
    .unwrap();
    let s1 = advance(&bc, s_committed.state_changes, seller, 1, 300);
    let s_revealed = EvaporVM::execute(
        &bc,
        "reveal",
        vec![Value::U64(500), Value::U64(500), Value::Str("hash".to_string())],
        s1,
        &ctx(alice, seller, 310, 10_000),
    )
    .unwrap();
    let s2 = advance(&bc, s_revealed.state_changes, seller, 2, 400);
    // record_winner with effective=600 != revealed effective=500.
    let err = EvaporVM::execute(
        &bc,
        "record_winner",
        vec![Value::Address(alice), Value::U64(600)],
        s2,
        &ctx(seller, seller, 410, 10_000),
    )
    .expect_err("mismatched effective must reject");
    assert!(
        format!("{err:?}").contains("effective bid mismatch"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_seller_settle_rejects() {
    let bc = compile_pilot();
    let seller = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let s0 = seal(&bc, seller, "x", 100);
    let s_c = EvaporVM::execute(
        &bc,
        "commit",
        vec![Value::Str("h".to_string())],
        s0,
        &ctx(alice, seller, 200, 10_000),
    )
    .unwrap();
    let s1 = advance(&bc, s_c.state_changes, seller, 1, 300);
    let s_r = EvaporVM::execute(
        &bc,
        "reveal",
        vec![Value::U64(500), Value::U64(500), Value::Str("h".to_string())],
        s1,
        &ctx(alice, seller, 310, 10_000),
    )
    .unwrap();
    let s2 = advance(&bc, s_r.state_changes, seller, 2, 400);
    let err = EvaporVM::execute(
        &bc,
        "record_winner",
        vec![Value::Address(alice), Value::U64(500)],
        s2,
        &ctx(alice, seller, 410, 10_000),
    )
    .expect_err("non-seller settle must reject");
    assert!(
        format!("{err:?}").contains("only seller can settle"),
        "wrong revert: {err:?}"
    );
}

/// SBA-1: reveal with wrong commitment hash must reject.
/// Closes the audit finding: commit stored the hash but reveal never
/// checked it.  Now `reveal` requires `commitment_hash == stored hash`.
#[test]
fn sba1_reveal_wrong_commitment_hash_rejects() {
    let bc = compile_pilot();
    let seller = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let s0 = seal(&bc, seller, "x", 100);
    // Alice commits with hash "correct-hash".
    let s_c = EvaporVM::execute(
        &bc,
        "commit",
        vec![Value::Str("correct-hash".to_string())],
        s0,
        &ctx(alice, seller, 200, 10_000),
    )
    .unwrap();
    let s1 = advance(&bc, s_c.state_changes, seller, 1, 300);
    // Alice tries to reveal with a different hash — must reject.
    let err = EvaporVM::execute(
        &bc,
        "reveal",
        vec![
            Value::U64(500),
            Value::U64(500),
            Value::Str("wrong-hash".to_string()),
        ],
        s1.clone(),
        &ctx(alice, seller, 310, 10_000),
    )
    .expect_err("reveal with wrong hash must reject");
    assert!(
        format!("{err:?}").contains("commitment hash mismatch"),
        "wrong revert: {err:?}"
    );
    // Alice reveals with the correct hash — must succeed.
    EvaporVM::execute(
        &bc,
        "reveal",
        vec![
            Value::U64(500),
            Value::U64(500),
            Value::Str("correct-hash".to_string()),
        ],
        s1,
        &ctx(alice, seller, 310, 10_000),
    )
    .expect("reveal with correct hash must succeed");
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let seller = [0xAAu8; 32];
    let s0 = seal(&bc, seller, "x", 100);
    for hook in &["on_grace", "on_refresh", "on_evaporate"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            s0.clone(),
            &ctx(seller, seller, 500, 100),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        let _ = r.events;
    }
}

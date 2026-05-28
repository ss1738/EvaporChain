//! Pilot — drive `contracts/evaporscript/mortal_nft.es` through the full
//! parse → compile → VM execution pipeline.
//!
//! Pins the documented invariants:
//!   1. `set_metadata` is one-shot and minter-only (caller == builtin owner).
//!   2. After mint, the on-chain `self.holder` is the recipient, which can
//!      differ from the minter (mint-to-someone-else flow).
//!   3. `transfer` is gated on `caller == self.holder` — the current holder,
//!      not the original minter. After transfer, the new holder is the
//!      only one who can transfer again.
//!   4. `transfer_count` and `last_transfer_epoch` track chain-of-custody.
//!   5. Operations gated on `sealed == true` revert before mint.
//!   6. Lifecycle hooks emit cleanly.
//!   7. NFT-1 (audit 2026-05-17): compiler rejects state fields with
//!      builtin-reserved names (owner, caller, epoch, energy).

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/mortal_nft.es");

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
    let ast = parser::parse(SOURCE).unwrap_or_else(|e| panic!("MortalNft failed to parse: {e:?}"));
    compiler::compile(&ast).unwrap_or_else(|e| panic!("MortalNft failed to compile: {e:?}"))
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
    assert_eq!(bc.name, "MortalNft");
    let public = [
        "set_metadata",
        "transfer",
        "current_owner",
        "metadata_uri",
        "transfers",
    ];
    for m in &public {
        assert!(
            bc.methods.contains_key(*m),
            "method `{m}` missing from compiled bytecode"
        );
    }
}

#[test]
fn mint_seals_and_initial_owner_is_recipient() {
    let bc = compile_pilot();
    let minter = [0xAAu8; 32];
    let recipient = [0xBBu8; 32];

    let minted = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("MyNFT".to_string()),
            Value::Str("Genesis".to_string()),
            Value::Str("ipfs://Qm...".to_string()),
            Value::Address(recipient),
        ],
        initial_state(&bc),
        &ctx(minter, minter, 100, 10_000),
    )
    .expect("mint must succeed for deployer");
    assert!(
        minted.events.iter().any(|e| e.contains("minted")),
        "mint must emit minted event"
    );

    let owner_q = EvaporVM::execute(
        &bc,
        "current_owner",
        vec![],
        minted.state_changes.clone(),
        &ctx(recipient, minter, 101, 9_900),
    )
    .unwrap();
    assert_eq!(
        owner_q.return_value,
        Value::Address(recipient),
        "current_owner must be the recipient passed at mint, not the minter"
    );

    let meta = EvaporVM::execute(
        &bc,
        "metadata_uri",
        vec![],
        minted.state_changes,
        &ctx(recipient, minter, 102, 9_900),
    )
    .unwrap();
    assert_eq!(meta.return_value, Value::Str("ipfs://Qm...".to_string()));
}

#[test]
fn non_minter_cannot_seal() {
    let bc = compile_pilot();
    let minter = [0xAAu8; 32];
    let attacker = [0xCCu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("Hostile".to_string()),
            Value::Str("X".to_string()),
            Value::Str("".to_string()),
            Value::Address(attacker),
        ],
        initial_state(&bc),
        &ctx(attacker, minter, 100, 10_000),
    )
    .expect_err("non-minter must revert");
    assert!(
        format!("{err:?}").contains("only minter"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn double_mint_reverts() {
    let bc = compile_pilot();
    let minter = [0xAAu8; 32];
    let recipient = [0xBBu8; 32];
    let first = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("First".to_string()),
            Value::Str("Genesis".to_string()),
            Value::Str("a".to_string()),
            Value::Address(recipient),
        ],
        initial_state(&bc),
        &ctx(minter, minter, 100, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("Second".to_string()),
            Value::Str("Genesis".to_string()),
            Value::Str("b".to_string()),
            Value::Address(recipient),
        ],
        first.state_changes,
        &ctx(minter, minter, 200, 9_500),
    )
    .expect_err("re-mint must revert");
    assert!(
        format!("{err:?}").contains("already minted"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn transfer_before_mint_reverts() {
    let bc = compile_pilot();
    let minter = [0xAAu8; 32];
    let target = [0xBBu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "transfer",
        vec![Value::Address(target)],
        initial_state(&bc),
        &ctx(minter, minter, 100, 10_000),
    )
    .expect_err("transfer before mint must revert");
    assert!(
        format!("{err:?}").contains("not yet minted"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn transfer_only_by_current_owner() {
    let bc = compile_pilot();
    let minter = [0xAAu8; 32];
    let alice = [0xB1u8; 32];
    let bob = [0xB2u8; 32];

    // Mint to alice.
    let minted = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("N".to_string()),
            Value::Str("C".to_string()),
            Value::Str("m".to_string()),
            Value::Address(alice),
        ],
        initial_state(&bc),
        &ctx(minter, minter, 100, 10_000),
    )
    .unwrap();

    // Bob (non-owner) attempts transfer — must revert. Note: caller is
    // bob, not alice; owner builtin is still minter.
    let err = EvaporVM::execute(
        &bc,
        "transfer",
        vec![Value::Address(bob)],
        minted.state_changes.clone(),
        &ctx(bob, minter, 110, 9_900),
    )
    .expect_err("non-holder transfer must revert");
    assert!(
        format!("{err:?}").contains("only current owner"),
        "wrong revert: {err:?}"
    );

    // The minter can NOT transfer either — they're not the current owner
    // anymore (alice is).
    let err2 = EvaporVM::execute(
        &bc,
        "transfer",
        vec![Value::Address(bob)],
        minted.state_changes.clone(),
        &ctx(minter, minter, 110, 9_900),
    )
    .expect_err("minter (no longer holder) must revert");
    assert!(
        format!("{err2:?}").contains("only current owner"),
        "wrong revert: {err2:?}"
    );

    // Alice transfers to bob.
    let after = EvaporVM::execute(
        &bc,
        "transfer",
        vec![Value::Address(bob)],
        minted.state_changes,
        &ctx(alice, minter, 110, 9_900),
    )
    .expect("holder transfer must succeed");

    let new_owner = EvaporVM::execute(
        &bc,
        "current_owner",
        vec![],
        after.state_changes.clone(),
        &ctx(bob, minter, 111, 9_800),
    )
    .unwrap();
    assert_eq!(
        new_owner.return_value,
        Value::Address(bob),
        "current_owner must be bob after transfer"
    );

    // Now alice can NOT transfer back — bob is the holder now.
    let err3 = EvaporVM::execute(
        &bc,
        "transfer",
        vec![Value::Address(alice)],
        after.state_changes.clone(),
        &ctx(alice, minter, 112, 9_800),
    )
    .expect_err("ex-holder must revert");
    assert!(
        format!("{err3:?}").contains("only current owner"),
        "wrong revert: {err3:?}"
    );

    // transfer_count is 1.
    let count = EvaporVM::execute(
        &bc,
        "transfers",
        vec![],
        after.state_changes,
        &ctx(bob, minter, 113, 9_800),
    )
    .unwrap();
    assert_eq!(count.return_value, Value::U64(1));
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let minter = [0xAAu8; 32];
    let recipient = [0xBBu8; 32];
    let minted = EvaporVM::execute(
        &bc,
        "set_metadata",
        vec![
            Value::Str("N".to_string()),
            Value::Str("C".to_string()),
            Value::Str("m".to_string()),
            Value::Address(recipient),
        ],
        initial_state(&bc),
        &ctx(minter, minter, 100, 10_000),
    )
    .unwrap();
    for hook in &["on_grace", "on_refresh", "on_evaporate"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            minted.state_changes.clone(),
            &ctx(minter, minter, 500, 0),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        assert!(
            !r.events.is_empty(),
            "hook {hook} must emit at least one event"
        );
    }
}

// NFT-1 adversarial: the compiler must reject state fields named with
// builtin-reserved words. Each such field would be silently inaccessible
// without `self.` but bare access would return the builtin value instead.
#[test]
fn nft1_reserved_state_field_names_rejected_at_compile_time() {
    for reserved in &["owner", "caller", "epoch", "energy"] {
        let src = format!("contract Bad {{ state {{ {reserved}: u64 = 0 }} fn foo() {{}} }}");
        let ast = parser::parse(&src)
            .unwrap_or_else(|e| panic!("parse must succeed for reserved-name test: {e:?}"));
        let err = compiler::compile(&ast).expect_err(&format!(
            "state field '{reserved}' must be rejected at compile time"
        ));
        let msg = format!("{err:?}");
        assert!(
            msg.contains("reserved"),
            "expected 'reserved' in compile error for field '{reserved}', got: {msg}"
        );
    }
}

//! Pilot — drive `contracts/evaporscript/attestation.es` through the full
//! parse → compile → VM execution pipeline.
//!
//! Sixth worked-example behavioural pilot. Attestation makes the
//! *strength* of a claim decay with the contract's energy — silence is
//! decay. Co-signers can endorse; the original attestor can revoke. The
//! ghost record after evaporation preserves the audit trail but
//! carries no live weight.
//!
//! Pins:
//!   1. attest is one-shot + attestor-only; subject + claim immutable
//!      after seal.
//!   2. endorse is open; one endorsement per address; bumps counter.
//!   3. revoke is attestor-only; sets revoked + revoked_at_epoch.
//!   4. age() returns 0 pre-attest, then current_epoch - attested_at.
//!   5. has_endorsement read works pre and post endorse.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/attestation.es");

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
        .unwrap_or_else(|e| panic!("Attestation failed to parse: {e:?}"));
    compiler::compile(&ast)
        .unwrap_or_else(|e| panic!("Attestation failed to compile: {e:?}"))
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

fn seal(bc: &EvaporBytecode, attestor: [u8; 32], subject: &str, claim: &str) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "attest",
        vec![Value::Str(subject.to_string()), Value::Str(claim.to_string())],
        initial_state(bc),
        &ctx(attestor, attestor, 100, 10_000),
    )
    .expect("attest must succeed");
    r.state_changes
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "Attestation");
    let public = [
        "attest",
        "endorse",
        "revoke",
        "subject_of",
        "claim_text",
        "attestor_of",
        "attested_at",
        "is_revoked",
        "endorsements_total",
        "has_endorsement",
        "age",
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
fn attest_seals_subject_and_claim() {
    let bc = compile_pilot();
    let attestor = [0xAAu8; 32];
    let sealed = seal(&bc, attestor, "alice@example.com", "verified KYC tier 2");

    let subj = EvaporVM::execute(
        &bc,
        "subject_of",
        vec![],
        sealed.clone(),
        &ctx(attestor, attestor, 101, 10_000),
    )
    .unwrap();
    assert_eq!(
        subj.return_value,
        Value::Str("alice@example.com".to_string())
    );

    let claim = EvaporVM::execute(
        &bc,
        "claim_text",
        vec![],
        sealed.clone(),
        &ctx(attestor, attestor, 102, 10_000),
    )
    .unwrap();
    assert_eq!(
        claim.return_value,
        Value::Str("verified KYC tier 2".to_string())
    );

    let at = EvaporVM::execute(
        &bc,
        "attested_at",
        vec![],
        sealed,
        &ctx(attestor, attestor, 103, 10_000),
    )
    .unwrap();
    assert_eq!(at.return_value, Value::U64(100));
}

#[test]
fn double_attest_rejects() {
    let bc = compile_pilot();
    let attestor = [0xAAu8; 32];
    let sealed = seal(&bc, attestor, "alice", "claim");
    let err = EvaporVM::execute(
        &bc,
        "attest",
        vec![Value::Str("bob".to_string()), Value::Str("other".to_string())],
        sealed,
        &ctx(attestor, attestor, 200, 10_000),
    )
    .expect_err("double attest must reject");
    assert!(
        format!("{err:?}").contains("already sealed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_attestor_attest_rejects() {
    let bc = compile_pilot();
    let attestor = [0xAAu8; 32];
    let attacker = [0xCCu8; 32];
    let err = EvaporVM::execute(
        &bc,
        "attest",
        vec![Value::Str("x".to_string()), Value::Str("y".to_string())],
        initial_state(&bc),
        &ctx(attacker, attestor, 100, 10_000),
    )
    .expect_err("non-attestor must reject");
    assert!(
        format!("{err:?}").contains("only attestor"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn endorse_records_caller_and_counts() {
    let bc = compile_pilot();
    let attestor = [0xAAu8; 32];
    let cosigner1 = [0xB1u8; 32];
    let cosigner2 = [0xB2u8; 32];
    let sealed = seal(&bc, attestor, "alice", "claim");

    let e1 = EvaporVM::execute(
        &bc,
        "endorse",
        vec![],
        sealed,
        &ctx(cosigner1, attestor, 200, 10_000),
    )
    .expect("first endorse");
    let e2 = EvaporVM::execute(
        &bc,
        "endorse",
        vec![],
        e1.state_changes,
        &ctx(cosigner2, attestor, 201, 10_000),
    )
    .expect("second endorse");

    let total = EvaporVM::execute(
        &bc,
        "endorsements_total",
        vec![],
        e2.state_changes.clone(),
        &ctx(attestor, attestor, 202, 10_000),
    )
    .unwrap();
    assert_eq!(total.return_value, Value::U64(2));

    let has1 = EvaporVM::execute(
        &bc,
        "has_endorsement",
        vec![Value::Address(cosigner1)],
        e2.state_changes.clone(),
        &ctx(attestor, attestor, 203, 10_000),
    )
    .unwrap();
    assert_eq!(has1.return_value, Value::Bool(true));

    // Non-endorser returns false.
    let third = [0xB3u8; 32];
    let has3 = EvaporVM::execute(
        &bc,
        "has_endorsement",
        vec![Value::Address(third)],
        e2.state_changes,
        &ctx(attestor, attestor, 204, 10_000),
    )
    .unwrap();
    assert_eq!(has3.return_value, Value::Bool(false));
}

#[test]
fn duplicate_endorse_rejects() {
    let bc = compile_pilot();
    let attestor = [0xAAu8; 32];
    let cosigner = [0xB1u8; 32];
    let sealed = seal(&bc, attestor, "alice", "claim");
    let e1 = EvaporVM::execute(
        &bc,
        "endorse",
        vec![],
        sealed,
        &ctx(cosigner, attestor, 200, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "endorse",
        vec![],
        e1.state_changes,
        &ctx(cosigner, attestor, 201, 10_000),
    )
    .expect_err("dup endorse must reject");
    assert!(
        format!("{err:?}").contains("already endorsed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn endorse_pre_attest_rejects() {
    let bc = compile_pilot();
    let attestor = [0xAAu8; 32];
    let cosigner = [0xB1u8; 32];
    let err = EvaporVM::execute(
        &bc,
        "endorse",
        vec![],
        initial_state(&bc),
        &ctx(cosigner, attestor, 200, 10_000),
    )
    .expect_err("pre-attest endorse must reject");
    assert!(
        format!("{err:?}").contains("not yet sealed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn revoke_flips_flag_and_blocks_double_revoke() {
    let bc = compile_pilot();
    let attestor = [0xAAu8; 32];
    let sealed = seal(&bc, attestor, "alice", "claim");

    let revoked = EvaporVM::execute(
        &bc,
        "revoke",
        vec![],
        sealed,
        &ctx(attestor, attestor, 300, 10_000),
    )
    .expect("revoke must succeed");
    let is_rev = EvaporVM::execute(
        &bc,
        "is_revoked",
        vec![],
        revoked.state_changes.clone(),
        &ctx(attestor, attestor, 301, 10_000),
    )
    .unwrap();
    assert_eq!(is_rev.return_value, Value::Bool(true));

    let err = EvaporVM::execute(
        &bc,
        "revoke",
        vec![],
        revoked.state_changes,
        &ctx(attestor, attestor, 302, 10_000),
    )
    .expect_err("double revoke must reject");
    assert!(
        format!("{err:?}").contains("already revoked"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_attestor_revoke_rejects() {
    let bc = compile_pilot();
    let attestor = [0xAAu8; 32];
    let attacker = [0xCCu8; 32];
    let sealed = seal(&bc, attestor, "alice", "claim");
    let err = EvaporVM::execute(
        &bc,
        "revoke",
        vec![],
        sealed,
        &ctx(attacker, attestor, 300, 10_000),
    )
    .expect_err("non-attestor revoke must reject");
    assert!(
        format!("{err:?}").contains("only attestor"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn endorse_post_revoke_still_allowed() {
    // Doctrine: co-signers may stand by a revoked claim. Endorsements
    // do not require unrevoked status.
    let bc = compile_pilot();
    let attestor = [0xAAu8; 32];
    let cosigner = [0xB1u8; 32];
    let sealed = seal(&bc, attestor, "alice", "claim");
    let revoked = EvaporVM::execute(
        &bc,
        "revoke",
        vec![],
        sealed,
        &ctx(attestor, attestor, 300, 10_000),
    )
    .unwrap();
    let endorsed = EvaporVM::execute(
        &bc,
        "endorse",
        vec![],
        revoked.state_changes,
        &ctx(cosigner, attestor, 310, 10_000),
    )
    .expect("endorse-after-revoke must succeed (cosigner stands by claim)");
    let total = EvaporVM::execute(
        &bc,
        "endorsements_total",
        vec![],
        endorsed.state_changes,
        &ctx(attestor, attestor, 311, 10_000),
    )
    .unwrap();
    assert_eq!(total.return_value, Value::U64(1));
}

#[test]
fn age_tracks_epoch_delta() {
    let bc = compile_pilot();
    let attestor = [0xAAu8; 32];
    // Pre-attest: age == 0.
    let pre = EvaporVM::execute(
        &bc,
        "age",
        vec![],
        initial_state(&bc),
        &ctx(attestor, attestor, 50, 10_000),
    )
    .unwrap();
    assert_eq!(pre.return_value, Value::U64(0));

    let sealed = seal(&bc, attestor, "alice", "claim");
    let later = EvaporVM::execute(
        &bc,
        "age",
        vec![],
        sealed,
        &ctx(attestor, attestor, 850, 10_000),
    )
    .unwrap();
    // attest happened at epoch 100; query at 850 → age 750.
    assert_eq!(later.return_value, Value::U64(750));
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let attestor = [0xAAu8; 32];
    let sealed = seal(&bc, attestor, "alice", "claim");
    for hook in &["on_grace", "on_refresh", "on_evaporate"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            sealed.clone(),
            &ctx(attestor, attestor, 500, 100),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        let _ = r.events;
    }
}

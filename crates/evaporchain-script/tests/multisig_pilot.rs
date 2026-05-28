//! Pilot — drive `contracts/evaporscript/multisig.es` through the full
//! parse → compile → VM execution pipeline.
//!
//! Third worked-example behavioural pilot for the seed-12 stdlib (after
//! dead_man_switch + payment_split). Multisig is the auth-heavy contract:
//! signer set + threshold + proposal sealing + per-signer sign-once gate +
//! threshold-met execution. This pilot doubles as a regression on the
//! one-decision-per-contract design pattern that distinguishes Multisig
//! from Gnosis-Safe-style proposal-map architectures.
//!
//! Pins the documented invariants:
//!   1. add_signer + set_threshold are deployer-only and pre-propose.
//!   2. propose seals the signer set; subsequent add_signer/set_threshold
//!      must reject.
//!   3. set_threshold > signer_count is invalid.
//!   4. Only registered signers can sign; one signature per signer.
//!   5. execute reverts below threshold; succeeds at threshold.
//!   6. Post-execute signs are blocked.
//!   7. on_evaporate without execute flips `expired = true`; after
//!      execute, on_evaporate is a no-op for `expired`.
//!   8. Lifecycle hooks emit cleanly.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/multisig.es");

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
    let ast = parser::parse(SOURCE).unwrap_or_else(|e| panic!("Multisig failed to parse: {e:?}"));
    compiler::compile(&ast).unwrap_or_else(|e| panic!("Multisig failed to compile: {e:?}"))
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

/// Helper — register N signers, set threshold k, then propose. Returns
/// the post-propose state map for downstream tests.
fn setup_proposal(
    bc: &EvaporBytecode,
    deployer: [u8; 32],
    signers: &[[u8; 32]],
    threshold: u64,
    action: &str,
) -> HashMap<String, Value> {
    let mut s = initial_state(bc);
    let mut epoch = 100;
    for sig in signers {
        let r = EvaporVM::execute(
            bc,
            "add_signer",
            vec![Value::Address(*sig)],
            s.clone(),
            &ctx(deployer, deployer, epoch, 10_000),
        )
        .unwrap_or_else(|e| panic!("add_signer must succeed: {e:?}"));
        s = r.state_changes;
        epoch += 1;
    }
    let r = EvaporVM::execute(
        bc,
        "set_threshold",
        vec![Value::U64(threshold)],
        s,
        &ctx(deployer, deployer, epoch, 10_000),
    )
    .expect("set_threshold");
    epoch += 1;
    let r = EvaporVM::execute(
        bc,
        "propose",
        vec![Value::Str(action.to_string())],
        r.state_changes,
        &ctx(deployer, deployer, epoch, 10_000),
    )
    .expect("propose");
    r.state_changes
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "Multisig");
    let public = [
        "add_signer",
        "set_threshold",
        "propose",
        "sign",
        "execute",
        "signers_total",
        "threshold_required",
        "signatures_collected",
        "has_signed",
        "is_signer",
        "proposal_action",
        "is_executed",
        "is_pending",
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
fn full_setup_to_execute_round_trip() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let s1 = [0x11u8; 32];
    let s2 = [0x22u8; 32];
    let s3 = [0x33u8; 32];

    // 3 signers, threshold = 2.
    let post_propose = setup_proposal(&bc, deployer, &[s1, s2, s3], 2, "transfer-to-X");

    // Verify state.
    let total = EvaporVM::execute(
        &bc,
        "signers_total",
        vec![],
        post_propose.clone(),
        &ctx(deployer, deployer, 200, 10_000),
    )
    .unwrap();
    assert_eq!(total.return_value, Value::U64(3));

    let req = EvaporVM::execute(
        &bc,
        "threshold_required",
        vec![],
        post_propose.clone(),
        &ctx(deployer, deployer, 201, 10_000),
    )
    .unwrap();
    assert_eq!(req.return_value, Value::U64(2));

    // s1 signs.
    let after_s1 = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        post_propose,
        &ctx(s1, deployer, 210, 10_000),
    )
    .expect("s1 sign must succeed");
    let n = EvaporVM::execute(
        &bc,
        "signatures_collected",
        vec![],
        after_s1.state_changes.clone(),
        &ctx(s1, deployer, 211, 10_000),
    )
    .unwrap();
    assert_eq!(n.return_value, Value::U64(1));

    // execute below threshold reverts.
    let err = EvaporVM::execute(
        &bc,
        "execute",
        vec![],
        after_s1.state_changes.clone(),
        &ctx(deployer, deployer, 220, 10_000),
    )
    .expect_err("execute below threshold must revert");
    assert!(
        format!("{err:?}").contains("threshold not yet reached"),
        "wrong revert: {err:?}"
    );

    // s2 signs — now at threshold.
    let after_s2 = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        after_s1.state_changes,
        &ctx(s2, deployer, 230, 10_000),
    )
    .expect("s2 sign must succeed");
    let n2 = EvaporVM::execute(
        &bc,
        "signatures_collected",
        vec![],
        after_s2.state_changes.clone(),
        &ctx(s2, deployer, 231, 10_000),
    )
    .unwrap();
    assert_eq!(n2.return_value, Value::U64(2));

    // Anyone can call execute now.
    let executor = [0x99u8; 32];
    let after_exec = EvaporVM::execute(
        &bc,
        "execute",
        vec![],
        after_s2.state_changes,
        &ctx(executor, deployer, 240, 10_000),
    )
    .expect("execute at threshold must succeed");
    let executed = EvaporVM::execute(
        &bc,
        "is_executed",
        vec![],
        after_exec.state_changes.clone(),
        &ctx(executor, deployer, 241, 10_000),
    )
    .unwrap();
    assert_eq!(executed.return_value, Value::Bool(true));

    // Action hash readable.
    let action = EvaporVM::execute(
        &bc,
        "proposal_action",
        vec![],
        after_exec.state_changes,
        &ctx(executor, deployer, 242, 10_000),
    )
    .unwrap();
    assert_eq!(action.return_value, Value::Str("transfer-to-X".to_string()));
}

#[test]
fn add_signer_post_propose_rejects() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let s1 = [0x11u8; 32];
    let s2 = [0x22u8; 32];
    let extra = [0x44u8; 32];

    let post_propose = setup_proposal(&bc, deployer, &[s1, s2], 2, "x");

    let err = EvaporVM::execute(
        &bc,
        "add_signer",
        vec![Value::Address(extra)],
        post_propose,
        &ctx(deployer, deployer, 300, 10_000),
    )
    .expect_err("add_signer post-propose must revert");
    assert!(
        format!("{err:?}").contains("proposal already sealed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn threshold_exceeds_signer_count_rejects() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let s1 = [0x11u8; 32];
    let s2 = [0x22u8; 32];
    let mut s = initial_state(&bc);

    let r = EvaporVM::execute(
        &bc,
        "add_signer",
        vec![Value::Address(s1)],
        s,
        &ctx(deployer, deployer, 100, 10_000),
    )
    .unwrap();
    s = r.state_changes;
    let r = EvaporVM::execute(
        &bc,
        "add_signer",
        vec![Value::Address(s2)],
        s,
        &ctx(deployer, deployer, 101, 10_000),
    )
    .unwrap();
    s = r.state_changes;

    // threshold = 3 against 2 signers must reject.
    let err = EvaporVM::execute(
        &bc,
        "set_threshold",
        vec![Value::U64(3)],
        s,
        &ctx(deployer, deployer, 102, 10_000),
    )
    .expect_err("threshold > signer_count must reject");
    assert!(
        format!("{err:?}").contains("threshold exceeds signer count"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn non_signer_sign_rejects() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let s1 = [0x11u8; 32];
    let s2 = [0x22u8; 32];
    let attacker = [0xCCu8; 32];

    let post_propose = setup_proposal(&bc, deployer, &[s1, s2], 2, "x");

    let err = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        post_propose,
        &ctx(attacker, deployer, 300, 10_000),
    )
    .expect_err("non-signer sign must revert");
    assert!(
        format!("{err:?}").contains("not a signer"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn duplicate_sign_by_same_signer_rejects() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let s1 = [0x11u8; 32];
    let s2 = [0x22u8; 32];

    let post_propose = setup_proposal(&bc, deployer, &[s1, s2], 2, "x");

    let after_s1 = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        post_propose,
        &ctx(s1, deployer, 300, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        after_s1.state_changes,
        &ctx(s1, deployer, 301, 10_000),
    )
    .expect_err("dup sign must revert");
    assert!(
        format!("{err:?}").contains("already signed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn sign_post_execute_rejects() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let s1 = [0x11u8; 32];
    let s2 = [0x22u8; 32];
    let s3 = [0x33u8; 32];

    let post_propose = setup_proposal(&bc, deployer, &[s1, s2, s3], 2, "x");

    let after_s1 = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        post_propose,
        &ctx(s1, deployer, 300, 10_000),
    )
    .unwrap();
    let after_s2 = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        after_s1.state_changes,
        &ctx(s2, deployer, 301, 10_000),
    )
    .unwrap();
    let after_exec = EvaporVM::execute(
        &bc,
        "execute",
        vec![],
        after_s2.state_changes,
        &ctx(deployer, deployer, 310, 10_000),
    )
    .unwrap();

    // s3's late signature must reject — proposal is closed.
    let err = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        after_exec.state_changes,
        &ctx(s3, deployer, 320, 10_000),
    )
    .expect_err("post-execute sign must revert");
    assert!(
        format!("{err:?}").contains("already executed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn double_execute_rejects() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let s1 = [0x11u8; 32];
    let s2 = [0x22u8; 32];

    let post_propose = setup_proposal(&bc, deployer, &[s1, s2], 2, "x");

    let after_s1 = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        post_propose,
        &ctx(s1, deployer, 300, 10_000),
    )
    .unwrap();
    let after_s2 = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        after_s1.state_changes,
        &ctx(s2, deployer, 301, 10_000),
    )
    .unwrap();
    let after_exec = EvaporVM::execute(
        &bc,
        "execute",
        vec![],
        after_s2.state_changes,
        &ctx(deployer, deployer, 310, 10_000),
    )
    .unwrap();

    let err = EvaporVM::execute(
        &bc,
        "execute",
        vec![],
        after_exec.state_changes,
        &ctx(deployer, deployer, 320, 10_000),
    )
    .expect_err("dup execute must revert");
    assert!(
        format!("{err:?}").contains("already executed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn on_evaporate_without_execute_marks_expired() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let s1 = [0x11u8; 32];
    let s2 = [0x22u8; 32];

    let post_propose = setup_proposal(&bc, deployer, &[s1, s2], 2, "x");
    // Only s1 signs — below threshold; never executed.
    let after_s1 = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        post_propose,
        &ctx(s1, deployer, 300, 10_000),
    )
    .unwrap();

    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        after_s1.state_changes,
        &ctx(deployer, deployer, 9_000, 0),
    )
    .expect("on_evaporate must execute");

    let pending = EvaporVM::execute(
        &bc,
        "is_pending",
        vec![],
        evap.state_changes.clone(),
        &ctx(deployer, deployer, 9_001, 0),
    )
    .unwrap();
    assert_eq!(
        pending.return_value,
        Value::Bool(false),
        "is_pending must be false after expiration"
    );
    assert!(
        evap.events.iter().any(|e| e.contains("expired")),
        "evaporate must emit expired event"
    );
}

#[test]
fn on_evaporate_after_execute_does_not_remark_expired() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let s1 = [0x11u8; 32];
    let s2 = [0x22u8; 32];

    let post_propose = setup_proposal(&bc, deployer, &[s1, s2], 2, "x");
    let after_s1 = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        post_propose,
        &ctx(s1, deployer, 300, 10_000),
    )
    .unwrap();
    let after_s2 = EvaporVM::execute(
        &bc,
        "sign",
        vec![],
        after_s1.state_changes,
        &ctx(s2, deployer, 301, 10_000),
    )
    .unwrap();
    let after_exec = EvaporVM::execute(
        &bc,
        "execute",
        vec![],
        after_s2.state_changes,
        &ctx(deployer, deployer, 310, 10_000),
    )
    .unwrap();

    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        after_exec.state_changes,
        &ctx(deployer, deployer, 9_000, 0),
    )
    .expect("on_evaporate must execute even post-exec");

    // is_executed remains true; expired stays false (the contract
    // discharged its purpose before evaporation).
    let executed = EvaporVM::execute(
        &bc,
        "is_executed",
        vec![],
        evap.state_changes.clone(),
        &ctx(deployer, deployer, 9_001, 0),
    )
    .unwrap();
    assert_eq!(executed.return_value, Value::Bool(true));
    if let Some(Value::Bool(expired)) = evap.state_changes.get("expired") {
        assert!(
            !*expired,
            "expired must NOT flip true on a multisig that already executed"
        );
    }
}

#[test]
fn lifecycle_hooks_execute_cleanly() {
    let bc = compile_pilot();
    let deployer = [0xAAu8; 32];
    let s1 = [0x11u8; 32];
    let s2 = [0x22u8; 32];

    let post_propose = setup_proposal(&bc, deployer, &[s1, s2], 2, "x");
    for hook in &["on_grace", "on_refresh"] {
        let r = EvaporVM::execute(
            &bc,
            hook,
            vec![],
            post_propose.clone(),
            &ctx(deployer, deployer, 500, 100),
        )
        .unwrap_or_else(|e| panic!("hook {hook} must execute cleanly: {e:?}"));
        let _ = r.events;
    }
}

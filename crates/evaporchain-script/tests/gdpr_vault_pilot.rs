//! Pilot — drive `contracts/evaporscript/gdpr_vault.es` through the
//! full parse → compile → VM execution pipeline.
//!
//! 15th worked-example behavioural pilot. GdprVault's doctrine
//! moment: GDPR Erasure-as-a-Service via crypto-shred. The chain
//! holds NO personal data — only a 32-byte ciphertext commitment +
//! the consent/retention lifecycle. "Erasure" = off-chain destruction
//! of the decryption key, triggered by the contract's terminal
//! evaporation (or by an explicit `withdraw_consent`). The contract's
//! OWN energy is the retention clock; the off-chain key-custody/HSM
//! subscribes to the emitted "erasure-due" events.
//!
//! Pins:
//!   1. seal() one-shot + controller-only; lawful_basis > 0.
//!   2. withdraw_consent gated to subject OR controller (dual-keyed);
//!      flips expiry_forced; emits the erasure-due event with the
//!      consent-withdrawn marker (distinguishable from natural-deadline
//!      shred so the audit log differentiates Art. 7(3) consent
//!      withdrawal from natural retention end).
//!   3. extend_retention is controller-only + rejected once
//!      expiry_forced (subject's erasure right cannot be silently
//!      overridden by a retention extension).
//!   4. status() returns 0 pre-seal, 2 sealed/normal, 1 erasure-forced
//!      (status code 1 = "consent withdrawn" comes BEFORE 2 because
//!      it's the more urgent state — pin the ordering).
//!   5. on_evaporate emits the "erasure-due: shred key" event
//!      unconditionally — the natural-deadline shred trigger.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str = include_str!("../../../contracts/evaporscript/gdpr_vault.es");

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
    let ast = parser::parse(SOURCE).unwrap_or_else(|e| panic!("GdprVault failed to parse: {e:?}"));
    compiler::compile(&ast).unwrap_or_else(|e| panic!("GdprVault failed to compile: {e:?}"))
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
    controller: [u8; 32],
    ct_commit: [u8; 32],
    subject: [u8; 32],
    basis: u64,
) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "seal",
        vec![
            Value::Address(ct_commit),
            Value::Address(subject),
            Value::U64(basis),
        ],
        initial_state(bc),
        &ctx(controller, controller, 100, 10_000),
    )
    .expect("seal must succeed");
    r.state_changes
}

#[test]
fn parses_and_compiles_cleanly() {
    let bc = compile_pilot();
    assert_eq!(bc.name, "GdprVault");
    let public = [
        "seal",
        "withdraw_consent",
        "extend_retention",
        "status",
        "lawful_basis_code",
        "subject",
        "ct_commitment",
        "sealed_epoch",
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
fn seal_validates_inputs() {
    let bc = compile_pilot();
    let controller = [0xAAu8; 32];
    let stranger = [0xCCu8; 32];
    let ct = [0x11u8; 32];
    let subject = [0xB1u8; 32];

    // Non-owner cannot seal.
    let err = EvaporVM::execute(
        &bc,
        "seal",
        vec![Value::Address(ct), Value::Address(subject), Value::U64(1)],
        initial_state(&bc),
        &ctx(stranger, controller, 100, 10_000),
    )
    .expect_err("non-controller must reject");
    assert!(
        format!("{err:?}").contains("only the controller can seal this vault"),
        "wrong revert: {err:?}"
    );

    // Zero lawful_basis is rejected — the audit trail requires a code.
    let err = EvaporVM::execute(
        &bc,
        "seal",
        vec![Value::Address(ct), Value::Address(subject), Value::U64(0)],
        initial_state(&bc),
        &ctx(controller, controller, 100, 10_000),
    )
    .expect_err("zero basis must reject");
    assert!(
        format!("{err:?}").contains("lawful basis code must be positive"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn seal_is_one_shot() {
    let bc = compile_pilot();
    let controller = [0xAAu8; 32];
    let ct = [0x11u8; 32];
    let subject = [0xB1u8; 32];
    let after_first = seal(&bc, controller, ct, subject, 1);

    let err = EvaporVM::execute(
        &bc,
        "seal",
        vec![
            Value::Address([0x22u8; 32]),
            Value::Address([0xB2u8; 32]),
            Value::U64(2),
        ],
        after_first,
        &ctx(controller, controller, 200, 10_000),
    )
    .expect_err("second seal must reject");
    assert!(
        format!("{err:?}").contains("vault already sealed"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn withdraw_consent_dual_keyed_subject_or_controller() {
    let bc = compile_pilot();
    let controller = [0xAAu8; 32];
    let subject = [0xB1u8; 32];
    let stranger = [0xCCu8; 32];
    let ct = [0x11u8; 32];
    let sealed_state = seal(&bc, controller, ct, subject, 1);

    // Stranger cannot withdraw.
    let err = EvaporVM::execute(
        &bc,
        "withdraw_consent",
        vec![],
        sealed_state.clone(),
        &ctx(stranger, controller, 150, 10_000),
    )
    .expect_err("stranger must reject");
    assert!(
        format!("{err:?}").contains("only the data subject or controller can request erasure"),
        "wrong revert: {err:?}"
    );

    // Subject CAN withdraw — Art. 7(3) right.
    let after_subj = EvaporVM::execute(
        &bc,
        "withdraw_consent",
        vec![],
        sealed_state.clone(),
        &ctx(subject, controller, 150, 10_000),
    )
    .expect("subject withdraw must succeed");
    // The erasure-due emit must distinguish consent-withdrawn from
    // natural-deadline so the audit log differentiates the two paths.
    let saw_consent_emit = after_subj
        .events
        .iter()
        .any(|e| format!("{e:?}").contains("consent withdrawn"));
    assert!(
        saw_consent_emit,
        "consent-withdrawn emit must include 'consent withdrawn' marker; events: {:?}",
        after_subj.events
    );

    // Second withdraw rejects (idempotent).
    let err = EvaporVM::execute(
        &bc,
        "withdraw_consent",
        vec![],
        after_subj.state_changes.clone(),
        &ctx(controller, controller, 160, 10_000),
    )
    .expect_err("double withdraw must reject");
    assert!(
        format!("{err:?}").contains("erasure already requested"),
        "wrong revert: {err:?}"
    );

    // Controller can withdraw on a fresh sealed vault (mirror path).
    let after_ctrl = EvaporVM::execute(
        &bc,
        "withdraw_consent",
        vec![],
        sealed_state,
        &ctx(controller, controller, 150, 10_000),
    )
    .expect("controller withdraw must succeed");
    let saw_consent_emit = after_ctrl
        .events
        .iter()
        .any(|e| format!("{e:?}").contains("consent withdrawn"));
    assert!(
        saw_consent_emit,
        "controller-withdraw emit must include 'consent withdrawn' marker; events: {:?}",
        after_ctrl.events
    );
}

#[test]
fn extend_retention_blocked_after_consent_withdrawn() {
    // The subject's erasure right cannot be silently overridden by a
    // retention extension. Once `withdraw_consent` flips
    // `expiry_forced`, `extend_retention` MUST reject.
    let bc = compile_pilot();
    let controller = [0xAAu8; 32];
    let subject = [0xB1u8; 32];
    let ct = [0x11u8; 32];
    let sealed_state = seal(&bc, controller, ct, subject, 1);

    // Controller can extend a sealed-but-not-forced vault.
    let _ok = EvaporVM::execute(
        &bc,
        "extend_retention",
        vec![],
        sealed_state.clone(),
        &ctx(controller, controller, 150, 10_000),
    )
    .expect("extend on sealed vault must succeed");

    // After consent withdrawn, extend must reject.
    let withdrawn = EvaporVM::execute(
        &bc,
        "withdraw_consent",
        vec![],
        sealed_state,
        &ctx(subject, controller, 150, 10_000),
    )
    .unwrap();
    let err = EvaporVM::execute(
        &bc,
        "extend_retention",
        vec![],
        withdrawn.state_changes,
        &ctx(controller, controller, 160, 10_000),
    )
    .expect_err("extend after consent-withdraw must reject");
    assert!(
        format!("{err:?}").contains("erasure already requested"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn status_codes_ordered_unsealed_forced_normal() {
    let bc = compile_pilot();
    let controller = [0xAAu8; 32];
    let subject = [0xB1u8; 32];
    let ct = [0x11u8; 32];

    // Status 0: not sealed.
    let q0 = EvaporVM::execute(
        &bc,
        "status",
        vec![],
        initial_state(&bc),
        &ctx(controller, controller, 100, 10_000),
    )
    .unwrap();
    assert_eq!(q0.return_value, Value::U64(0));

    // Status 2: sealed, normal retention.
    let sealed_state = seal(&bc, controller, ct, subject, 1);
    let q2 = EvaporVM::execute(
        &bc,
        "status",
        vec![],
        sealed_state.clone(),
        &ctx(controller, controller, 150, 10_000),
    )
    .unwrap();
    assert_eq!(q2.return_value, Value::U64(2));

    // Status 1: erasure forced (subject withdrew consent).
    let withdrawn = EvaporVM::execute(
        &bc,
        "withdraw_consent",
        vec![],
        sealed_state,
        &ctx(subject, controller, 160, 10_000),
    )
    .unwrap();
    let q1 = EvaporVM::execute(
        &bc,
        "status",
        vec![],
        withdrawn.state_changes,
        &ctx(controller, controller, 170, 10_000),
    )
    .unwrap();
    assert_eq!(q1.return_value, Value::U64(1));
}

#[test]
fn on_evaporate_emits_natural_deadline_shred_trigger() {
    let bc = compile_pilot();
    let controller = [0xAAu8; 32];
    let subject = [0xB1u8; 32];
    let ct = [0x11u8; 32];

    let sealed_state = seal(&bc, controller, ct, subject, 1);
    let evap = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        sealed_state,
        &ctx(controller, controller, 9999, 0),
    )
    .expect("on_evaporate must succeed");
    let saw_shred_emit = evap
        .events
        .iter()
        .any(|e| format!("{e:?}").contains("erasure-due"));
    assert!(
        saw_shred_emit,
        "on_evaporate must emit the natural-deadline shred trigger; events: {:?}",
        evap.events
    );
}

#[test]
fn audit_views_record_immutable_disposition() {
    // The audit trail a DPO/regulator reads: ct_commitment + subject +
    // lawful_basis_code + sealed_epoch. Verify all four are recorded
    // by seal() and survive intact through a withdraw_consent (the
    // erasure-request path doesn't overwrite the audit fields).
    let bc = compile_pilot();
    let controller = [0xAAu8; 32];
    let subject = [0xB1u8; 32];
    let ct = [0x11u8; 32];
    let sealed_state = seal(&bc, controller, ct, subject, 3);
    let withdrawn = EvaporVM::execute(
        &bc,
        "withdraw_consent",
        vec![],
        sealed_state,
        &ctx(controller, controller, 160, 10_000),
    )
    .unwrap();
    let state = withdrawn.state_changes;

    let ct_q = EvaporVM::execute(
        &bc,
        "ct_commitment",
        vec![],
        state.clone(),
        &ctx(controller, controller, 200, 10_000),
    )
    .unwrap();
    assert_eq!(ct_q.return_value, Value::Address(ct));

    let subj_q = EvaporVM::execute(
        &bc,
        "subject",
        vec![],
        state.clone(),
        &ctx(controller, controller, 200, 10_000),
    )
    .unwrap();
    assert_eq!(subj_q.return_value, Value::Address(subject));

    let basis_q = EvaporVM::execute(
        &bc,
        "lawful_basis_code",
        vec![],
        state.clone(),
        &ctx(controller, controller, 200, 10_000),
    )
    .unwrap();
    assert_eq!(basis_q.return_value, Value::U64(3));

    let sealed_at_q = EvaporVM::execute(
        &bc,
        "sealed_epoch",
        vec![],
        state,
        &ctx(controller, controller, 200, 10_000),
    )
    .unwrap();
    // seal() ran at epoch 100.
    assert_eq!(sealed_at_q.return_value, Value::U64(100));
}

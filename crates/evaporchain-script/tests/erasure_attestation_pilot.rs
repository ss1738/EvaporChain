//! Pilot — drive `contracts/evaporscript/erasure_attestation.es`
//! through the full parse → compile → VM execution pipeline.
//!
//! 16th worked-example behavioural pilot. ErasureAttestation's
//! doctrine moment: Proof-of-Erasure-as-a-Service for the
//! right-to-be-forgotten / AI machine-unlearning frontier. NIST
//! SP 800-88 answers the general media-disposition case with a
//! Certificate of Media Disposition (data ref + sanitization
//! METHOD + VERIFICATION result + who/when, retained as tamper-
//! evident proof); this contract IS that certificate, on-chain.
//! The chain holds NO personal data and does NOT perform
//! sanitization — it is the verifiable attestation/proof layer.
//!
//! Pair with `gdpr_vault.es` (model A crypto-shred): GdprVault is
//! the retention clock + shred trigger; ErasureAttestation is the
//! immutable proof that the obligation was honoured (or that its
//! window terminally closed un-attested — a regulator-grade
//! NEGATIVE proof).
//!
//! Pins:
//!   1. seal() one-shot + controller-only; basis > 0; method > 0.
//!   2. attest_erasure() one-shot + controller-only + requires
//!      sealed + verification_code > 0; flips attested + stamps
//!      attested_at.
//!   3. status() ordered: 0 not-opened, 1 sealed-not-yet-attested,
//!      2 attested (the canonical disposition lifecycle).
//!   4. on_evaporate emits "obligation window CLOSED with no
//!      attestation" ONLY when attested == false — a sealed-and-
//!      attested vault that evaporates is silent (the proof already
//!      stands). The negative-proof path is the regulator's
//!      tamper-evident record that the deadline lapsed un-attested.
//!   5. Audit fields locked at seal (data_ref, subject,
//!      obligation_basis, method, sealed_at) survive a later
//!      attest_erasure intact; attested_at is the only new field.

use std::collections::HashMap;

use evaporchain_script::{
    compiler::{self, EvaporBytecode},
    parser,
    vm::EvaporVM,
    ExecutionContext, Value,
};

const SOURCE: &str =
    include_str!("../../../contracts/evaporscript/erasure_attestation.es");

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
        .unwrap_or_else(|e| panic!("ErasureAttestation failed to parse: {e:?}"));
    compiler::compile(&ast)
        .unwrap_or_else(|e| panic!("ErasureAttestation failed to compile: {e:?}"))
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
    data: [u8; 32],
    subject: [u8; 32],
    basis: u64,
    method: u64,
) -> HashMap<String, Value> {
    let r = EvaporVM::execute(
        bc,
        "seal",
        vec![
            Value::Address(data),
            Value::Address(subject),
            Value::U64(basis),
            Value::U64(method),
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
    assert_eq!(bc.name, "ErasureAttestation");
    let public = [
        "seal",
        "attest_erasure",
        "status",
        "obligation_basis_code",
        "method_code",
        "subject",
        "data_commitment",
        "attested_epoch",
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
    let data = [0x11u8; 32];
    let subject = [0xB1u8; 32];

    // Non-owner cannot seal.
    let err = EvaporVM::execute(
        &bc,
        "seal",
        vec![
            Value::Address(data),
            Value::Address(subject),
            Value::U64(1),
            Value::U64(1),
        ],
        initial_state(&bc),
        &ctx(stranger, controller, 100, 10_000),
    )
    .expect_err("non-controller must reject");
    assert!(
        format!("{err:?}").contains("only the controller can open this attestation"),
        "wrong revert: {err:?}"
    );

    // basis == 0 rejected.
    let err = EvaporVM::execute(
        &bc,
        "seal",
        vec![
            Value::Address(data),
            Value::Address(subject),
            Value::U64(0),
            Value::U64(1),
        ],
        initial_state(&bc),
        &ctx(controller, controller, 100, 10_000),
    )
    .expect_err("zero basis must reject");
    assert!(
        format!("{err:?}").contains("obligation basis code must be positive"),
        "wrong revert: {err:?}"
    );

    // method == 0 rejected.
    let err = EvaporVM::execute(
        &bc,
        "seal",
        vec![
            Value::Address(data),
            Value::Address(subject),
            Value::U64(1),
            Value::U64(0),
        ],
        initial_state(&bc),
        &ctx(controller, controller, 100, 10_000),
    )
    .expect_err("zero method must reject");
    assert!(
        format!("{err:?}").contains("sanitization method code must be positive"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn seal_is_one_shot() {
    let bc = compile_pilot();
    let controller = [0xAAu8; 32];
    let data = [0x11u8; 32];
    let subject = [0xB1u8; 32];
    let after_first = seal(&bc, controller, data, subject, 1, 1);

    let err = EvaporVM::execute(
        &bc,
        "seal",
        vec![
            Value::Address([0x22u8; 32]),
            Value::Address([0xB2u8; 32]),
            Value::U64(2),
            Value::U64(2),
        ],
        after_first,
        &ctx(controller, controller, 200, 10_000),
    )
    .expect_err("second seal must reject");
    assert!(
        format!("{err:?}").contains("attestation already opened"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn attest_erasure_validates_and_is_one_shot() {
    let bc = compile_pilot();
    let controller = [0xAAu8; 32];
    let stranger = [0xCCu8; 32];
    let data = [0x11u8; 32];
    let subject = [0xB1u8; 32];

    // Attest before seal rejects.
    let err = EvaporVM::execute(
        &bc,
        "attest_erasure",
        vec![Value::U64(1)],
        initial_state(&bc),
        &ctx(controller, controller, 100, 10_000),
    )
    .expect_err("attest before seal must reject");
    assert!(
        format!("{err:?}").contains("attestation not yet opened"),
        "wrong revert: {err:?}"
    );

    let sealed_state = seal(&bc, controller, data, subject, 1, 1);

    // Non-owner cannot attest.
    let err = EvaporVM::execute(
        &bc,
        "attest_erasure",
        vec![Value::U64(1)],
        sealed_state.clone(),
        &ctx(stranger, controller, 150, 10_000),
    )
    .expect_err("non-controller attest must reject");
    assert!(
        format!("{err:?}").contains("only the controller can attest erasure"),
        "wrong revert: {err:?}"
    );

    // verification == 0 rejected.
    let err = EvaporVM::execute(
        &bc,
        "attest_erasure",
        vec![Value::U64(0)],
        sealed_state.clone(),
        &ctx(controller, controller, 150, 10_000),
    )
    .expect_err("zero verification must reject");
    assert!(
        format!("{err:?}").contains("verification result code must be positive"),
        "wrong revert: {err:?}"
    );

    // Successful attest flips attested + stamps attested_at.
    let attested = EvaporVM::execute(
        &bc,
        "attest_erasure",
        vec![Value::U64(42)],
        sealed_state,
        &ctx(controller, controller, 200, 10_000),
    )
    .expect("attest must succeed");
    let attested_at_q = EvaporVM::execute(
        &bc,
        "attested_epoch",
        vec![],
        attested.state_changes.clone(),
        &ctx(controller, controller, 300, 10_000),
    )
    .unwrap();
    assert_eq!(attested_at_q.return_value, Value::U64(200));

    // Second attest rejects.
    let err = EvaporVM::execute(
        &bc,
        "attest_erasure",
        vec![Value::U64(99)],
        attested.state_changes,
        &ctx(controller, controller, 250, 10_000),
    )
    .expect_err("second attest must reject");
    assert!(
        format!("{err:?}").contains("erasure already attested"),
        "wrong revert: {err:?}"
    );
}

#[test]
fn status_codes_lifecycle_progression() {
    let bc = compile_pilot();
    let controller = [0xAAu8; 32];
    let data = [0x11u8; 32];
    let subject = [0xB1u8; 32];

    // 0 = not opened.
    let q0 = EvaporVM::execute(
        &bc,
        "status",
        vec![],
        initial_state(&bc),
        &ctx(controller, controller, 100, 10_000),
    )
    .unwrap();
    assert_eq!(q0.return_value, Value::U64(0));

    // 1 = sealed, not yet attested.
    let sealed_state = seal(&bc, controller, data, subject, 1, 1);
    let q1 = EvaporVM::execute(
        &bc,
        "status",
        vec![],
        sealed_state.clone(),
        &ctx(controller, controller, 150, 10_000),
    )
    .unwrap();
    assert_eq!(q1.return_value, Value::U64(1));

    // 2 = attested.
    let attested = EvaporVM::execute(
        &bc,
        "attest_erasure",
        vec![Value::U64(42)],
        sealed_state,
        &ctx(controller, controller, 200, 10_000),
    )
    .unwrap();
    let q2 = EvaporVM::execute(
        &bc,
        "status",
        vec![],
        attested.state_changes,
        &ctx(controller, controller, 250, 10_000),
    )
    .unwrap();
    assert_eq!(q2.return_value, Value::U64(2));
}

#[test]
fn on_evaporate_emits_negative_proof_only_when_unattested() {
    let bc = compile_pilot();
    let controller = [0xAAu8; 32];
    let data = [0x11u8; 32];
    let subject = [0xB1u8; 32];

    // Unattested evap: emit the regulator-grade negative proof.
    let sealed_state = seal(&bc, controller, data, subject, 1, 1);
    let evap_unattested = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        sealed_state.clone(),
        &ctx(controller, controller, 9999, 0),
    )
    .expect("on_evaporate must succeed");
    let saw_neg_proof = evap_unattested
        .events
        .iter()
        .any(|e| format!("{e:?}").contains("obligation window CLOSED with no attestation"));
    assert!(
        saw_neg_proof,
        "unattested evap MUST emit negative-proof event; events: {:?}",
        evap_unattested.events
    );

    // Attested evap: silent on the negative-proof emit (the proof
    // already stands; emitting it on an attested vault would
    // contradict the existing positive proof).
    let attested = EvaporVM::execute(
        &bc,
        "attest_erasure",
        vec![Value::U64(42)],
        sealed_state,
        &ctx(controller, controller, 200, 10_000),
    )
    .unwrap();
    let evap_attested = EvaporVM::execute(
        &bc,
        "on_evaporate",
        vec![],
        attested.state_changes,
        &ctx(controller, controller, 9999, 0),
    )
    .expect("on_evaporate must succeed");
    let saw_neg_proof = evap_attested
        .events
        .iter()
        .any(|e| format!("{e:?}").contains("obligation window CLOSED with no attestation"));
    assert!(
        !saw_neg_proof,
        "attested evap MUST NOT emit negative-proof event; events: {:?}",
        evap_attested.events
    );
}

#[test]
fn audit_fields_survive_attest_intact() {
    // The Certificate of Disposition fields (data_ref, subject,
    // obligation_basis, method, sealed_at) MUST be locked at seal
    // and survive a later attest_erasure intact — attest only adds
    // the attested_at stamp. If any audit field could drift between
    // seal and attest, the certificate's chain of custody is
    // broken.
    let bc = compile_pilot();
    let controller = [0xAAu8; 32];
    let data = [0x11u8; 32];
    let subject = [0xB1u8; 32];

    let sealed_state = seal(&bc, controller, data, subject, 2, 3); // basis=2, method=3
    let attested = EvaporVM::execute(
        &bc,
        "attest_erasure",
        vec![Value::U64(42)],
        sealed_state,
        &ctx(controller, controller, 200, 10_000),
    )
    .unwrap();
    let state = attested.state_changes;

    let data_q = EvaporVM::execute(
        &bc,
        "data_commitment",
        vec![],
        state.clone(),
        &ctx(controller, controller, 300, 10_000),
    )
    .unwrap();
    assert_eq!(data_q.return_value, Value::Address(data));

    let subj_q = EvaporVM::execute(
        &bc,
        "subject",
        vec![],
        state.clone(),
        &ctx(controller, controller, 300, 10_000),
    )
    .unwrap();
    assert_eq!(subj_q.return_value, Value::Address(subject));

    let basis_q = EvaporVM::execute(
        &bc,
        "obligation_basis_code",
        vec![],
        state.clone(),
        &ctx(controller, controller, 300, 10_000),
    )
    .unwrap();
    assert_eq!(basis_q.return_value, Value::U64(2));

    let method_q = EvaporVM::execute(
        &bc,
        "method_code",
        vec![],
        state.clone(),
        &ctx(controller, controller, 300, 10_000),
    )
    .unwrap();
    assert_eq!(method_q.return_value, Value::U64(3));

    let sealed_at_q = EvaporVM::execute(
        &bc,
        "sealed_epoch",
        vec![],
        state,
        &ctx(controller, controller, 300, 10_000),
    )
    .unwrap();
    // seal() ran at epoch 100.
    assert_eq!(sealed_at_q.return_value, Value::U64(100));
}

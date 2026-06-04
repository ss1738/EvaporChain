import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  sealPayload,
  attestErasurePayload,
  statusPayload,
  obligationBasisCodePayload,
  methodCodePayload,
  subjectPayload,
  dataCommitmentPayload,
  attestedEpochPayload,
  sealedEpochPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { ERASURE_ATTESTATION_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 5, energy: 100_000, halfLife: 365 });
  assert.equal(p.deployer, 5);
  assert.equal(p.source_code, ERASURE_ATTESTATION_SOURCE);
  assert.equal(p.energy, 100_000);
  assert.equal(p.half_life, 365);
});

test("seal carries (data_commitment, subject, basis, method) in canonical order", () => {
  const p = sealPayload({
    caller: 5, contractId: 42,
    dataCommitmentHex: "0xdead",
    subjectHex: "0xbeef",
    basis: 1, sanitizeMethod: 1, epoch: 100,
  });
  assert.equal(p.method, "seal");
  assert.deepEqual(p.args, ["0xdead", "0xbeef", 1, 1]);
});

test("attest_erasure carries the verification code", () => {
  const p = attestErasurePayload({
    caller: 5, contractId: 42, verificationCode: 42, epoch: 200,
  });
  assert.equal(p.method, "attest_erasure");
  assert.deepEqual(p.args, [42]);
});

test("no-arg views have correct method names + zero args", () => {
  for (const [fn, name] of [
    [statusPayload, "status"],
    [obligationBasisCodePayload, "obligation_basis_code"],
    [methodCodePayload, "method_code"],
    [subjectPayload, "subject"],
    [dataCommitmentPayload, "data_commitment"],
    [attestedEpochPayload, "attested_epoch"],
    [sealedEpochPayload, "sealed_epoch"],
  ] as const) {
    const p = fn({ caller: 1, contractId: 2, epoch: 3 });
    assert.equal(p.method, name);
    assert.deepEqual(p.args, []);
  }
});

test("endpoint paths match the node API", () => {
  assert.equal(DEPLOY_PATH, "/api/tx/deploy-script");
  assert.equal(CALL_PATH, "/api/tx/call-script");
});

test("ERASURE_ATTESTATION_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "data_ref:",
    "subject_ref:",
    "obligation_basis:",
    "method:",
    "sealed_at:",
    "attested_at:",
    "sealed:",
    "attested:",
    // mutators
    "fn seal(",
    "fn attest_erasure(",
    // views
    "fn status()",
    "fn obligation_basis_code()",
    "fn method_code()",
    "fn subject()",
    "fn data_commitment()",
    "fn attested_epoch()",
    "fn sealed_epoch()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only the controller can open this attestation",
    "attestation already opened",
    "obligation basis code must be positive",
    "sanitization method code must be positive",
    "attestation not yet opened",
    "erasure already attested",
    "only the controller can attest erasure",
    "verification result code must be positive",
    "PROOF-OF-ERASURE: obligation honoured, off-chain sanitization verified",
    "erasure-attestation: obligation window CLOSED with no attestation",
  ]) {
    assert.ok(
      ERASURE_ATTESTATION_SOURCE.includes(name),
      `ERASURE_ATTESTATION_SOURCE missing: ${name}`,
    );
  }
});

test("ERASURE_ATTESTATION_SOURCE: on_evaporate emits NEGATIVE-PROOF only when un-attested", () => {
  // The audit-grade premise — the negative-proof path makes a missed
  // deadline as immutable as the positive — depends on on_evaporate's
  // `if self.attested == false` gate. If this drifts to
  // unconditional emit, every attested vault's natural evaporation
  // would emit a "no attestation" event, CONTRADICTING the positive
  // proof that already stands. Pin the gate AND pin the absence on
  // the attested path.
  const evap = ERASURE_ATTESTATION_SOURCE.slice(
    ERASURE_ATTESTATION_SOURCE.indexOf("on_evaporate()"),
    ERASURE_ATTESTATION_SOURCE.length,
  );
  assert.ok(
    evap.includes("if self.attested == false"),
    "on_evaporate MUST gate the negative-proof emit on attested == false",
  );
  assert.ok(
    evap.includes("obligation window CLOSED with no attestation"),
    "on_evaporate MUST emit the negative-proof marker on the un-attested branch",
  );
});

test("ERASURE_ATTESTATION_SOURCE: seal is controller-only + one-shot + writes 5 audit fields", () => {
  // The Certificate of Disposition fields (data_ref, subject_ref,
  // obligation_basis, method, sealed_at) MUST be locked at seal in
  // ONE atomic call. Pin (a) caller == owner gate, (b) sealed-flag
  // one-shot, (c) all 5 audit fields written, (d) basis > 0 +
  // method > 0 validation gates (without these the audit metadata
  // could be opened as zeros, undermining the certificate).
  const seal = ERASURE_ATTESTATION_SOURCE.slice(
    ERASURE_ATTESTATION_SOURCE.indexOf("fn seal("),
    ERASURE_ATTESTATION_SOURCE.indexOf("fn attest_erasure("),
  );
  assert.ok(
    seal.includes("require(caller == owner, \"only the controller can open this attestation\")"),
    "seal MUST be controller-only",
  );
  assert.ok(
    seal.includes("require(self.sealed == false, \"attestation already opened\")"),
    "seal MUST be one-shot",
  );
  for (const field of [
    "self.data_ref = data_commitment",
    "self.subject_ref = subject",
    "self.obligation_basis = basis",
    "self.method = sanitize_method",
    "self.sealed_at = epoch",
  ]) {
    assert.ok(
      seal.includes(field),
      `seal MUST write audit field: ${field}`,
    );
  }
  assert.ok(
    seal.includes("require(basis > 0"),
    "seal MUST reject basis == 0",
  );
  assert.ok(
    seal.includes("require(sanitize_method > 0"),
    "seal MUST reject method == 0",
  );
});

test("ERASURE_ATTESTATION_SOURCE: attest_erasure is controller-only + one-shot + requires sealed + verification > 0", () => {
  // The positive-proof event — what a regulator/DPO reads as the
  // certificate of compliance — must be: (a) controller-only (so
  // the data subject can't fake an attestation against the
  // controller's wishes); (b) one-shot (so the proof isn't
  // rewritable); (c) requires sealed (so an empty/zero-field
  // certificate can't be attested); (d) requires verification > 0
  // (so the controller can't attest without recording a
  // verification taxonomy code). Pin all four.
  const attest = ERASURE_ATTESTATION_SOURCE.slice(
    ERASURE_ATTESTATION_SOURCE.indexOf("fn attest_erasure("),
    ERASURE_ATTESTATION_SOURCE.indexOf("fn status()"),
  );
  assert.ok(
    attest.includes("require(self.sealed == true, \"attestation not yet opened\")"),
    "attest_erasure MUST require sealed",
  );
  assert.ok(
    attest.includes("require(self.attested == false, \"erasure already attested\")"),
    "attest_erasure MUST be one-shot",
  );
  assert.ok(
    attest.includes("require(caller == owner, \"only the controller can attest erasure\")"),
    "attest_erasure MUST be controller-only",
  );
  assert.ok(
    attest.includes("require(verification_code > 0, \"verification result code must be positive\")"),
    "attest_erasure MUST reject verification_code == 0",
  );
  assert.ok(
    attest.includes("self.attested = true"),
    "attest_erasure MUST flip attested = true",
  );
  assert.ok(
    attest.includes("self.attested_at = epoch"),
    "attest_erasure MUST stamp attested_at",
  );
});

test("ERASURE_ATTESTATION_SOURCE: status() lifecycle ordering 0 -> 1 -> 2", () => {
  // The disposition lifecycle ordering is the canonical regulator-
  // readable status: 0 = not opened, 1 = open (window running, not
  // yet attested), 2 = attested. If the contract ever drifts to a
  // different ordering (e.g. 2 = sealed and 1 = attested), every
  // off-chain consumer breaks. Pin the structural order.
  const status = ERASURE_ATTESTATION_SOURCE.slice(
    ERASURE_ATTESTATION_SOURCE.indexOf("fn status()"),
    ERASURE_ATTESTATION_SOURCE.indexOf("fn obligation_basis_code()"),
  );
  // Pin the unsealed -> 0 path is FIRST (before attested check).
  const idxSealedCheck = status.indexOf("if self.sealed == false");
  const idxAttestedCheck = status.indexOf("if self.attested == true");
  const idxReturn1 = status.indexOf("return 1");
  assert.ok(idxSealedCheck >= 0, "status must check sealed first");
  assert.ok(idxAttestedCheck > idxSealedCheck, "status must check attested AFTER sealed");
  assert.ok(idxReturn1 > idxAttestedCheck, "status must return 1 (open) AFTER attested check fails");
});

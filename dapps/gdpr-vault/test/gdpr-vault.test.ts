import { test } from "node:test";
import assert from "node:assert/strict";
import {
  deployPayload,
  sealPayload,
  withdrawConsentPayload,
  extendRetentionPayload,
  statusPayload,
  lawfulBasisCodePayload,
  subjectPayload,
  ctCommitmentPayload,
  sealedEpochPayload,
  DEPLOY_PATH,
  CALL_PATH,
} from "../src/client.ts";
import { GDPR_VAULT_SOURCE } from "../src/contract.ts";

test("deployPayload carries the contract source + energy config", () => {
  const p = deployPayload({ deployer: 5, energy: 100_000, halfLife: 365 });
  assert.equal(p.deployer, 5);
  assert.equal(p.source_code, GDPR_VAULT_SOURCE);
  assert.equal(p.energy, 100_000);
  assert.equal(p.half_life, 365);
});

test("seal carries (ct_commitment, subject, basis) in canonical order", () => {
  const p = sealPayload({
    caller: 5, contractId: 42, ctCommitmentHex: "0xdead", subjectHex: "0xbeef", basis: 1, epoch: 100,
  });
  assert.equal(p.method, "seal");
  assert.deepEqual(p.args, ["0xdead", "0xbeef", 1]);
});

test("withdraw_consent is a no-arg dual-keyed call", () => {
  const p = withdrawConsentPayload({ caller: 9, contractId: 42, epoch: 150 });
  assert.equal(p.method, "withdraw_consent");
  assert.deepEqual(p.args, []);
});

test("extend_retention is a no-arg controller-only call", () => {
  const p = extendRetentionPayload({ caller: 5, contractId: 42, epoch: 150 });
  assert.equal(p.method, "extend_retention");
  assert.deepEqual(p.args, []);
});

test("no-arg views have correct method names + zero args", () => {
  for (const [fn, name] of [
    [statusPayload, "status"],
    [lawfulBasisCodePayload, "lawful_basis_code"],
    [subjectPayload, "subject"],
    [ctCommitmentPayload, "ct_commitment"],
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

test("GDPR_VAULT_SOURCE contains all methods + lifecycle hooks + doctrine markers", () => {
  for (const name of [
    // state fields
    "ct_commit:",
    "subject_ref:",
    "lawful_basis:",
    "sealed_at:",
    "sealed:",
    "expiry_forced:",
    // mutators
    "fn seal(",
    "fn withdraw_consent()",
    "fn extend_retention()",
    // views
    "fn status()",
    "fn lawful_basis_code()",
    "fn subject()",
    "fn ct_commitment()",
    "fn sealed_epoch()",
    // lifecycle hooks
    "on_grace()",
    "on_refresh()",
    "on_evaporate()",
    // doctrine markers
    "only the controller can seal this vault",
    "vault already sealed",
    "lawful basis code must be positive",
    "vault not yet sealed",
    "erasure already requested",
    "only the data subject or controller can request erasure",
    "only the controller can extend retention",
    "erasure-due: shred key for this ct_commit",
    "erasure-due (consent withdrawn): shred key for this ct_commit",
  ]) {
    assert.ok(
      GDPR_VAULT_SOURCE.includes(name),
      `GDPR_VAULT_SOURCE missing: ${name}`,
    );
  }
});

test("GDPR_VAULT_SOURCE: withdraw_consent is dual-keyed (subject OR controller)", () => {
  // The whole audit-grade premise — Art. 7(3) right cannot be
  // gatekept by the controller — depends on withdraw_consent
  // accepting BOTH the data subject AND the controller (owner). If
  // this ever drifts to `caller == owner` (controller-only), the
  // subject loses their structural erasure path; if it drifts to
  // `caller == self.subject_ref` (subject-only), the controller
  // cannot honour an Art. 17 request on the subject's behalf. Pin
  // the disjunction.
  const wc = GDPR_VAULT_SOURCE.slice(
    GDPR_VAULT_SOURCE.indexOf("fn withdraw_consent()"),
    GDPR_VAULT_SOURCE.indexOf("fn extend_retention()"),
  );
  assert.ok(
    wc.includes("caller == self.subject_ref || caller == owner"),
    "withdraw_consent MUST be dual-keyed (subject_ref OR owner)",
  );
});

test("GDPR_VAULT_SOURCE: extend_retention REJECTS after consent withdrawn", () => {
  // The subject's erasure right cannot be silently overridden by a
  // retention extension. If extend_retention ever stops checking
  // `expiry_forced == false`, a controller could extend retention
  // on a vault the subject already asked to be erased — a
  // catastrophic compliance failure. Pin the gate.
  const er = GDPR_VAULT_SOURCE.slice(
    GDPR_VAULT_SOURCE.indexOf("fn extend_retention()"),
    GDPR_VAULT_SOURCE.indexOf("fn status()"),
  );
  assert.ok(
    er.includes("require(self.expiry_forced == false, \"erasure already requested\")"),
    "extend_retention MUST reject once expiry_forced == true",
  );
});

test("GDPR_VAULT_SOURCE: seal is controller-only + one-shot + audit-recording", () => {
  // The immutable audit trail (ct_commit, subject, basis, sealed_at)
  // is what a DPO/regulator reads. Pin: (a) caller == owner gate,
  // (b) sealed-flag one-shot gate, (c) all 4 audit fields are
  // written. If any of these drift, the audit trail becomes
  // either mutable (catastrophic) or unproduced.
  const seal = GDPR_VAULT_SOURCE.slice(
    GDPR_VAULT_SOURCE.indexOf("fn seal("),
    GDPR_VAULT_SOURCE.indexOf("fn withdraw_consent()"),
  );
  assert.ok(
    seal.includes("require(caller == owner, \"only the controller can seal this vault\")"),
    "seal MUST be controller-only",
  );
  assert.ok(
    seal.includes("require(self.sealed == false, \"vault already sealed\")"),
    "seal MUST be one-shot",
  );
  for (const field of [
    "self.ct_commit = ct_commitment",
    "self.subject_ref = subject",
    "self.lawful_basis = basis",
    "self.sealed_at = epoch",
  ]) {
    assert.ok(
      seal.includes(field),
      `seal MUST write audit field: ${field}`,
    );
  }
});

test("GDPR_VAULT_SOURCE: withdraw_consent emit distinguishes from natural-deadline", () => {
  // The audit log MUST differentiate Art. 7(3) consent withdrawal
  // from natural-retention-end shred. If both paths emitted the
  // identical "erasure-due" string, a regulator couldn't tell
  // which path closed the vault. Pin the "consent withdrawn"
  // marker in withdraw_consent's emit; pin its ABSENCE from
  // on_evaporate's emit.
  const wc = GDPR_VAULT_SOURCE.slice(
    GDPR_VAULT_SOURCE.indexOf("fn withdraw_consent()"),
    GDPR_VAULT_SOURCE.indexOf("fn extend_retention()"),
  );
  assert.ok(
    wc.includes("consent withdrawn"),
    "withdraw_consent emit MUST carry the 'consent withdrawn' marker",
  );

  const evap = GDPR_VAULT_SOURCE.slice(
    GDPR_VAULT_SOURCE.indexOf("on_evaporate()"),
    GDPR_VAULT_SOURCE.length,
  );
  assert.ok(
    !evap.includes("consent withdrawn"),
    "on_evaporate emit MUST NOT carry the 'consent withdrawn' marker (it's the natural-deadline path)",
  );
  assert.ok(
    evap.includes("erasure-due: shred key for this ct_commit"),
    "on_evaporate MUST emit the natural-deadline shred trigger",
  );
});

// ErasureAttestation — chain client. Proof-of-Erasure-as-a-Service
// for the right-to-be-forgotten / AI machine-unlearning frontier
// (Privacy lane). NIST SP 800-88 Certificate of Media Disposition
// (data ref + sanitization METHOD + VERIFICATION result + who/when),
// on-chain and tamper-evident. Pair with `gdpr_vault.es`: GdprVault
// destroys the key (shred trigger); ErasureAttestation immutably
// proves the destruction was performed AND verified.
//
// Why this exists: regulators currently have no standardised way to
// PROVE a deletion deadline was missed (EDPS/NeurIPS/IAPP). This
// contract makes the negative outcome as immutable as the positive —
// the `on_evaporate` emit on an un-attested vault IS the regulator-
// grade negative proof.
//
// Lifecycle:
//   1. Controller deploys with energy + half_life sized to the
//      obligation window.
//   2. Controller one-shot seal(data_commitment, subject, basis,
//      method) opens the attestation. data_commitment is blake3 of
//      the off-chain ciphertext/data (never the data); basis is
//      1=GDPR-Art17, 2=CCPA/AB1008, 3=NIST-program; method is the
//      NIST 800-88 code (1=crypto-shred, 2=clear, 3=purge,
//      4=destroy, 5=ML-unlearn).
//   3. Controller performs the off-chain sanitization.
//   4. Controller one-shot attest_erasure(verification_code) records
//      the immutable proof event a regulator/DPO reads.
//   5. If step 4 never happens before energy decays, on_evaporate
//      emits the regulator-grade NEGATIVE proof
//      "obligation window CLOSED with no attestation".
//
// Status codes (canonical disposition lifecycle):
//   0 = not opened (pre-seal)
//   1 = open (sealed, obligation window running, not yet attested)
//   2 = attested (erasure proven)

import { ERASURE_ATTESTATION_SOURCE } from "./contract.ts";

export const DEPLOY_PATH = "/api/tx/deploy-script";
export const CALL_PATH = "/api/tx/call-script";

export interface DeployPayload {
  deployer: number;
  source_code: string;
  energy: number;
  half_life: number;
}

export interface CallPayload {
  caller: number;
  contract_id: number;
  method: string;
  args: Array<string | number>;
  epoch: number;
}

export function deployPayload(opts: {
  deployer: number;
  energy: number;
  halfLife: number;
}): DeployPayload {
  return {
    deployer: opts.deployer,
    source_code: ERASURE_ATTESTATION_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Controller-only, one-shot: open the attestation. data_commitment
 *  is blake3 of the off-chain data/ciphertext (NEVER the data);
 *  subject is the data-subject address; basis is 1=GDPR-Art17,
 *  2=CCPA/AB1008, 3=NIST-program; method is the NIST 800-88 code
 *  (1=crypto-shred, 2=clear, 3=purge, 4=destroy, 5=ML-unlearn). */
export function sealPayload(opts: {
  caller: number;
  contractId: number;
  dataCommitmentHex: string;
  subjectHex: string;
  basis: number;
  sanitizeMethod: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "seal",
    args: [opts.dataCommitmentHex, opts.subjectHex, opts.basis, opts.sanitizeMethod],
    epoch: opts.epoch,
  };
}

/** Controller-only, one-shot: record the verified-erasure proof
 *  event. The contract is post-fact — the off-chain sanitization
 *  has already been performed AND verified; this records the
 *  certification on-chain. verification_code is the controller's
 *  verification taxonomy (e.g. 1=automated-hash-match,
 *  2=manual-DPO-review, 3=external-auditor-sign-off). */
export function attestErasurePayload(opts: {
  caller: number;
  contractId: number;
  verificationCode: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "attest_erasure",
    args: [opts.verificationCode],
    epoch: opts.epoch,
  };
}

// ── Views ────────────────────────────────────────────────────────

/** View: status code (0=not-opened, 1=sealed-not-attested,
 *  2=attested). */
export function statusPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("status", opts.caller, opts.contractId, opts.epoch);
}

/** View: obligation basis (1=GDPR-Art17, 2=CCPA/AB1008,
 *  3=NIST-program). */
export function obligationBasisCodePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("obligation_basis_code", opts.caller, opts.contractId, opts.epoch);
}

/** View: NIST 800-88 sanitization method code (1=crypto-shred,
 *  2=clear, 3=purge, 4=destroy, 5=ML-unlearn). */
export function methodCodePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("method_code", opts.caller, opts.contractId, opts.epoch);
}

/** View: data-subject address locked at seal time. */
export function subjectPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("subject", opts.caller, opts.contractId, opts.epoch);
}

/** View: 32-byte data commitment (blake3 of off-chain data) locked
 *  at seal time. NEVER the personal data itself. */
export function dataCommitmentPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("data_commitment", opts.caller, opts.contractId, opts.epoch);
}

/** View: the epoch at which `attest_erasure()` ran (0 if not yet
 *  attested). */
export function attestedEpochPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("attested_epoch", opts.caller, opts.contractId, opts.epoch);
}

/** View: the epoch at which `seal()` ran. */
export function sealedEpochPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("sealed_epoch", opts.caller, opts.contractId, opts.epoch);
}

function noArgCall(method: string, caller: number, contractId: number, epoch: number): CallPayload {
  return { caller, contract_id: contractId, method, args: [], epoch };
}

// Auth-injected POST: reads the session token from localStorage
// (set by `dapps/wallet/`) and adds the Authorization header.
// See `dapps/shared/auth.ts` for the contract.
import { authedPost, type TxResponse } from "../../shared/auth.ts";
export type { TxResponse };

const post = authedPost;

export const deployTx = (baseUrl: string, o: Parameters<typeof deployPayload>[0]) =>
  post(baseUrl, DEPLOY_PATH, deployPayload(o));
export const sealTx = (baseUrl: string, o: Parameters<typeof sealPayload>[0]) =>
  post(baseUrl, CALL_PATH, sealPayload(o));
export const attestErasureTx = (baseUrl: string, o: Parameters<typeof attestErasurePayload>[0]) =>
  post(baseUrl, CALL_PATH, attestErasurePayload(o));

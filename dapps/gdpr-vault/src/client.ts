// GdprVault — chain client. GDPR Erasure-as-a-Service via crypto-
// shred (Privacy lane). The chain holds NO personal data — only a
// 32-byte ciphertext commitment + the consent/retention lifecycle
// (Dead Drop §9 founding constraint, verified 2026-05-17). The
// contract's own energy IS the retention clock; on_evaporate emits
// the natural-deadline shred trigger that off-chain key-custody/HSM
// subscribes to.
//
// Lifecycle:
//   1. Controller deploys with energy + half_life sized to the
//      retention period.
//   2. Controller calls one-shot seal(ct_commit, subject, basis) to
//      bind the audit fields. ct_commit is blake3 of the off-chain
//      ciphertext (never the data); basis is the Art. 6 lawful-basis
//      code (1=consent, 2=contract, 3=legal-obligation,
//      6=legitimate-interest).
//   3. (a) Natural path: energy decays out, on_evaporate emits
//      "erasure-due: shred key for this ct_commit". HSM destroys
//      the decryption key. Ciphertext is permanently inaccessible.
//   3. (b) Early-erasure path: subject OR controller calls
//      withdraw_consent() to fire the shred trigger NOW (Art. 7(3) /
//      Art. 17). Distinguishable from natural-deadline by the
//      "consent withdrawn" marker in the emit.
//
// Audit-grade properties (regulator-readable):
//   - withdraw_consent is dual-keyed — subject's Art. 7(3) right
//     cannot be gatekept by the controller.
//   - extend_retention is controller-only AND rejects once consent
//     is withdrawn — the subject's erasure right cannot be silently
//     overridden by a retention extension.
//   - All audit views (ct_commitment, subject, lawful_basis_code,
//     sealed_epoch) survive a withdraw_consent intact.

import { GDPR_VAULT_SOURCE } from "./contract.ts";

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
    source_code: GDPR_VAULT_SOURCE,
    energy: opts.energy,
    half_life: opts.halfLife,
  };
}

/** Controller-only, one-shot: bind the audit fields. ct_commitment is
 *  blake3 of the off-chain ciphertext (NEVER the data); subject is
 *  the data-subject address; basis is the Art. 6 lawful-basis code
 *  (1=consent, 2=contract, 3=legal-obligation, 6=legitimate-interest). */
export function sealPayload(opts: {
  caller: number;
  contractId: number;
  ctCommitmentHex: string;
  subjectHex: string;
  basis: number;
  epoch: number;
}): CallPayload {
  return {
    caller: opts.caller,
    contract_id: opts.contractId,
    method: "seal",
    args: [opts.ctCommitmentHex, opts.subjectHex, opts.basis],
    epoch: opts.epoch,
  };
}

/** Open to subject OR controller (Art. 7(3) right cannot be
 *  gatekept): fire the shred trigger NOW, before the natural
 *  retention deadline. Idempotent — second call rejects. */
export function withdrawConsentPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("withdraw_consent", opts.caller, opts.contractId, opts.epoch);
}

/** Controller-only: log a lawful retention extension (Art. 5(1)(e)
 *  storage limitation). The chain applies the actual energy refresh;
 *  this records the event for the audit trail. REJECTS once consent
 *  has been withdrawn — the subject's erasure right cannot be
 *  silently overridden. */
export function extendRetentionPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("extend_retention", opts.caller, opts.contractId, opts.epoch);
}

// ── Views ────────────────────────────────────────────────────────

/** View: status code (0=unsealed, 1=erasure-forced, 2=sealed-normal). */
export function statusPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("status", opts.caller, opts.contractId, opts.epoch);
}

/** View: the Art. 6 lawful-basis code locked at seal time. */
export function lawfulBasisCodePayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("lawful_basis_code", opts.caller, opts.contractId, opts.epoch);
}

/** View: the data-subject address locked at seal time. */
export function subjectPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("subject", opts.caller, opts.contractId, opts.epoch);
}

/** View: the 32-byte ciphertext commitment (blake3 of off-chain
 *  ciphertext) locked at seal time. NEVER the personal data itself. */
export function ctCommitmentPayload(opts: {
  caller: number;
  contractId: number;
  epoch: number;
}): CallPayload {
  return noArgCall("ct_commitment", opts.caller, opts.contractId, opts.epoch);
}

/** View: the epoch at which `seal()` ran (for the audit log start
 *  marker; the deadline is observable via the contract's terminal
 *  evaporated state). */
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
export const withdrawConsentTx = (baseUrl: string, o: Parameters<typeof withdrawConsentPayload>[0]) =>
  post(baseUrl, CALL_PATH, withdrawConsentPayload(o));
export const extendRetentionTx = (baseUrl: string, o: Parameters<typeof extendRetentionPayload>[0]) =>
  post(baseUrl, CALL_PATH, extendRetentionPayload(o));

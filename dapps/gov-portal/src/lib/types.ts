/**
 * Off-chain proposal types for the EvaporChain governance portal.
 *
 * Proposals live in `data/proposals.json` until they accumulate enough
 * validator endorsements to satisfy `required_stake`. At that point any user
 * can hit Submit, which fires the canonical chain endpoint
 * (`/api/governance/fork_choice_mode` for fork-choice changes,
 * `/api/tx/upgrade_contract` governance-path for contract upgrades). The
 * chain re-validates the stake quorum from its own validator registry — the
 * portal's stake sum is purely a UX gate, not an auth check.
 */

/** Singh-Attractor basin (mirrors api.rs::AttractorReq). */
export interface AttractorSpec {
  center: number;
  basin_radius: number;
}

/** Payload for `/api/governance/fork_choice_mode` POST. */
export interface ForkChoicePayload {
  mode: "mcc" | "singh_attractor";
  attractors: AttractorSpec[];
}

/**
 * Payload for `/api/tx/upgrade_contract` POST (governance path — no
 * admin_signature_hex). The chain enforces
 * `sum(endorser_stakes) >= required_stake` at apply time.
 */
export interface UpgradeContractPayload {
  /** Sender (proposer) hex address — pays gas for the tx. */
  owner: string;
  contract_id: number;
  /** Hex-encoded raw bytecode bytes (we use new_bytecode_hex, not new_bytecode). */
  new_bytecode_hex: string;
  /** BLAKE3(new_bytecode) — 64 hex chars (32 bytes). The chain re-checks. */
  new_bytecode_hash_hex: string;
  nonce: number;
}

export interface Endorsement {
  /** Validator id (numeric, from /api/validators) — optional for paste-address mode. */
  validator_id?: number;
  /** Validator's hex address (32-byte) used as the canonical identity. */
  address: string;
  /** Stake at the time of endorsement (snapshot). */
  stake: number;
  /** ISO timestamp the validator signed. */
  signed_at: number;
  /**
   * Hex signature over `endorsementSignablePayload(...)`. The chain ignores
   * this; the portal records it so the off-chain endorsement chain is
   * verifiable independently of the chain's own quorum check.
   */
  signature_hex?: string;
  /** Hex public key matching the signature (optional in paste-mode). */
  public_key_hex?: string;
}

export type ProposalState = "open" | "active" | "historical";
export type ProposalType = "fork_choice" | "upgrade_contract";

export interface Proposal {
  id: string;
  type: ProposalType;
  title: string;
  /** Markdown explanation. */
  rationale: string;
  payload: ForkChoicePayload | UpgradeContractPayload;
  required_stake: number;
  endorsements: Endorsement[];
  state: ProposalState;
  on_chain_tx_hash?: string;
  /** Why a historical proposal lost — quorum failed, superseded, rejected, etc. */
  historical_reason?: string;
  created_at: number;
  updated_at: number;
}

/**
 * Canonical signing payload for an endorsement. Validators sign the UTF-8
 * bytes of this string with their ML-DSA-65 key. The portal stores the
 * resulting hex sig in Endorsement.signature_hex.
 */
export function endorsementSignablePayload(
  proposalId: string,
  address: string,
  stake: number,
): string {
  return `{"type":"gov_endorse","proposal_id":"${proposalId}","endorser":"${address}","stake":${stake}}`;
}

/**
 * Gov-portal client wrapper around @evaporchain/wallet-sdk.
 *
 * The SDK already covers fork-choice GET/POST and the validator list. The
 * UpgradeContract POST has no SDK helper yet, so we hit it directly via the
 * underlying fetch — body shape mirrors api.rs::UpgradeContractRequest with
 * the governance-path fields populated (no admin_signature_hex).
 */

import { EvaporChainAPI, type Validator } from "@evaporchain/wallet-sdk";
import type {
  ForkChoicePayload,
  Proposal,
  UpgradeContractPayload,
} from "./types";

let _client: EvaporChainAPI | null = null;

export function getClient(): EvaporChainAPI {
  if (!_client) {
    _client = new EvaporChainAPI({
      rpcUrl:
        typeof window !== "undefined"
          ? window.location.origin
          : "https://testnet.evaporchain.com",
    });
  }
  return _client;
}

export const govApi = {
  api: getClient(),

  async chainStatus() {
    return getClient().getChainStatus();
  },

  async getForkChoiceMode() {
    return getClient().getForkChoiceMode();
  },

  async getValidators(): Promise<Validator[]> {
    return getClient().getValidators();
  },

  /**
   * Look up a validator by hex address from the active validator list.
   * Returns null if the address is not currently active.
   */
  async findValidatorByAddress(address: string): Promise<Validator | null> {
    const all = await this.getValidators();
    const norm = address.toLowerCase();
    return (
      all.find((v) => (v.address ?? "").toLowerCase() === norm) ?? null
    );
  },

  /** Submit a fork-choice amendment via the SDK helper. */
  async submitForkChoice(
    payload: ForkChoicePayload,
    endorserStakes: number[],
    requiredStake: number,
  ): Promise<unknown> {
    return getClient().submitForkChoiceAmend({
      mode: payload.mode,
      attractors: payload.attractors.map((a) => ({
        center: a.center,
        basinRadius: a.basin_radius,
      })),
      endorserStakes,
      requiredStake,
    });
  },

  /**
   * Submit a contract upgrade on the governance path. No SDK helper yet, so
   * we POST directly. `admin_signature_hex` and `admin_public_key_hex` are
   * intentionally omitted; the chain reads that as "governance path" and
   * enforces sum(endorser_stakes) >= required_stake.
   */
  async submitUpgradeContract(
    payload: UpgradeContractPayload,
    endorserStakes: number[],
    requiredStake: number,
  ): Promise<{ success: boolean; message: string; tx_hash?: string }> {
    const base =
      typeof window !== "undefined"
        ? window.location.origin
        : "https://testnet.evaporchain.com";
    const res = await fetch(`${base}/api/tx/upgrade_contract`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        owner: payload.owner,
        contract_id: payload.contract_id,
        new_bytecode_hex: payload.new_bytecode_hex,
        new_bytecode_hash_hex: payload.new_bytecode_hash_hex,
        nonce: payload.nonce,
        endorser_stakes: endorserStakes,
        required_stake: requiredStake,
      }),
    });
    return res.json();
  },
};

/** Submit a proposal to the chain based on its type. */
export async function broadcastProposal(p: Proposal, endorserStakes: number[]) {
  if (p.type === "fork_choice") {
    return govApi.submitForkChoice(
      p.payload as ForkChoicePayload,
      endorserStakes,
      p.required_stake,
    );
  }
  return govApi.submitUpgradeContract(
    p.payload as UpgradeContractPayload,
    endorserStakes,
    p.required_stake,
  );
}

"use client";

import { useState } from "react";
import { CheckCircle2, Loader2 } from "lucide-react";
import { AddressInput } from "./AddressInput";
import {
  endorsementSignablePayload,
  type Endorsement,
  type Proposal,
} from "@/lib/types";
import type { Validator } from "@evaporchain/wallet-sdk";

interface Props {
  proposal: Proposal;
  /** Called on successful PATCH so the parent re-fetches the proposal. */
  onEndorsed: () => void;
}

/**
 * Validator endorsement widget. Three steps:
 *   1. Validator pastes their hex address; we look it up via /api/validators
 *      to confirm they're active and snapshot their current stake.
 *   2. They click Endorse; the browser asks the wallet provider (when one
 *      is injected) to signMessage(endorsementSignablePayload). With no
 *      wallet provider, paste-mode skips the signature — useful for testnet
 *      coordinators bootstrapping early proposals.
 *   3. PATCH /api/proposals/[id] with action=endorse appends the row.
 *
 * The endorsement signature is purely off-chain provenance; the chain's
 * /api/governance/fork_choice_mode and /api/tx/upgrade_contract endpoints
 * accept raw stake numbers without per-validator sigs (they trust their
 * own validator registry).
 */
export function EndorseButton({ proposal, onEndorsed }: Props) {
  const [address, setAddress] = useState("");
  const [validator, setValidator] = useState<Validator | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  const stake = validator?.stake ?? 0;
  const canEndorse = address.length >= 8 && stake > 0 && !submitting;

  const handleEndorse = async () => {
    setError(null);
    setSubmitting(true);
    try {
      const payload = endorsementSignablePayload(proposal.id, address, stake);
      // If a wallet provider is injected, ask it to sign. Otherwise we record
      // an unsigned endorsement and rely on the chain's quorum check.
      let signature_hex: string | undefined;
      let public_key_hex: string | undefined;
      const injected = (
        typeof window !== "undefined"
          ? (window as unknown as {
              evaporchain?: {
                signMessage?: (msg: string) => Promise<{
                  signature_hex: string;
                  public_key_hex: string;
                }>;
              };
            }).evaporchain
          : undefined
      );
      if (injected?.signMessage) {
        const sig = await injected.signMessage(payload);
        signature_hex = sig.signature_hex;
        public_key_hex = sig.public_key_hex;
      }

      const endorsement: Endorsement = {
        address,
        stake,
        signed_at: Date.now(),
        signature_hex,
        public_key_hex,
      };
      const res = await fetch(
        `/api/proposals/${encodeURIComponent(proposal.id)}`,
        {
          method: "PATCH",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ action: "endorse", endorsement }),
        },
      );
      if (!res.ok) {
        const j = (await res.json().catch(() => null)) as { error?: string } | null;
        throw new Error(j?.error || `HTTP ${res.status}`);
      }
      setDone(true);
      onEndorsed();
    } catch (e) {
      setError(e instanceof Error ? e.message : "endorse failed");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="rounded-xl border border-evap-border bg-white p-4">
      <div className="mb-3">
        <p className="text-sm font-semibold text-zinc-900">Endorse this proposal</p>
        <p className="mt-0.5 text-[11px] text-zinc-500">
          Active validators can pile stake onto an open proposal. The portal
          aggregates stakes; the chain re-validates quorum at submit time.
        </p>
      </div>

      <div className="space-y-3">
        <AddressInput
          value={address}
          onChange={(addr, v) => {
            setAddress(addr);
            setValidator(v);
          }}
        />
        {validator && (
          <div className="rounded-lg border border-evap-border bg-zinc-50 px-3 py-2">
            <div className="flex items-center justify-between text-[11px]">
              <span className="text-zinc-500">Endorsing with stake</span>
              <span className="font-mono font-semibold text-zinc-900">
                {stake.toLocaleString()} EVAP
              </span>
            </div>
          </div>
        )}

        {error && (
          <div className="rounded-lg border border-evap-red/30 bg-red-50 px-3 py-2 text-[11px] text-evap-red">
            {error}
          </div>
        )}

        <button
          onClick={handleEndorse}
          disabled={!canEndorse}
          className="flex w-full items-center justify-center gap-2 rounded-lg bg-gradient-to-r from-evap-violet to-evap-cyan px-4 py-2 text-xs font-semibold text-white hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {submitting ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : done ? (
            <>
              <CheckCircle2 className="h-4 w-4" />
              Endorsement recorded
            </>
          ) : (
            "Sign & endorse"
          )}
        </button>
      </div>
    </div>
  );
}

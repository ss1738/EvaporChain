"use client";

import { useState } from "react";
import { Loader2, Send } from "lucide-react";
import { broadcastProposal } from "@/lib/api";
import type { Proposal } from "@/lib/types";

interface Props {
  proposal: Proposal;
  endorsedStake: number;
  onSubmitted: (txHash: string) => void;
}

/**
 * Broadcast button. Active only when the off-chain endorsement sum hits
 * required_stake. On success the chain returns either a tx_hash (for
 * upgrade_contract) or { status: "amended", ... } (for fork_choice_mode);
 * we fold both shapes into a single hash string and PATCH the proposal to
 * `active`.
 */
export function SubmitButton({ proposal, endorsedStake, onSubmitted }: Props) {
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ready = endorsedStake >= proposal.required_stake && proposal.state === "open";

  const handleSubmit = async () => {
    setError(null);
    setSubmitting(true);
    try {
      const stakes = proposal.endorsements.map((e) => e.stake);
      const result = (await broadcastProposal(proposal, stakes)) as {
        status?: string;
        success?: boolean;
        message?: string;
        detail?: string;
        tx_hash?: string;
        fork_choice_mode?: string;
      };
      // upgrade_contract returns { success, message, tx_hash }; fork_choice
      // returns { status: "amended"|"error", detail, fork_choice_mode? }.
      const accepted =
        result.status === "amended" || result.success === true || !!result.tx_hash;
      if (!accepted) {
        throw new Error(result.detail || result.message || "chain rejected");
      }
      const txHash =
        result.tx_hash ||
        `forkchoice-${proposal.id.slice(0, 12)}`;
      // Mark proposal active.
      await fetch(`/api/proposals/${encodeURIComponent(proposal.id)}`, {
        method: "PATCH",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ action: "activate", on_chain_tx_hash: txHash }),
      });
      onSubmitted(txHash);
    } catch (e) {
      setError(e instanceof Error ? e.message : "submit failed");
    } finally {
      setSubmitting(false);
    }
  };

  if (proposal.state === "active") {
    return (
      <div className="rounded-lg border border-evap-emerald/30 bg-emerald-50 px-3 py-2 text-[11px] text-evap-emerald">
        Active on-chain. tx{" "}
        <span className="font-mono">{proposal.on_chain_tx_hash}</span>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <button
        onClick={handleSubmit}
        disabled={!ready || submitting}
        className="flex w-full items-center justify-center gap-2 rounded-lg bg-evap-emerald px-4 py-2 text-xs font-semibold text-white hover:bg-emerald-600 disabled:cursor-not-allowed disabled:opacity-40"
      >
        {submitting ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <>
            <Send className="h-3.5 w-3.5" />
            Broadcast amendment
          </>
        )}
      </button>
      {!ready && (
        <p className="text-[10px] text-zinc-500">
          Submit unlocks once the endorsed stake reaches{" "}
          <span className="font-mono">{proposal.required_stake.toLocaleString()}</span>.
        </p>
      )}
      {error && (
        <p className="text-[10px] text-evap-red">{error}</p>
      )}
    </div>
  );
}

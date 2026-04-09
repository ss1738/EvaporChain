import { useState } from "react";
import { vote } from "@/utils/api";
import type { Proposal } from "@/utils/types";

interface Props {
  proposal: Proposal;
  voterAddress: string | null;
  onClose: () => void;
  onVoted: () => void;
}

export function VoteModal({ proposal, voterAddress, onClose, onVoted }: Props) {
  const [direction, setDirection] = useState<"for" | "against">("for");
  const [energyBoost, setEnergyBoost] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async () => {
    if (!voterAddress) {
      setError("Connect your wallet first");
      return;
    }

    setSubmitting(true);
    setError(null);
    try {
      const result = await vote({
        proposal_id: proposal.id,
        voter: voterAddress,
        direction,
        energy_boost: energyBoost ? parseInt(energyBoost) : 0,
      });
      if (result.success) {
        onVoted();
        onClose();
      } else {
        setError(result.message ?? "Vote failed");
      }
    } catch {
      setError("Failed to submit vote");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center px-4">
      <div className="fixed inset-0 bg-black/30" onClick={onClose} />
      <div className="relative bg-white rounded-2xl border border-evap-border w-full max-w-md p-6 shadow-xl">
        <h2 className="text-lg font-bold text-zinc-900 mb-1">Cast Your Vote</h2>
        <p className="text-xs text-zinc-400 mb-5 truncate">
          {proposal.title}
        </p>

        {/* Direction */}
        <div className="grid grid-cols-2 gap-2 mb-4">
          <button
            onClick={() => setDirection("for")}
            className={`py-3 rounded-xl text-sm font-medium border-2 transition-colors ${
              direction === "for"
                ? "border-evap-green bg-evap-green/5 text-evap-green"
                : "border-evap-border text-zinc-400 hover:border-zinc-300"
            }`}
          >
            ✓ Vote For
          </button>
          <button
            onClick={() => setDirection("against")}
            className={`py-3 rounded-xl text-sm font-medium border-2 transition-colors ${
              direction === "against"
                ? "border-evap-red bg-evap-red/5 text-evap-red"
                : "border-evap-border text-zinc-400 hover:border-zinc-300"
            }`}
          >
            ✕ Vote Against
          </button>
        </div>

        {/* Optional energy boost */}
        <div className="mb-4">
          <label className="text-[10px] text-zinc-400 uppercase tracking-wider block mb-1">
            Energy Boost (optional)
          </label>
          <input
            type="number"
            value={energyBoost}
            onChange={(e) => setEnergyBoost(e.target.value)}
            placeholder="0"
            min="0"
            className="w-full px-3 py-2 rounded-lg border border-evap-border text-sm text-zinc-900 focus:outline-none focus:border-evap-cyan transition-colors"
          />
          <p className="text-[10px] text-zinc-400 mt-1">
            Boost proposal energy to keep it alive longer. Costs EVAP from your balance.
          </p>
        </div>

        {!voterAddress && (
          <div className="mb-4 px-3 py-2 rounded-lg bg-evap-amber/10 text-[10px] text-evap-amber">
            Connect your wallet to vote
          </div>
        )}

        {error && (
          <div className="mb-4 px-3 py-2 rounded-lg bg-evap-red/10 text-[10px] text-evap-red">
            {error}
          </div>
        )}

        {/* Actions */}
        <div className="flex items-center gap-2">
          <button
            onClick={onClose}
            className="flex-1 py-2.5 rounded-xl border border-evap-border text-sm text-zinc-500 hover:bg-zinc-50 transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            disabled={submitting || !voterAddress}
            className={`flex-1 py-2.5 rounded-xl text-sm font-medium text-white transition-colors ${
              direction === "for"
                ? "bg-evap-green hover:bg-evap-green/90"
                : "bg-evap-red hover:bg-evap-red/90"
            } disabled:opacity-50`}
          >
            {submitting ? "Submitting..." : `Vote ${direction === "for" ? "For" : "Against"}`}
          </button>
        </div>
      </div>
    </div>
  );
}

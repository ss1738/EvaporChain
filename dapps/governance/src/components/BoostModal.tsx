import { useState } from "react";
import { boostProposal } from "@/utils/api";
import type { Proposal } from "@/utils/types";

interface Props {
  proposal: Proposal;
  onClose: () => void;
  onBoosted: () => void;
}

const PRESETS = [100, 500, 1000, 5000];

export function BoostModal({ proposal, onClose, onBoosted }: Props) {
  const [amount, setAmount] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const energyPct = proposal.max_energy > 0
    ? (proposal.current_energy / proposal.max_energy) * 100
    : 0;

  const handleSubmit = async () => {
    const val = parseInt(amount);
    if (!val || val <= 0) {
      setError("Enter a valid energy amount");
      return;
    }

    setSubmitting(true);
    setError(null);
    try {
      const result = await boostProposal(proposal.id, val);
      if (result.success) {
        onBoosted();
        onClose();
      } else {
        setError(result.message ?? "Boost failed");
      }
    } catch {
      setError("Failed to boost proposal");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center px-4">
      <div className="fixed inset-0 bg-black/30" onClick={onClose} />
      <div className="relative bg-white rounded-2xl border border-evap-border w-full max-w-md p-6 shadow-xl">
        <h2 className="text-lg font-bold text-zinc-900 mb-1">Boost Proposal</h2>
        <p className="text-xs text-zinc-400 mb-4 truncate">{proposal.title}</p>

        {/* Current energy */}
        <div className="mb-4 p-3 rounded-lg bg-zinc-50">
          <div className="flex items-center justify-between text-[10px] mb-1">
            <span className="text-zinc-500">Current Energy</span>
            <span className={`font-medium ${energyPct > 50 ? "text-evap-cyan" : energyPct > 20 ? "text-evap-amber" : "text-evap-red"}`}>
              {proposal.current_energy.toLocaleString()} / {proposal.max_energy.toLocaleString()}
            </span>
          </div>
          <div className="h-2 rounded-full bg-zinc-200 overflow-hidden">
            <div
              className={`h-full rounded-full ${energyPct > 50 ? "bg-evap-cyan" : energyPct > 20 ? "bg-evap-amber" : "bg-evap-red"}`}
              style={{ width: `${energyPct}%` }}
            />
          </div>
        </div>

        {/* Presets */}
        <div className="grid grid-cols-4 gap-2 mb-3">
          {PRESETS.map((p) => (
            <button
              key={p}
              onClick={() => setAmount(String(p))}
              className={`py-1.5 rounded-lg border text-xs font-medium transition-colors ${
                amount === String(p)
                  ? "border-evap-amber bg-evap-amber/5 text-evap-amber"
                  : "border-evap-border text-zinc-500 hover:border-zinc-300"
              }`}
            >
              {p.toLocaleString()}
            </button>
          ))}
        </div>

        {/* Custom amount */}
        <input
          type="number"
          value={amount}
          onChange={(e) => setAmount(e.target.value)}
          placeholder="Custom amount"
          min="1"
          className="w-full px-3 py-2 rounded-lg border border-evap-border text-sm text-zinc-900 focus:outline-none focus:border-evap-amber mb-1 transition-colors"
        />
        <p className="text-[10px] text-zinc-400 mb-4">
          Energy extends the proposal&apos;s lifetime. Without energy, it will evaporate.
        </p>

        {error && (
          <div className="mb-4 px-3 py-2 rounded-lg bg-evap-red/10 text-[10px] text-evap-red">
            {error}
          </div>
        )}

        <div className="flex items-center gap-2">
          <button
            onClick={onClose}
            className="flex-1 py-2.5 rounded-xl border border-evap-border text-sm text-zinc-500 hover:bg-zinc-50 transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            disabled={submitting}
            className="flex-1 py-2.5 rounded-xl bg-evap-amber text-white text-sm font-medium hover:bg-evap-amber/90 transition-colors disabled:opacity-50"
          >
            {submitting ? "Boosting..." : "Boost Energy"}
          </button>
        </div>
      </div>
    </div>
  );
}

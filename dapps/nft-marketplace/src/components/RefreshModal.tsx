import { useState } from "react";
import { api } from "@/utils/api";
import type { Nft } from "@/utils/types";

interface RefreshModalProps {
  nft: Nft;
  onClose: () => void;
  onRefreshed: () => void;
}

export function RefreshModal({ nft, onClose, onRefreshed }: RefreshModalProps) {
  const [energy, setEnergy] = useState("5000");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const energyPercent = nft.max_energy > 0
    ? Math.round((nft.current_energy / nft.max_energy) * 100)
    : 0;

  const handleRefresh = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");

    const energyVal = parseInt(energy);
    if (isNaN(energyVal) || energyVal < 1) return setError("Energy must be at least 1");

    setLoading(true);
    try {
      const result = await api.refreshNft(nft.id, energyVal);
      if (result.success) {
        onRefreshed();
        onClose();
      } else {
        setError(result.message);
      }
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  const isResurrection = nft.state === "Ghost";

  return (
    <div className="fixed inset-0 bg-black/40 flex items-center justify-center z-50 p-4" onClick={onClose}>
      <div
        className="bg-white rounded-2xl shadow-xl w-full max-w-sm overflow-hidden"
        onClick={e => e.stopPropagation()}
      >
        <div className="px-6 py-4 border-b border-evap-border">
          <h2 className="text-lg font-semibold text-zinc-900">
            {isResurrection ? "Resurrect NFT" : "Refresh Energy"}
          </h2>
          <p className="text-xs text-zinc-500 mt-0.5">
            {isResurrection
              ? `Bring "${nft.name}" back from the dead`
              : `Add energy to keep "${nft.name}" alive`}
          </p>
        </div>

        <form onSubmit={handleRefresh} className="px-6 py-4 space-y-4">
          {/* Current state */}
          <div className="px-3 py-3 rounded-lg bg-zinc-50 border border-zinc-100">
            <div className="flex items-center justify-between mb-2">
              <span className="text-xs text-zinc-600">{nft.name}</span>
              <span className={`text-[10px] px-2 py-0.5 rounded-full ${
                nft.state === "Ghost"
                  ? "bg-zinc-200 text-zinc-500"
                  : nft.state === "Grace"
                  ? "bg-amber-50 text-evap-amber"
                  : "bg-green-50 text-evap-green"
              }`}>
                {nft.state}
              </span>
            </div>
            {!isResurrection && (
              <>
                <div className="w-full h-2 bg-zinc-200 rounded-full overflow-hidden mb-1">
                  <div
                    className={`h-full rounded-full transition-all ${
                      energyPercent > 50 ? "bg-evap-green"
                      : energyPercent > 20 ? "bg-evap-amber"
                      : "bg-evap-red"
                    }`}
                    style={{ width: `${Math.max(energyPercent, 1)}%` }}
                  />
                </div>
                <p className="text-[10px] text-zinc-400">
                  {nft.current_energy.toLocaleString()} / {nft.max_energy.toLocaleString()} energy ({energyPercent}%)
                </p>
              </>
            )}
          </div>

          {/* Energy input */}
          <div>
            <label className="text-xs font-medium text-zinc-700 mb-1 block">
              Energy to {isResurrection ? "resurrect with" : "add"}
            </label>
            <input
              type="number"
              min={1}
              max={1000000000}
              value={energy}
              onChange={e => setEnergy(e.target.value)}
              className="input"
              autoFocus
            />
          </div>

          {/* Quick amounts */}
          <div className="flex gap-2">
            {[1000, 5000, 10000, 50000].map(amt => (
              <button
                key={amt}
                type="button"
                onClick={() => setEnergy(String(amt))}
                className="flex-1 py-1.5 rounded-lg bg-zinc-50 border border-zinc-200 text-[10px] text-zinc-600 hover:bg-zinc-100 transition"
              >
                {(amt / 1000)}k
              </button>
            ))}
          </div>

          {error && (
            <p className="text-xs text-evap-red bg-red-50 px-3 py-2 rounded-lg">{error}</p>
          )}

          <div className="flex gap-2 pt-2">
            <button
              type="button"
              onClick={onClose}
              className="flex-1 py-2.5 rounded-lg border border-evap-border text-sm text-zinc-600 hover:bg-zinc-50 transition"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={loading}
              className={`flex-1 py-2.5 rounded-lg text-sm font-semibold text-white hover:opacity-90 transition disabled:opacity-50 ${
                isResurrection
                  ? "bg-gradient-to-r from-zinc-600 to-zinc-800"
                  : "bg-gradient-to-r from-evap-cyan to-evap-green"
              }`}
            >
              {loading
                ? isResurrection ? "Resurrecting..." : "Refreshing..."
                : isResurrection ? "Resurrect" : "Refresh Energy"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

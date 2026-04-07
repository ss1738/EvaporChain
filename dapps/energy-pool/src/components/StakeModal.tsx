import { useState } from "react";
import { api } from "@/utils/api";
import type { Pool } from "@/utils/types";
import { EnergyGauge } from "./EnergyGauge";

interface StakeModalProps {
  pool: Pool;
  onClose: () => void;
  onStaked: () => void;
}

type Mode = "stake" | "unstake";

const QUICK_AMOUNTS = [100, 500, 1000, 5000];

export function StakeModal({ pool, onClose, onStaked }: StakeModalProps) {
  const [mode, setMode] = useState<Mode>("stake");
  const [amount, setAmount] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async () => {
    const numAmount = Number(amount);
    if (!numAmount || numAmount <= 0) {
      setError("Enter a valid energy amount");
      return;
    }

    setSubmitting(true);
    setError(null);

    try {
      const result =
        mode === "stake"
          ? await api.stakeEnergy(pool.id, numAmount)
          : await api.unstakeEnergy(pool.id, numAmount);

      if (result.success) {
        onStaked();
        onClose();
      } else {
        setError(result.message);
      }
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : `Failed to ${mode}`;
      setError(message);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm">
      <div className="bg-white rounded-2xl border border-evap-border shadow-xl w-full max-w-md mx-4 p-6">
        {/* Header */}
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-base font-bold text-zinc-900">
            {mode === "stake" ? "Stake Energy" : "Unstake Energy"}
          </h2>
          <button
            onClick={onClose}
            className="text-zinc-400 hover:text-zinc-600 text-lg"
          >
            x
          </button>
        </div>

        {/* Pool info */}
        <div className="flex items-center gap-4 p-3 rounded-lg bg-zinc-50 border border-evap-border mb-5">
          <EnergyGauge percent={pool.health_pct} size={48} strokeWidth={4} />
          <div>
            <p className="text-sm font-semibold text-zinc-900">{pool.name}</p>
            <p className="text-[10px] text-zinc-400">
              {pool.total_energy.toLocaleString()} energy | {pool.protected_objects.length} objects
            </p>
          </div>
        </div>

        {/* Mode tabs */}
        <div className="flex bg-zinc-100 rounded-lg p-0.5 mb-5">
          <button
            onClick={() => setMode("stake")}
            className={`flex-1 px-3 py-1.5 rounded-md text-xs font-medium transition ${
              mode === "stake"
                ? "bg-white text-zinc-900 shadow-sm"
                : "text-zinc-500 hover:text-zinc-700"
            }`}
          >
            Stake
          </button>
          <button
            onClick={() => setMode("unstake")}
            className={`flex-1 px-3 py-1.5 rounded-md text-xs font-medium transition ${
              mode === "unstake"
                ? "bg-white text-zinc-900 shadow-sm"
                : "text-zinc-500 hover:text-zinc-700"
            }`}
          >
            Unstake
          </button>
        </div>

        {/* Amount input */}
        <div className="mb-3">
          <label className="block text-xs font-medium text-zinc-700 mb-1">
            Energy Amount
          </label>
          <input
            className="input text-lg font-mono"
            type="number"
            min={1}
            placeholder="0"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
          />
        </div>

        {/* Quick amount buttons */}
        <div className="flex gap-2 mb-5">
          {QUICK_AMOUNTS.map((qa) => (
            <button
              key={qa}
              onClick={() => setAmount(String(qa))}
              className="flex-1 px-2 py-1.5 rounded-lg border border-evap-border text-[10px] font-medium text-zinc-600 hover:bg-zinc-50 hover:border-zinc-300 transition"
            >
              {qa.toLocaleString()}
            </button>
          ))}
        </div>

        {error && <p className="text-xs text-evap-red mb-4">{error}</p>}

        <div className="flex gap-3">
          <button
            onClick={onClose}
            className="flex-1 px-4 py-2.5 rounded-lg border border-evap-border text-xs font-medium text-zinc-600 hover:bg-zinc-50 transition"
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            disabled={submitting}
            className={`flex-1 px-4 py-2.5 rounded-lg text-xs font-semibold text-white hover:opacity-90 transition disabled:opacity-50 ${
              mode === "stake"
                ? "bg-gradient-to-r from-evap-cyan to-evap-green"
                : "bg-gradient-to-r from-evap-purple to-evap-ember"
            }`}
          >
            {submitting
              ? mode === "stake"
                ? "Staking..."
                : "Unstaking..."
              : mode === "stake"
                ? "Stake Energy"
                : "Unstake Energy"}
          </button>
        </div>
      </div>
    </div>
  );
}

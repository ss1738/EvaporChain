import { useState } from "react";
import { boostMessage } from "@/utils/api";

interface Props {
  messageId: string;
  currentEnergy: number;
  onClose: () => void;
  onBoosted: () => void;
}

export default function BoostModal({ messageId, currentEnergy, onClose, onBoosted }: Props) {
  const [energy, setEnergy] = useState(10);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleBoost = async () => {
    setSubmitting(true);
    setError(null);
    try {
      await boostMessage({ message_id: messageId, energy });
      onBoosted();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Boost failed");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
      <div className="w-full max-w-sm rounded-2xl border border-evap-border bg-evap-surface p-6 shadow-xl">
        <h3 className="text-lg font-semibold text-zinc-800">Boost Message Energy</h3>
        <p className="mt-1 text-sm text-zinc-500">
          Add energy to extend the life of this message.
        </p>

        <div className="mt-4 rounded-lg bg-zinc-50 p-3 text-sm text-zinc-600">
          Current energy: <span className="font-mono font-medium">{currentEnergy.toFixed(1)} EVP</span>
        </div>

        <div className="mt-4">
          <label className="block text-sm font-medium text-zinc-700">Energy to add (EVP)</label>
          <input
            type="number"
            min={1}
            value={energy}
            onChange={(e) => setEnergy(Math.max(1, Number(e.target.value)))}
            className="mt-1 w-full rounded-lg border border-evap-border bg-evap-surface px-3 py-2 text-sm focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
          />
        </div>

        {error && (
          <p className="mt-3 text-sm text-evap-red">{error}</p>
        )}

        <div className="mt-6 flex gap-3">
          <button
            onClick={onClose}
            className="flex-1 rounded-lg border border-evap-border px-4 py-2 text-sm font-medium text-zinc-600 hover:bg-zinc-50"
          >
            Cancel
          </button>
          <button
            onClick={handleBoost}
            disabled={submitting}
            className="flex-1 rounded-lg bg-evap-cyan px-4 py-2 text-sm font-medium text-white hover:bg-evap-cyan/90 disabled:opacity-50"
          >
            {submitting ? "Boosting..." : `Boost +${energy} EVP`}
          </button>
        </div>
      </div>
    </div>
  );
}

import { useState } from "react";
import { delegate } from "@/utils/api";

interface Props {
  fromAddress: string;
  onClose: () => void;
  onDelegated: () => void;
}

export function DelegateModal({ fromAddress, onClose, onDelegated }: Props) {
  const [toAddress, setToAddress] = useState("");
  const [weight, setWeight] = useState("100");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async () => {
    const addr = toAddress.trim();
    if (!addr) {
      setError("Enter a delegate address");
      return;
    }

    setSubmitting(true);
    setError(null);
    try {
      const result = await delegate(fromAddress, addr, parseInt(weight) || 100);
      if (result.success) {
        onDelegated();
        onClose();
      } else {
        setError(result.message ?? "Delegation failed");
      }
    } catch {
      setError("Failed to delegate");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center px-4">
      <div className="fixed inset-0 bg-black/30" onClick={onClose} />
      <div className="relative bg-white rounded-2xl border border-evap-border w-full max-w-md p-6 shadow-xl">
        <h2 className="text-lg font-bold text-zinc-900 mb-1">Delegate Voting Power</h2>
        <p className="text-xs text-zinc-400 mb-5">
          Allow another address to vote on your behalf.
        </p>

        <div className="mb-3">
          <label className="text-[10px] text-zinc-400 uppercase tracking-wider block mb-1">
            Delegate To
          </label>
          <input
            type="text"
            value={toAddress}
            onChange={(e) => setToAddress(e.target.value)}
            placeholder="0x..."
            className="w-full px-3 py-2 rounded-lg border border-evap-border text-sm text-zinc-900 font-mono focus:outline-none focus:border-evap-purple transition-colors"
          />
        </div>

        <div className="mb-4">
          <label className="text-[10px] text-zinc-400 uppercase tracking-wider block mb-1">
            Voting Weight (%)
          </label>
          <input
            type="number"
            value={weight}
            onChange={(e) => setWeight(e.target.value)}
            min="1"
            max="100"
            className="w-full px-3 py-2 rounded-lg border border-evap-border text-sm text-zinc-900 focus:outline-none focus:border-evap-purple transition-colors"
          />
          <p className="text-[10px] text-zinc-400 mt-1">
            Percentage of your voting power to delegate. You keep the rest.
          </p>
        </div>

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
            className="flex-1 py-2.5 rounded-xl bg-evap-purple text-white text-sm font-medium hover:bg-evap-purple/90 transition-colors disabled:opacity-50"
          >
            {submitting ? "Delegating..." : "Delegate"}
          </button>
        </div>
      </div>
    </div>
  );
}

"use client";
import { useState } from "react";
import { singhApi, type SinghPoolPosition } from "@/lib/api";

interface Props {
  position: SinghPoolPosition;
  currentEpoch: number;
  onClose: () => void;
  onPledged?: () => void;
}

// Adds a Patronage Covenant to a position guaranteeing N epochs of immunity
// from auto-eviction. Wires POST /api/patronage/pledge through the SDK.
export function PatronageDialog({ position, currentEpoch, onClose, onPledged }: Props) {
  const [donationPerEpoch, setDonationPerEpoch] = useState(5);
  const [epochs, setEpochs] = useState(120);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const total = donationPerEpoch * epochs;

  const handlePledge = async () => {
    setSubmitting(true);
    setError(null);
    try {
      const res = await singhApi.openCovenant(position.id, donationPerEpoch, epochs, currentEpoch);
      if (res.status !== "pledged") {
        throw new Error(res.detail || "pledge failed");
      }
      onPledged?.();
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Pledge failed");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
      <div className="w-full max-w-sm rounded-2xl border border-evap-border bg-white p-6 shadow-xl">
        <h3 className="text-lg font-semibold text-zinc-900">Add Patronage Covenant</h3>
        <p className="mt-1 text-xs text-zinc-500">
          Pre-fund this position so it&rsquo;s immune to auto-eviction for the
          duration of the covenant. Refundable on revoke.
        </p>

        <div className="mt-4 grid grid-cols-2 gap-3">
          <div>
            <label className="block text-xs font-medium text-zinc-700">EVAP/epoch</label>
            <input
              type="number"
              min={1}
              value={donationPerEpoch}
              onChange={(e) => setDonationPerEpoch(Math.max(1, Number(e.target.value)))}
              className="mt-1 w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-sm focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-zinc-700">Epochs</label>
            <input
              type="number"
              min={1}
              value={epochs}
              onChange={(e) => setEpochs(Math.max(1, Number(e.target.value)))}
              className="mt-1 w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-sm focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
            />
          </div>
        </div>

        <div className="mt-4 rounded-lg bg-zinc-50 p-3 text-xs text-zinc-600">
          Pre-funded total:{" "}
          <span className="font-mono font-semibold text-zinc-900">{total} EVAP</span>
          <p className="mt-1 text-[10px] text-zinc-400">
            Position {position.id.slice(0, 10)}… expires at epoch {currentEpoch + epochs}.
          </p>
        </div>

        {error && <p className="mt-3 text-sm text-evap-red">{error}</p>}

        <div className="mt-6 flex gap-3">
          <button
            onClick={onClose}
            className="flex-1 rounded-lg border border-evap-border px-4 py-2 text-sm font-medium text-zinc-600 hover:bg-zinc-50"
          >
            Cancel
          </button>
          <button
            onClick={handlePledge}
            disabled={submitting}
            className="flex-1 rounded-lg bg-evap-purple px-4 py-2 text-sm font-medium text-white hover:bg-evap-purple/90 disabled:opacity-50"
          >
            {submitting ? "Pledging..." : "Open Covenant"}
          </button>
        </div>
      </div>
    </div>
  );
}

"use client";
import { useState } from "react";
import { singhApi, type SinghPoolPosition } from "@/lib/api";

interface Props {
  position: SinghPoolPosition;
  onRefreshed?: () => void;
}

// Tops up a position's energy reserve by N EVAP. Wires POST /api/tx/refresh.
// Active positions stop earning fees once they decay; refreshing pulls them
// back into the active liquidity range.
export function RefreshButton({ position, onRefreshed }: Props) {
  const [open, setOpen] = useState(false);
  const [energy, setEnergy] = useState(Math.max(10, Math.floor(position.maxEnergy / 4)));
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleRefresh = async () => {
    setSubmitting(true);
    setError(null);
    try {
      const res = await singhApi.refreshPosition(position.id, energy);
      if (!res.success) {
        throw new Error(res.message || "refresh failed");
      }
      setOpen(false);
      onRefreshed?.();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Refresh failed");
    } finally {
      setSubmitting(false);
    }
  };

  if (!open) {
    return (
      <button
        onClick={() => setOpen(true)}
        className="rounded-lg border border-evap-cyan/30 bg-evap-cyan/5 px-3 py-1.5 text-xs font-semibold text-evap-cyan hover:bg-evap-cyan/10"
      >
        ⟳ Refresh Energy
      </button>
    );
  }

  return (
    <div className="rounded-lg border border-evap-cyan/40 bg-cyan-50 p-3">
      <p className="text-[11px] font-semibold text-evap-cyan">
        Top up energy reserve
      </p>
      <div className="mt-2 flex items-center gap-2">
        <input
          type="number"
          min={1}
          value={energy}
          onChange={(e) => setEnergy(Math.max(1, Number(e.target.value)))}
          className="flex-1 rounded-md border border-evap-cyan/30 bg-white px-2 py-1 text-sm focus:outline-none focus:ring-1 focus:ring-evap-cyan"
        />
        <span className="text-[10px] text-evap-cyan">EVAP</span>
      </div>
      {error && <p className="mt-2 text-xs text-evap-red">{error}</p>}
      <div className="mt-2 flex gap-2">
        <button
          onClick={() => setOpen(false)}
          className="flex-1 rounded-md border border-evap-border bg-white px-2 py-1 text-xs text-zinc-600"
        >
          Cancel
        </button>
        <button
          onClick={handleRefresh}
          disabled={submitting}
          className="flex-1 rounded-md bg-evap-cyan px-2 py-1 text-xs font-semibold text-white hover:bg-evap-cyan/90 disabled:opacity-50"
        >
          {submitting ? "Refreshing..." : "Refresh"}
        </button>
      </div>
    </div>
  );
}

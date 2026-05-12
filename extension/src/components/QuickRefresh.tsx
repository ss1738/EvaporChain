import { useState, useMemo } from "react";
import { Check } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import { energyPercent, formatBalance } from "@/utils/format";

const URGENT_THRESHOLD = 20;

export function QuickRefresh() {
  const { objects, batchRefreshObjects, loading, balance } = useWallet();
  const [expanded, setExpanded] = useState(false);
  const [confirmed, setConfirmed] = useState(false);

  const urgentObjects = useMemo(() => {
    return objects.filter(obj => {
      if (obj.state === "Ghost") return false;
      return energyPercent(obj.current_energy, obj.max_energy) < URGENT_THRESHOLD;
    });
  }, [objects]);

  const totalCost = useMemo(() => {
    return urgentObjects.reduce((sum, obj) => sum + (obj.max_energy - obj.current_energy), 0);
  }, [urgentObjects]);

  const averageEpochsExtended = useMemo(() => {
    if (urgentObjects.length === 0) return 0;
    const totalHalfLife = urgentObjects.reduce((sum, obj) => sum + obj.half_life, 0);
    return Math.round(totalHalfLife / urgentObjects.length);
  }, [urgentObjects]);

  if (urgentObjects.length === 0) return null;

  const canAfford = balance >= totalCost;

  const handleQuickRefresh = async () => {
    if (!canAfford || loading) return;

    const items = urgentObjects.map(obj => ({
      id: obj.id,
      energy: obj.max_energy - obj.current_energy,
    }));

    await batchRefreshObjects(items);
    setConfirmed(true);
    setExpanded(false);

    // Reset confirmation after a moment
    setTimeout(() => setConfirmed(false), 3000);
  };

  return (
    <div className="fixed bottom-16 right-3 z-50">
      {/* Expanded panel */}
      {expanded && (
        <div className="mb-2 w-56 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border shadow-xl">
          <p className="text-xs font-semibold text-zinc-200 mb-1">
            Quick Refresh
          </p>
          <p className="text-xs text-zinc-400 mb-2">
            {urgentObjects.length} object{urgentObjects.length > 1 ? "s" : ""} below {URGENT_THRESHOLD}% energy
          </p>

          <div className="space-y-1 mb-2">
            <div className="flex justify-between text-[10px]">
              <span className="text-zinc-500">Energy cost</span>
              <span className="text-evap-cyan font-medium">{formatBalance(totalCost)} EVAP</span>
            </div>
            <div className="flex justify-between text-[10px]">
              <span className="text-zinc-500">Extends life by</span>
              <span className="text-evap-green font-medium">~{averageEpochsExtended} epochs</span>
            </div>
          </div>

          <button
            onClick={handleQuickRefresh}
            disabled={loading || !canAfford}
            className={`w-full py-2 rounded-lg text-xs font-semibold transition ${
              canAfford
                ? "bg-evap-cyan text-black hover:bg-evap-cyan/90"
                : "bg-zinc-700 text-zinc-400 cursor-not-allowed"
            } disabled:opacity-50`}
          >
            {!canAfford
              ? "Insufficient balance"
              : loading
              ? "Refreshing..."
              : `Refresh All (${formatBalance(totalCost)} EVAP)`}
          </button>

          <button
            onClick={() => setExpanded(false)}
            className="mt-1.5 w-full text-[10px] text-zinc-500 hover:text-zinc-300"
          >
            Dismiss
          </button>
        </div>
      )}

      {/* FAB button */}
      <button
        onClick={() => setExpanded(!expanded)}
        className={`w-12 h-12 rounded-full shadow-lg flex items-center justify-center transition-all ${
          confirmed
            ? "bg-evap-green"
            : "bg-evap-cyan hover:bg-evap-cyan/90"
        }`}
      >
        {confirmed ? (
          <span className="text-black text-lg"><Check className="w-3.5 h-3.5" strokeWidth={1.5} /></span>
        ) : (
          <div className="relative">
            <span className="text-black text-lg">🔄</span>
            <span className="absolute -top-2 -right-2 w-4 h-4 rounded-full bg-red-500 text-[8px] font-bold text-white flex items-center justify-center">
              {urgentObjects.length}
            </span>
          </div>
        )}
      </button>
    </div>
  );
}

import { useState, useMemo, useEffect, useCallback } from "react";
import { CheckCircle } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import { EnergyBar } from "./EnergyBar";
import { energyPercent, formatBalance } from "@/utils/format";
import { Header } from "./Header";
import { api } from "@/utils/api";

type ThresholdPreset = 10 | 20 | 30 | 50;
type AutoFrequency = "every-epoch" | "every-10" | "every-100";

interface RefreshEstimate {
  objectId: string;
  energyNeeded: number;
  evapCost: number;
}

export function BatchRefresh() {
  const {
    objects, refreshObjects, batchRefreshObjects,
    setView, loading, balance,
  } = useWallet();

  const [threshold, setThreshold] = useState<number>(20);
  const [excluded, setExcluded] = useState<Set<string>>(new Set());
  const [estimates, setEstimates] = useState<Map<string, RefreshEstimate>>(new Map());
  const [progress, setProgress] = useState<{ current: number; total: number } | null>(null);
  const [result, setResult] = useState<{ count: number; cost: number } | null>(null);

  // Auto-refresh scheduler state
  const [autoEnabled, setAutoEnabled] = useState(false);
  const [autoThreshold, setAutoThreshold] = useState(30);
  const [autoFrequency, setAutoFrequency] = useState<AutoFrequency>("every-10");

  useEffect(() => {
    refreshObjects();
  }, [refreshObjects]);

  // Filter objects below threshold
  const eligibleObjects = useMemo(() => {
    return objects
      .filter(obj => obj.state !== "Ghost")
      .filter(obj => {
        const pct = energyPercent(obj.current_energy, obj.max_energy);
        return pct < threshold;
      })
      .sort((a, b) => a.current_energy - b.current_energy);
  }, [objects, threshold]);

  // Selected objects (not excluded)
  const selectedObjects = useMemo(() => {
    return eligibleObjects.filter(obj => !excluded.has(obj.id));
  }, [eligibleObjects, excluded]);

  // Fetch cost estimates when selected objects change
  const fetchEstimates = useCallback(async () => {
    const newEstimates = new Map<string, RefreshEstimate>();
    for (const obj of eligibleObjects) {
      try {
        const cost = await api.getRefreshCost(obj.id, obj.max_energy);
        newEstimates.set(obj.id, {
          objectId: obj.id,
          energyNeeded: cost.energy_needed,
          evapCost: cost.evap_cost,
        });
      } catch {
        // Fallback: estimate as energy difference
        const needed = obj.max_energy - obj.current_energy;
        newEstimates.set(obj.id, {
          objectId: obj.id,
          energyNeeded: needed,
          evapCost: needed,
        });
      }
    }
    setEstimates(newEstimates);
  }, [eligibleObjects]);

  useEffect(() => {
    if (eligibleObjects.length > 0) {
      fetchEstimates();
    }
  }, [eligibleObjects, fetchEstimates]);

  const totalCost = useMemo(() => {
    return selectedObjects.reduce((sum, obj) => {
      const est = estimates.get(obj.id);
      return sum + (est?.evapCost ?? (obj.max_energy - obj.current_energy));
    }, 0);
  }, [selectedObjects, estimates]);

  const toggleExclude = (id: string) => {
    setExcluded(prev => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const handleBatchRefresh = async () => {
    if (selectedObjects.length === 0) return;

    const refreshItems = selectedObjects.map(obj => {
      const est = estimates.get(obj.id);
      return {
        id: obj.id,
        energy: est?.energyNeeded ?? (obj.max_energy - obj.current_energy),
      };
    });

    setProgress({ current: 0, total: refreshItems.length });

    try {
      await batchRefreshObjects(refreshItems);
      setResult({ count: refreshItems.length, cost: totalCost });
      setProgress(null);
    } catch {
      setProgress(null);
    }
  };

  const canAfford = balance >= totalCost;

  return (
    <div className="flex flex-col h-full">
      <Header />

      {/* Sub-header */}
      <div className="px-4 pt-4 pb-2 flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold text-zinc-100">Batch Refresh</h2>
          <p className="text-xs text-zinc-500">Keep your objects alive</p>
        </div>
        <button
          onClick={() => setView("home")}
          className="text-xs text-zinc-500 hover:text-zinc-300"
        >
          &larr; Back
        </button>
      </div>

      {/* Success result */}
      {result && (
        <div className="mx-4 mb-3 px-3 py-3 rounded-lg bg-evap-green/10 border border-evap-green/30">
          <p className="text-xs text-evap-green text-center font-medium">
            Refreshed {result.count} objects, spent {formatBalance(result.cost)} EVAP
          </p>
          <button
            onClick={() => setResult(null)}
            className="mt-2 w-full text-xs text-zinc-400 hover:text-zinc-200"
          >
            Dismiss
          </button>
        </div>
      )}

      {/* Progress indicator */}
      {progress && (
        <div className="mx-4 mb-3 px-3 py-3 rounded-lg bg-evap-cyan/10 border border-evap-cyan/30">
          <p className="text-xs text-evap-cyan text-center">
            Refreshing objects...
          </p>
          <div className="mt-2 w-full bg-evap-border rounded-full h-2 overflow-hidden">
            <div
              className="h-2 rounded-full bg-evap-cyan transition-all duration-500"
              style={{ width: "100%" }}
            />
          </div>
        </div>
      )}

      {/* Threshold selector */}
      <div className="px-4 pb-3">
        <p className="text-xs text-zinc-400 mb-2">
          Refresh all objects below <span className="text-evap-cyan font-semibold">{threshold}%</span> energy
        </p>
        <div className="flex gap-1.5">
          {([10, 20, 30, 50] as ThresholdPreset[]).map(pct => (
            <button
              key={pct}
              onClick={() => setThreshold(pct)}
              className={`flex-1 py-1.5 text-xs rounded-lg border transition ${
                threshold === pct
                  ? "bg-evap-cyan/10 border-evap-cyan/40 text-evap-cyan"
                  : "border-evap-border text-zinc-500 hover:text-zinc-300"
              }`}
            >
              {pct}%
            </button>
          ))}
        </div>

        {/* Custom slider */}
        <div className="mt-2 flex items-center gap-2">
          <input
            type="range"
            min={5}
            max={80}
            value={threshold}
            onChange={e => setThreshold(Number(e.target.value))}
            className="flex-1 h-1 accent-evap-cyan"
          />
          <span className="text-xs text-zinc-500 w-8 text-right">{threshold}%</span>
        </div>
      </div>

      {/* Preview list */}
      <div className="flex-1 overflow-y-auto px-4 pb-2 space-y-1.5">
        {eligibleObjects.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-10">
            <span className="text-2xl mb-2"><CheckCircle className="w-3.5 h-3.5" strokeWidth={1.5} /></span>
            <p className="text-sm text-zinc-500">All objects above {threshold}%</p>
            <p className="text-xs text-zinc-600 mt-1">Nothing to refresh</p>
          </div>
        ) : (
          eligibleObjects.map(obj => {
            const pct = energyPercent(obj.current_energy, obj.max_energy);
            const est = estimates.get(obj.id);
            const isExcluded = excluded.has(obj.id);

            return (
              <div
                key={obj.id}
                className={`px-3 py-2.5 rounded-lg border transition ${
                  isExcluded
                    ? "bg-zinc-900/50 border-zinc-800 opacity-50"
                    : "bg-evap-surface border-evap-border"
                }`}
              >
                <div className="flex items-center gap-2">
                  {/* Checkbox */}
                  <button
                    onClick={() => toggleExclude(obj.id)}
                    className={`w-4 h-4 rounded border flex items-center justify-center text-[9px] transition ${
                      isExcluded
                        ? "border-zinc-700 text-zinc-700"
                        : "border-evap-cyan bg-evap-cyan/10 text-evap-cyan"
                    }`}
                  >
                    {!isExcluded && "✓"}
                  </button>

                  {/* Object info */}
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center justify-between mb-1">
                      <p className="text-xs font-semibold text-zinc-200 truncate">
                        {obj.name || "Object"}
                      </p>
                      <span className="text-xs text-zinc-500">{pct}%</span>
                    </div>
                    <EnergyBar current={obj.current_energy} max={obj.max_energy} showLabel={false} size="sm" />
                  </div>
                </div>

                {/* Cost info */}
                {est && !isExcluded && (
                  <div className="flex items-center justify-between mt-1.5 pl-6">
                    <span className="text-[9px] text-zinc-500">
                      Needs +{formatBalance(est.energyNeeded)} energy
                    </span>
                    <span className="text-[9px] text-evap-cyan font-medium">
                      {formatBalance(est.evapCost)} EVAP
                    </span>
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>

      {/* Summary footer */}
      {selectedObjects.length > 0 && (
        <div className="px-4 py-3 border-t border-evap-border bg-evap-surface/50">
          <div className="flex items-center justify-between mb-2">
            <span className="text-xs text-zinc-400">
              Total objects: <span className="text-zinc-200 font-medium">{selectedObjects.length}</span>
            </span>
            <span className="text-xs text-zinc-400">
              Total cost: <span className="text-evap-cyan font-medium">{formatBalance(totalCost)} EVAP</span>
            </span>
          </div>
          <button
            onClick={handleBatchRefresh}
            disabled={loading || !canAfford || selectedObjects.length === 0}
            className={`w-full py-2.5 rounded-lg text-xs font-semibold transition ${
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
        </div>
      )}

      {/* Auto-refresh scheduler */}
      <div className="px-4 py-3 border-t border-evap-border">
        <div className="flex items-center justify-between mb-2">
          <p className="text-xs font-semibold text-zinc-300">Auto-Refresh Scheduler</p>
          <button
            onClick={() => setAutoEnabled(!autoEnabled)}
            className={`w-9 h-5 rounded-full transition relative ${
              autoEnabled ? "bg-evap-cyan" : "bg-zinc-700"
            }`}
          >
            <div
              className={`w-3.5 h-3.5 rounded-full bg-white absolute top-0.5 transition-all ${
                autoEnabled ? "left-[18px]" : "left-[3px]"
              }`}
            />
          </button>
        </div>

        {autoEnabled && (
          <div className="space-y-2">
            <div>
              <p className="text-xs text-zinc-500 mb-1">
                Keep objects above <span className="text-evap-cyan">{autoThreshold}%</span> automatically
              </p>
              <input
                type="range"
                min={10}
                max={50}
                value={autoThreshold}
                onChange={e => setAutoThreshold(Number(e.target.value))}
                className="w-full h-1 accent-evap-cyan"
              />
            </div>

            <div>
              <p className="text-xs text-zinc-500 mb-1">Frequency</p>
              <div className="flex gap-1.5">
                {([
                  { key: "every-epoch" as AutoFrequency, label: "Every epoch" },
                  { key: "every-10" as AutoFrequency, label: "Every 10" },
                  { key: "every-100" as AutoFrequency, label: "Every 100" },
                ]).map(opt => (
                  <button
                    key={opt.key}
                    onClick={() => setAutoFrequency(opt.key)}
                    className={`flex-1 py-1.5 text-[9px] rounded-lg border transition ${
                      autoFrequency === opt.key
                        ? "bg-evap-cyan/10 border-evap-cyan/40 text-evap-cyan"
                        : "border-evap-border text-zinc-500 hover:text-zinc-300"
                    }`}
                  >
                    {opt.label}
                  </button>
                ))}
              </div>
            </div>

            <p className="text-[9px] text-zinc-600 italic">
              Auto-refresh will execute when wallet is unlocked
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

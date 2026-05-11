import { useEffect, useMemo, useState } from "react";
import { ArrowLeft } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import type { StateObject } from "@/utils/api";
import {
  daysUntilEvaporation,
  estimateEvaporationDate,
  optimalRefreshStrategy,
  projectedPortfolioEnergy,
  projectedObjectEnergy,
  totalBudgetForSurvival,
  epochsUntilThreshold,
} from "@/utils/decay";

/** Default epoch duration: 30 seconds (testnet). */
const DEFAULT_EPOCH_MS = 30_000;

/** Grace threshold: objects enter grace state below this energy %. */
const GRACE_THRESHOLD = 25;

/** Ghost threshold: objects become ghosts below this energy %. */
const GHOST_THRESHOLD = 5;

function getEpochDurationMs(chainStatus: { epoch?: number } | null): number {
  // In a real implementation this would come from chain config.
  return DEFAULT_EPOCH_MS;
}

function urgencyColor(daysLeft: number): string {
  if (daysLeft <= 1) return "bg-red-500/10 border-red-500/30";
  if (daysLeft <= 3) return "bg-amber-500/10 border-amber-500/30";
  if (daysLeft <= 7) return "bg-yellow-400/10 border-yellow-400/30";
  return "bg-emerald-500/10 border-emerald-500/30";
}

function urgencyTextColor(daysLeft: number): string {
  if (daysLeft <= 1) return "text-red-400";
  if (daysLeft <= 3) return "text-amber-400";
  if (daysLeft <= 7) return "text-yellow-300";
  return "text-emerald-400";
}

function energyBarColor(percent: number): string {
  if (percent <= GHOST_THRESHOLD) return "bg-zinc-500";
  if (percent <= GRACE_THRESHOLD) return "bg-red-500";
  if (percent <= 50) return "bg-amber-500";
  return "bg-emerald-500";
}

export function DecayForecasting() {
  const {
    objects,
    chainStatus,
    setView,
    refreshObjects,
    loading,
    setNotification,
    setError,
  } = useWallet();

  const [selectedObject, setSelectedObject] = useState<StateObject | null>(null);
  const [strategyDays, setStrategyDays] = useState<7 | 30 | 90>(7);
  const [executingStrategy, setExecutingStrategy] = useState(false);

  useEffect(() => {
    refreshObjects();
  }, [refreshObjects]);

  const epochMs = getEpochDurationMs(chainStatus);

  // Epochs per day
  const epochsPerDay = (24 * 60 * 60 * 1000) / epochMs;

  // ── Portfolio Projection (next 7 days) ──
  const portfolioProjection = useMemo(() => {
    if (objects.length === 0) return [];
    const futureEpochs = Math.ceil(epochsPerDay * 7);
    const step = Math.max(1, Math.floor(futureEpochs / 7));
    return projectedPortfolioEnergy(objects, futureEpochs, step);
  }, [objects, epochsPerDay]);

  // Current vs next-week energy
  const currentTotalEnergy = objects.reduce((s, o) => s + o.current_energy, 0);
  const maxTotalEnergy = objects.reduce((s, o) => s + o.max_energy, 0);
  const weekEndEnergy = portfolioProjection.length > 0
    ? portfolioProjection[portfolioProjection.length - 1].totalEnergy
    : currentTotalEnergy;
  const weekLossPercent = currentTotalEnergy > 0
    ? ((currentTotalEnergy - weekEndEnergy) / currentTotalEnergy) * 100
    : 0;
  const isGaining = weekEndEnergy >= currentTotalEnergy;

  // ── Per-object forecasts sorted by urgency ──
  const objectForecasts = useMemo(() => {
    return objects
      .map((obj) => {
        const days = daysUntilEvaporation(obj.current_energy, obj.half_life, epochMs);
        const evapDate = estimateEvaporationDate(obj.current_energy, obj.half_life, epochMs);
        const energyPercent = obj.max_energy > 0
          ? (obj.current_energy / obj.max_energy) * 100
          : 0;
        return { obj, days, evapDate, energyPercent };
      })
      .sort((a, b) => a.days - b.days);
  }, [objects, epochMs]);

  // ── Refresh Strategy ──
  const strategy = useMemo(() => {
    if (objects.length === 0) return [];
    return optimalRefreshStrategy(objects, Infinity, strategyDays, epochMs);
  }, [objects, strategyDays, epochMs]);

  const budgets = useMemo(() => ({
    week: totalBudgetForSurvival(objects, 7, epochMs),
    month: totalBudgetForSurvival(objects, 30, epochMs),
    quarter: totalBudgetForSurvival(objects, 90, epochMs),
  }), [objects, epochMs]);

  // ── Per-Object Decay Curve ──
  const objectCurve = useMemo(() => {
    if (!selectedObject) return [];
    const futureEpochs = Math.ceil(epochsPerDay * 14); // 14 days
    const step = Math.max(1, Math.floor(futureEpochs / 14));
    return projectedObjectEnergy(
      selectedObject.current_energy,
      selectedObject.half_life,
      futureEpochs,
      step,
    );
  }, [selectedObject, epochsPerDay]);

  const handleExecuteStrategy = async () => {
    const itemsToRefresh = strategy.filter((r) => r.energyToAdd > 0);
    if (itemsToRefresh.length === 0) {
      setNotification("All objects already survive the target period");
      return;
    }
    setExecutingStrategy(true);
    try {
      const { api } = await import("@/utils/api");
      await api.batchRefresh(
        itemsToRefresh.map((r) => ({ id: r.objectId, energy: r.energyToAdd })),
      );
      setNotification(`Refreshed ${itemsToRefresh.length} objects`);
      refreshObjects();
    } catch (e: any) {
      setError(e.message);
    } finally {
      setExecutingStrategy(false);
    }
  };

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-evap-border">
        <button
          onClick={() => setView("home")}
          className="text-zinc-400 hover:text-zinc-200 transition text-sm"
        >
          <><ArrowLeft className="inline w-3.5 h-3.5 mr-1 -mt-0.5" strokeWidth={1.5} />Back</>
        </button>
        <h1 className="text-sm font-semibold text-zinc-100">Decay Forecast</h1>
      </div>

      <div className="flex-1 overflow-y-auto px-4 py-3 space-y-4">
        {objects.length === 0 ? (
          <div className="text-center py-12">
            <p className="text-zinc-500 text-sm">No objects to forecast</p>
            <p className="text-zinc-600 text-xs mt-1">Create objects to see decay predictions</p>
          </div>
        ) : (
          <>
            {/* ── Portfolio Summary ── */}
            <div className="rounded-lg bg-evap-surface border border-evap-border p-3">
              <div className="flex items-center justify-between mb-2">
                <p className="text-xs font-medium text-zinc-300">Portfolio Outlook</p>
                <div className="flex items-center gap-1">
                  <span className={`text-xs font-semibold ${isGaining ? "text-emerald-400" : "text-red-400"}`}>
                    {isGaining ? "↑" : "↓"} {weekLossPercent.toFixed(1)}%
                  </span>
                  <span className="text-xs text-zinc-500">this week</span>
                </div>
              </div>

              <p className="text-xs text-zinc-500 mb-2">
                At current rate, your portfolio loses{" "}
                <span className="text-amber-400 font-medium">{weekLossPercent.toFixed(1)}%</span>{" "}
                energy this week
              </p>

              {/* 7-day bar chart */}
              <div className="flex items-end gap-1 h-16 mb-1">
                {portfolioProjection.map((point, i) => {
                  const heightPercent = maxTotalEnergy > 0
                    ? (point.totalEnergy / maxTotalEnergy) * 100
                    : 0;
                  return (
                    <div key={i} className="flex-1 flex flex-col items-center justify-end h-full">
                      <div
                        className={`w-full rounded-t ${energyBarColor(heightPercent)} transition-all`}
                        style={{ height: `${Math.max(2, heightPercent)}%` }}
                      />
                    </div>
                  );
                })}
              </div>
              <div className="flex justify-between">
                <span className="text-[10px] text-zinc-600">Today</span>
                <span className="text-[10px] text-zinc-600">+7 days</span>
              </div>

              {/* Energy summary */}
              <div className="grid grid-cols-2 gap-2 mt-2">
                <div className="text-center">
                  <p className="text-xs font-semibold text-zinc-200">{Math.round(currentTotalEnergy)}</p>
                  <p className="text-xs text-zinc-500">Current Energy</p>
                </div>
                <div className="text-center">
                  <p className="text-xs font-semibold text-zinc-200">{Math.round(weekEndEnergy)}</p>
                  <p className="text-xs text-zinc-500">Projected (7d)</p>
                </div>
              </div>
            </div>

            {/* ── Per-Object Forecast Table ── */}
            <div>
              <p className="text-xs font-medium text-zinc-300 mb-2">Object Forecasts</p>
              <div className="space-y-1.5">
                {objectForecasts.map(({ obj, days, evapDate, energyPercent }) => (
                  <button
                    key={obj.id}
                    onClick={() => setSelectedObject(selectedObject?.id === obj.id ? null : obj)}
                    className={`w-full text-left rounded-lg border p-2.5 transition ${urgencyColor(days)} ${
                      selectedObject?.id === obj.id ? "ring-1 ring-evap-cyan/50" : ""
                    }`}
                  >
                    <div className="flex items-center justify-between mb-1">
                      <span className="text-xs font-medium text-zinc-200 truncate max-w-[140px]">
                        {obj.name}
                      </span>
                      <span className={`text-xs font-semibold ${urgencyTextColor(days)}`}>
                        {days < 1 ? "<1 day" : `${days.toFixed(1)}d`} left
                      </span>
                    </div>
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-2">
                        {/* Energy bar */}
                        <div className="w-16 h-1.5 rounded-full bg-zinc-700 overflow-hidden">
                          <div
                            className={`h-full rounded-full ${energyBarColor(energyPercent)}`}
                            style={{ width: `${Math.min(100, energyPercent)}%` }}
                          />
                        </div>
                        <span className="text-xs text-zinc-400">
                          {energyPercent.toFixed(0)}%
                        </span>
                      </div>
                      <span className="text-[10px] text-zinc-500">
                        Evaporates {evapDate.toLocaleDateString()} {evapDate.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                      </span>
                    </div>

                    {/* Expanded decay curve */}
                    {selectedObject?.id === obj.id && objectCurve.length > 0 && (
                      <div className="mt-3 pt-2 border-t border-white/5">
                        <p className="text-xs text-zinc-400 mb-1">Decay Curve (14 days)</p>
                        <div className="relative">
                          {/* Threshold lines */}
                          <div
                            className="absolute left-0 right-0 border-t border-dashed border-amber-500/30"
                            style={{ bottom: `${GRACE_THRESHOLD}%` }}
                          >
                            <span className="text-[8px] text-amber-500/60 absolute -top-3 right-0">Grace {GRACE_THRESHOLD}%</span>
                          </div>
                          <div
                            className="absolute left-0 right-0 border-t border-dashed border-red-500/30"
                            style={{ bottom: `${GHOST_THRESHOLD}%` }}
                          >
                            <span className="text-[8px] text-red-500/60 absolute -top-3 right-0">Ghost {GHOST_THRESHOLD}%</span>
                          </div>

                          <div className="flex items-end gap-0.5 h-20">
                            {objectCurve.map((point, i) => {
                              const maxE = selectedObject.max_energy || 1;
                              const pct = (point.energy / maxE) * 100;
                              const isNow = i === 0;
                              return (
                                <div key={i} className="flex-1 flex flex-col items-center justify-end h-full relative">
                                  <div
                                    className={`w-full rounded-t transition-all ${
                                      isNow ? "bg-evap-cyan" : energyBarColor(pct)
                                    }`}
                                    style={{ height: `${Math.max(1, pct)}%` }}
                                  />
                                  {isNow && (
                                    <div className="absolute -top-3">
                                      <span className="text-[8px] text-evap-cyan">Now</span>
                                    </div>
                                  )}
                                </div>
                              );
                            })}
                          </div>
                        </div>
                        <div className="flex justify-between mt-0.5">
                          <span className="text-[10px] text-zinc-600">Today</span>
                          <span className="text-[10px] text-zinc-600">+14 days</span>
                        </div>
                      </div>
                    )}
                  </button>
                ))}
              </div>
            </div>

            {/* ── Cheapest Refresh Strategy ── */}
            <div className="rounded-lg bg-evap-surface border border-evap-border p-3">
              <p className="text-xs font-medium text-zinc-300 mb-2">Cheapest Refresh Strategy</p>

              {/* Time horizon selector */}
              <div className="flex gap-1 mb-3">
                {([7, 30, 90] as const).map((d) => (
                  <button
                    key={d}
                    onClick={() => setStrategyDays(d)}
                    className={`flex-1 py-1.5 rounded text-xs font-medium transition ${
                      strategyDays === d
                        ? "bg-evap-cyan/20 text-evap-cyan border border-evap-cyan/30"
                        : "bg-zinc-800 text-zinc-500 border border-transparent hover:border-zinc-600"
                    }`}
                  >
                    {d}d
                  </button>
                ))}
              </div>

              {/* Budget summary */}
              <div className="grid grid-cols-3 gap-2 mb-3">
                <BudgetBox label="7 days" cost={budgets.week} active={strategyDays === 7} />
                <BudgetBox label="30 days" cost={budgets.month} active={strategyDays === 30} />
                <BudgetBox label="90 days" cost={budgets.quarter} active={strategyDays === 90} />
              </div>

              {/* Recommended refresh order */}
              <div className="space-y-1 mb-3">
                {strategy
                  .filter((r) => r.energyToAdd > 0)
                  .map((rec, i) => (
                    <div
                      key={rec.objectId}
                      className="flex items-center justify-between px-2 py-1.5 rounded bg-zinc-800/50"
                    >
                      <div className="flex items-center gap-2">
                        <span className="text-xs text-zinc-500 w-4">{i + 1}.</span>
                        <div>
                          <p className="text-xs text-zinc-200 truncate max-w-[120px]">
                            {rec.objectName}
                          </p>
                          <p className="text-[10px] text-zinc-500">
                            {rec.daysRemaining.toFixed(1)}d left → {rec.daysSavedAfterRefresh.toFixed(1)}d after
                          </p>
                        </div>
                      </div>
                      <span className="text-xs text-evap-cyan font-medium">
                        +{rec.energyToAdd} E
                      </span>
                    </div>
                  ))}

                {strategy.filter((r) => r.energyToAdd > 0).length === 0 && (
                  <p className="text-xs text-zinc-500 text-center py-2">
                    All objects survive the next {strategyDays} days
                  </p>
                )}
              </div>

              {/* Execute button */}
              {strategy.some((r) => r.energyToAdd > 0) && (
                <button
                  onClick={handleExecuteStrategy}
                  disabled={executingStrategy || loading}
                  className="w-full py-2 rounded-lg bg-evap-cyan text-zinc-900 text-xs font-semibold hover:bg-evap-cyan/90 transition disabled:opacity-50"
                >
                  {executingStrategy ? "Refreshing..." : `Execute Strategy (${strategyDays}d)`}
                </button>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function BudgetBox({
  label,
  cost,
  active,
}: {
  label: string;
  cost: number;
  active: boolean;
}) {
  return (
    <div
      className={`text-center py-1.5 rounded border transition ${
        active
          ? "border-evap-cyan/30 bg-evap-cyan/5"
          : "border-transparent bg-zinc-800/50"
      }`}
    >
      <p className={`text-xs font-semibold ${active ? "text-evap-cyan" : "text-zinc-300"}`}>
        {cost > 0 ? cost.toLocaleString() : "0"}
      </p>
      <p className="text-[10px] text-zinc-500">{label}</p>
    </div>
  );
}

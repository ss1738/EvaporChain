import { useState } from "react";
import { api, type SimulationResult } from "@/utils/api";
import { formatBalance, shortAddress, energyPercent, energyColor } from "@/utils/format";
import { DecayForecast, type DecayForecastData } from "./DecayForecast";

interface TxSimulationProps {
  from: string;
  to: string;
  amount: number;
  currentBalance: number;
  currentEpoch: number;
  epochDurationMs: number;
  onConfirm: () => void;
  onCancel: () => void;
}

export function TxSimulation({
  from,
  to,
  amount,
  currentBalance,
  currentEpoch,
  epochDurationMs,
  onConfirm,
  onCancel,
}: TxSimulationProps) {
  const [simulation, setSimulation] = useState<SimulationResult | null>(null);
  const [simulating, setSimulating] = useState(false);
  const [simError, setSimError] = useState<string | null>(null);
  const [showDetails, setShowDetails] = useState(false);

  const handleSimulate = async () => {
    setSimulating(true);
    setSimError(null);
    try {
      const result = await api.simulateTransaction({ from, to, amount });
      setSimulation(result);
    } catch (e: any) {
      setSimError(e.message || "Simulation failed");
    } finally {
      setSimulating(false);
    }
  };

  const warnings: Array<{ level: "amber" | "red"; message: string }> = [];

  if (simulation) {
    if (simulation.recipientIsGhost) {
      warnings.push({ level: "red", message: "Recipient address is a ghost (evaporated). Funds may be lost." });
    }
    if (simulation.balanceAfter < simulation.balanceBefore * 0.1) {
      warnings.push({ level: "amber", message: "This transaction will drop your energy below 10%." });
    }
    if (simulation.balanceAfter <= 0) {
      warnings.push({ level: "red", message: "Insufficient balance to complete this transaction." });
    }
    if (simulation.objectSurvivalEpochs !== undefined && simulation.objectSurvivalEpochs < 10) {
      warnings.push({ level: "amber", message: `Your account will only survive ${simulation.objectSurvivalEpochs} more epochs after this.` });
    }
  }

  // Before simulation: show simulate button
  if (!simulation && !simulating) {
    return (
      <div className="rounded-lg border border-evap-border bg-evap-surface p-3">
        <div className="flex items-center justify-between mb-3">
          <span className="text-[10px] font-medium text-zinc-400 uppercase tracking-wide">Transaction Preview</span>
        </div>

        {/* Summary */}
        <div className="space-y-2 mb-3">
          <div className="flex justify-between text-[11px]">
            <span className="text-zinc-500">To</span>
            <span className="text-zinc-300 font-mono">{shortAddress(to)}</span>
          </div>
          <div className="flex justify-between text-[11px]">
            <span className="text-zinc-500">Amount</span>
            <span className="text-zinc-200 font-medium">{formatBalance(amount)} EVAP</span>
          </div>
          <div className="flex justify-between text-[11px]">
            <span className="text-zinc-500">Current Balance</span>
            <span className="text-zinc-300">{formatBalance(currentBalance)} EVAP</span>
          </div>
        </div>

        {simError && (
          <div className="rounded-md bg-evap-red/10 border border-evap-red/30 px-3 py-2 mb-3">
            <p className="text-[10px] text-evap-red">{simError}</p>
          </div>
        )}

        <div className="flex gap-2">
          <button
            onClick={onCancel}
            className="flex-1 py-2 rounded-lg border border-evap-border text-[11px] text-zinc-400 hover:text-zinc-200 hover:border-zinc-500 transition"
          >
            Cancel
          </button>
          <button
            onClick={handleSimulate}
            className="flex-1 py-2 rounded-lg bg-evap-cyan/20 border border-evap-cyan/30 text-[11px] text-evap-cyan font-medium hover:bg-evap-cyan/30 transition"
          >
            Simulate
          </button>
        </div>
      </div>
    );
  }

  // Loading state
  if (simulating) {
    return (
      <div className="rounded-lg border border-evap-border bg-evap-surface p-4">
        <div className="flex flex-col items-center justify-center py-6">
          <div className="w-8 h-8 border-2 border-evap-cyan border-t-transparent rounded-full animate-spin mb-3" />
          <p className="text-[11px] text-zinc-400">Simulating transaction...</p>
          <p className="text-[9px] text-zinc-600 mt-1">Calculating energy costs and decay impact</p>
        </div>
      </div>
    );
  }

  // Simulation results
  const balanceBeforePercent = simulation!.maxEnergy > 0
    ? energyPercent(simulation!.balanceBefore, simulation!.maxEnergy)
    : 100;
  const balanceAfterPercent = simulation!.maxEnergy > 0
    ? energyPercent(simulation!.balanceAfter, simulation!.maxEnergy)
    : 0;

  const forecastData: DecayForecastData | null = simulation!.objectHalfLife
    ? {
        currentEnergy: simulation!.balanceAfter,
        maxEnergy: simulation!.maxEnergy,
        halfLife: simulation!.objectHalfLife,
        currentEpoch,
        epochDurationMs,
      }
    : null;

  return (
    <div className="rounded-lg border border-evap-border bg-evap-surface overflow-hidden">
      {/* Header */}
      <div className="px-3 py-2 border-b border-evap-border bg-evap-bg/50">
        <span className="text-[10px] font-medium text-zinc-400 uppercase tracking-wide">Simulation Result</span>
      </div>

      <div className="p-3 space-y-3">
        {/* Warnings */}
        {warnings.map((w, i) => (
          <div
            key={i}
            className={`rounded-md px-3 py-2 ${
              w.level === "red"
                ? "bg-evap-red/10 border border-evap-red/30"
                : "bg-evap-amber/10 border border-evap-amber/30"
            }`}
          >
            <p className={`text-[10px] font-medium ${w.level === "red" ? "text-evap-red" : "text-evap-amber"}`}>
              {w.level === "red" ? "Warning" : "Caution"}
            </p>
            <p className={`text-[10px] mt-0.5 ${w.level === "red" ? "text-evap-red/80" : "text-evap-amber/80"}`}>
              {w.message}
            </p>
          </div>
        ))}

        {/* Before / After comparison */}
        <div className="grid grid-cols-2 gap-2">
          {/* Before */}
          <div className="rounded-md bg-evap-bg p-2.5 border border-evap-border">
            <span className="text-[9px] text-zinc-500 uppercase tracking-wide block mb-1.5">Before</span>
            <p className="text-sm font-semibold text-zinc-200">{formatBalance(simulation!.balanceBefore)}</p>
            <p className="text-[9px] text-zinc-500">EVAP</p>
            <div className="mt-1.5 w-full h-1.5 bg-evap-border rounded-full overflow-hidden">
              <div
                className="h-full rounded-full transition-all"
                style={{
                  width: `${Math.max(balanceBeforePercent, 1)}%`,
                  backgroundColor: energyColor(balanceBeforePercent),
                }}
              />
            </div>
            <p className="text-[9px] mt-0.5" style={{ color: energyColor(balanceBeforePercent) }}>
              {balanceBeforePercent}% energy
            </p>
          </div>

          {/* After */}
          <div className="rounded-md bg-evap-bg p-2.5 border border-evap-border">
            <span className="text-[9px] text-zinc-500 uppercase tracking-wide block mb-1.5">After</span>
            <p className="text-sm font-semibold text-zinc-200">{formatBalance(simulation!.balanceAfter)}</p>
            <p className="text-[9px] text-zinc-500">EVAP</p>
            <div className="mt-1.5 w-full h-1.5 bg-evap-border rounded-full overflow-hidden">
              <div
                className="h-full rounded-full transition-all"
                style={{
                  width: `${Math.max(balanceAfterPercent, 1)}%`,
                  backgroundColor: energyColor(balanceAfterPercent),
                }}
              />
            </div>
            <p className="text-[9px] mt-0.5" style={{ color: energyColor(balanceAfterPercent) }}>
              {balanceAfterPercent}% energy
            </p>
          </div>
        </div>

        {/* Cost breakdown */}
        <div className="space-y-1.5">
          <div className="flex justify-between text-[11px]">
            <span className="text-zinc-500">Transfer amount</span>
            <span className="text-zinc-300">{formatBalance(amount)} EVAP</span>
          </div>
          <div className="flex justify-between text-[11px]">
            <span className="text-zinc-500">Gas cost</span>
            <span className="text-zinc-300">{formatBalance(simulation!.gasCost)} EVAP</span>
          </div>
          <div className="flex justify-between text-[11px]">
            <span className="text-zinc-500">Energy cost (decay impact)</span>
            <span className="text-evap-ember">{formatBalance(simulation!.energyCost)} EVAP</span>
          </div>
          <div className="flex justify-between text-[11px] pt-1.5 border-t border-evap-border">
            <span className="text-zinc-400 font-medium">Total cost</span>
            <span className="text-zinc-200 font-medium">
              {formatBalance(amount + simulation!.gasCost + simulation!.energyCost)} EVAP
            </span>
          </div>
        </div>

        {/* Survival forecast */}
        {simulation!.objectSurvivalEpochs !== undefined && (
          <div className="flex justify-between items-center text-[11px] px-2 py-1.5 rounded bg-evap-bg border border-evap-border">
            <span className="text-zinc-500">Survives after tx</span>
            <span className={`font-medium ${
              simulation!.objectSurvivalEpochs > 50 ? "text-evap-green" :
              simulation!.objectSurvivalEpochs > 10 ? "text-evap-amber" :
              "text-evap-red"
            }`}>
              {simulation!.objectSurvivalEpochs} more epochs
            </span>
          </div>
        )}

        {/* Decay forecast */}
        {forecastData && <DecayForecast data={forecastData} />}

        {/* Expandable details */}
        <button
          onClick={() => setShowDetails(!showDetails)}
          className="flex items-center gap-1 text-[10px] text-zinc-500 hover:text-zinc-300 transition"
        >
          <span className={`transition-transform ${showDetails ? "rotate-90" : ""}`}>&#9654;</span>
          Raw Transaction Data
        </button>

        {showDetails && (
          <div className="rounded-md bg-evap-bg border border-evap-border p-2 overflow-x-auto">
            <pre className="text-[9px] text-zinc-400 font-mono leading-relaxed whitespace-pre-wrap break-all">
{JSON.stringify(
  {
    type: "transfer",
    from: shortAddress(from),
    to: shortAddress(to),
    amount,
    gasCost: simulation!.gasCost,
    energyCost: simulation!.energyCost,
    nonce: simulation!.nonce,
    estimatedBlock: simulation!.estimatedBlock,
    balanceBefore: simulation!.balanceBefore,
    balanceAfter: simulation!.balanceAfter,
  },
  null,
  2
)}
            </pre>
          </div>
        )}

        {/* Action buttons */}
        <div className="flex gap-2 pt-1">
          <button
            onClick={onCancel}
            className="flex-1 py-2.5 rounded-lg border border-evap-border text-[11px] text-zinc-400 hover:text-zinc-200 hover:border-zinc-500 transition"
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            disabled={warnings.some(w => w.level === "red" && w.message.includes("Insufficient"))}
            className="flex-1 py-2.5 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-[11px] font-semibold text-black hover:opacity-90 transition disabled:opacity-40"
          >
            Confirm & Send
          </button>
        </div>
      </div>
    </div>
  );
}

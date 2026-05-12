import { useMemo } from "react";
import { energyColor } from "@/utils/format";

export interface DecayForecastData {
  currentEnergy: number;
  maxEnergy: number;
  halfLife: number; // in epochs
  currentEpoch: number;
  epochDurationMs: number; // milliseconds per epoch
}

interface DecayForecastProps {
  data: DecayForecastData;
  compact?: boolean;
}

/**
 * Calculates how many epochs until energy drops below a threshold,
 * using exponential decay: E(t) = E0 * (0.5)^(t / halfLife)
 */
function epochsUntilThreshold(
  currentEnergy: number,
  maxEnergy: number,
  halfLife: number,
  thresholdPercent: number
): number {
  if (currentEnergy <= 0 || maxEnergy <= 0 || halfLife <= 0) return 0;
  const thresholdEnergy = maxEnergy * (thresholdPercent / 100);
  if (currentEnergy <= thresholdEnergy) return 0;
  // t = halfLife * log2(currentEnergy / thresholdEnergy)
  return Math.floor(halfLife * Math.log2(currentEnergy / thresholdEnergy));
}

function formatDate(ms: number): string {
  const d = new Date(ms);
  return d.toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" });
}

function formatDuration(epochs: number, epochDurationMs: number): string {
  const totalMs = epochs * epochDurationMs;
  const hours = totalMs / 3_600_000;
  if (hours < 1) return `${Math.round(totalMs / 60_000)}m`;
  if (hours < 24) return `${Math.round(hours)}h`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days}d`;
  return `${Math.round(days / 30)}mo`;
}

export function DecayForecast({ data, compact = false }: DecayForecastProps) {
  const { currentEnergy, maxEnergy, halfLife, currentEpoch, epochDurationMs } = data;

  const forecast = useMemo(() => {
    const percent = maxEnergy > 0 ? Math.round((currentEnergy / maxEnergy) * 100) : 0;

    // Epochs until key thresholds
    const epochsToGrace = epochsUntilThreshold(currentEnergy, maxEnergy, halfLife, 10); // Grace at 10%
    const epochsToGhost = epochsUntilThreshold(currentEnergy, maxEnergy, halfLife, 1);  // Ghost at 1%
    const epochsToZero = epochsUntilThreshold(currentEnergy, maxEnergy, halfLife, 0.1); // Effectively 0

    const evaporationEpoch = currentEpoch + epochsToZero;
    const evaporationDate = Date.now() + epochsToZero * epochDurationMs;

    // Urgency level
    let urgency: "green" | "amber" | "red";
    if (percent > 50) urgency = "green";
    else if (percent > 20) urgency = "amber";
    else urgency = "red";

    // Timeline segments for the bar (normalized to epochsToZero or a minimum)
    const totalSpan = Math.max(epochsToZero, 1);
    const graceStart = ((totalSpan - epochsToGrace) / totalSpan) * 100;
    const ghostStart = ((totalSpan - epochsToGhost) / totalSpan) * 100;
    const currentPos = 0; // current is always "now" at the left

    return {
      percent,
      epochsToGrace,
      epochsToGhost,
      epochsToZero,
      evaporationEpoch,
      evaporationDate,
      urgency,
      graceStart,
      ghostStart,
      currentPos,
    };
  }, [currentEnergy, maxEnergy, halfLife, currentEpoch, epochDurationMs]);

  const urgencyColors = {
    green: { bg: "bg-evap-green/10", border: "border-evap-green/30", text: "text-evap-green", dot: "bg-evap-green" },
    amber: { bg: "bg-evap-amber/10", border: "border-evap-amber/30", text: "text-evap-amber", dot: "bg-evap-amber" },
    red: { bg: "bg-evap-red/10", border: "border-evap-red/30", text: "text-evap-red", dot: "bg-evap-red" },
  };

  const colors = urgencyColors[forecast.urgency];

  if (compact) {
    return (
      <div className={`flex items-center gap-2 px-2 py-1.5 rounded ${colors.bg}`}>
        <div className={`w-1.5 h-1.5 rounded-full ${colors.dot}`} />
        <span className={`text-xs ${colors.text}`}>
          {forecast.epochsToZero > 0
            ? `~${formatDuration(forecast.epochsToZero, epochDurationMs)} remaining`
            : "Evaporated"}
        </span>
      </div>
    );
  }

  return (
    <div className={`rounded-lg border p-3 ${colors.bg} ${colors.border}`}>
      <div className="flex items-center justify-between mb-2">
        <span className="text-xs font-medium text-zinc-400 uppercase tracking-wide">Decay Forecast</span>
        <div className="flex items-center gap-1">
          <div className={`w-1.5 h-1.5 rounded-full ${colors.dot}`} />
          <span className={`text-xs font-medium ${colors.text}`}>
            {forecast.percent}% energy
          </span>
        </div>
      </div>

      {/* Timeline bar */}
      <div className="relative w-full h-3 bg-evap-border rounded-full overflow-hidden mb-2">
        {/* Healthy zone */}
        <div
          className="absolute top-0 left-0 h-full bg-evap-green/40 rounded-l-full"
          style={{ width: `${forecast.graceStart}%` }}
        />
        {/* Grace zone */}
        <div
          className="absolute top-0 h-full bg-evap-amber/40"
          style={{ left: `${forecast.graceStart}%`, width: `${forecast.ghostStart - forecast.graceStart}%` }}
        />
        {/* Ghost zone */}
        <div
          className="absolute top-0 h-full bg-evap-red/40 rounded-r-full"
          style={{ left: `${forecast.ghostStart}%`, width: `${100 - forecast.ghostStart}%` }}
        />
        {/* Current position marker */}
        <div
          className="absolute top-0 w-0.5 h-full bg-white"
          style={{ left: `${Math.min(100 - forecast.percent, 99)}%` }}
        />
      </div>

      {/* Labels */}
      <div className="flex justify-between text-[9px] text-zinc-500 mb-2">
        <span>Now</span>
        <span>Grace</span>
        <span>Ghost</span>
      </div>

      {/* Evaporation prediction */}
      <div className="flex items-center justify-between pt-2 border-t border-evap-border">
        <span className="text-xs text-zinc-500">Evaporates on</span>
        <span className={`text-xs font-medium ${colors.text}`}>
          {forecast.epochsToZero > 0
            ? formatDate(forecast.evaporationDate)
            : "Already evaporated"}
        </span>
      </div>

      {/* Time remaining */}
      {forecast.epochsToZero > 0 && (
        <div className="flex items-center justify-between mt-1">
          <span className="text-xs text-zinc-500">Time remaining</span>
          <span className="text-xs text-zinc-300">
            ~{formatDuration(forecast.epochsToZero, epochDurationMs)} ({forecast.epochsToZero} epochs)
          </span>
        </div>
      )}
    </div>
  );
}

import { energyColor, energyPercent } from "@/utils/format";

interface EnergyBarProps {
  current: number;
  max: number;
  showLabel?: boolean;
  size?: "sm" | "md";
}

export function EnergyBar({ current, max, showLabel = true, size = "md" }: EnergyBarProps) {
  const percent = energyPercent(current, max);
  const color = energyColor(percent);
  const height = size === "sm" ? "h-1.5" : "h-2.5";

  return (
    <div className="w-full">
      <div className={`w-full bg-evap-border rounded-full ${height} overflow-hidden`}>
        <div
          className={`${height} rounded-full transition-all duration-1000 ${percent <= 5 ? "energy-critical" : ""}`}
          style={{ width: `${Math.max(percent, 1)}%`, backgroundColor: color }}
        />
      </div>
      {showLabel && (
        <div className="flex justify-between mt-1">
          <span className="text-xs text-zinc-500">{current.toLocaleString()} / {max.toLocaleString()}</span>
          <span className="text-xs font-medium" style={{ color }}>{percent}%</span>
        </div>
      )}
    </div>
  );
}

interface EnergyGaugeProps {
  percent: number;
  size?: number;
  strokeWidth?: number;
  label?: string;
  showText?: boolean;
}

export function EnergyGauge({
  percent,
  size = 80,
  strokeWidth = 6,
  label,
  showText = true,
}: EnergyGaugeProps) {
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const offset = circumference - (percent / 100) * circumference;

  const color =
    percent >= 70
      ? "text-evap-green"
      : percent >= 40
        ? "text-evap-amber"
        : percent >= 15
          ? "text-evap-ember"
          : "text-evap-red";

  const strokeColor =
    percent >= 70
      ? "#16a34a"
      : percent >= 40
        ? "#d97706"
        : percent >= 15
          ? "#ea580c"
          : "#dc2626";

  return (
    <div className="relative inline-flex items-center justify-center" style={{ width: size, height: size }}>
      <svg width={size} height={size} className="-rotate-90">
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="#e4e4e7"
          strokeWidth={strokeWidth}
        />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke={strokeColor}
          strokeWidth={strokeWidth}
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          style={{ transition: "stroke-dashoffset 0.6s ease" }}
        />
      </svg>
      {showText && (
        <div className="absolute inset-0 flex flex-col items-center justify-center">
          <span className={`text-sm font-bold ${color}`}>{Math.round(percent)}%</span>
          {label && <span className="text-[8px] text-zinc-400">{label}</span>}
        </div>
      )}
    </div>
  );
}

interface EnergyBarProps {
  current: number;
  max: number;
  height?: number;
}

export function EnergyBar({ current, max, height = 6 }: EnergyBarProps) {
  const pct = max > 0 ? Math.min((current / max) * 100, 100) : 0;
  const bg =
    pct >= 70
      ? "bg-gradient-to-r from-evap-cyan to-evap-green"
      : pct >= 40
        ? "bg-evap-amber"
        : pct >= 15
          ? "bg-evap-ember"
          : "bg-evap-red";

  return (
    <div className="w-full bg-zinc-100 rounded-full overflow-hidden" style={{ height }}>
      <div
        className={`h-full rounded-full transition-all duration-500 ${bg}`}
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}

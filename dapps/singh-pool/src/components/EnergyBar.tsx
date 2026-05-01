interface Props {
  energy: number;
  maxEnergy: number;
  state: "Active" | "Grace" | "Ghost";
}

// Visualises an LP position's energy reserve. The bar tone matches the
// position's substrate state: emerald above 60%, amber 25-60%, red below,
// grey when ghosted.
export function EnergyBar({ energy, maxEnergy, state }: Props) {
  const pct =
    maxEnergy > 0 ? Math.max(0, Math.min(100, (energy / maxEnergy) * 100)) : 0;
  const tone =
    state === "Ghost"
      ? "bg-zinc-300"
      : pct >= 60
        ? "bg-evap-green"
        : pct >= 25
          ? "bg-evap-amber"
          : "bg-evap-red";
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-[11px] text-zinc-500">
        <span className="font-medium">Energy</span>
        <span className="font-mono">
          {energy.toLocaleString()} / {maxEnergy.toLocaleString()}
        </span>
      </div>
      <div className="h-2 overflow-hidden rounded-full bg-zinc-100">
        <div
          className={`h-full rounded-full transition-all duration-700 ${tone}`}
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

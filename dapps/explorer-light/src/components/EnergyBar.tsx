// Reused from singh-pool — visualises an object's energy reserve. Kept
// in-tree so explorer-light has no cross-dApp imports.

interface Props {
  energy: number;
  maxEnergy: number;
  state: string;
}

export function EnergyBar({ energy, maxEnergy, state }: Props) {
  const pct =
    maxEnergy > 0 ? Math.max(0, Math.min(100, (energy / maxEnergy) * 100)) : 0;
  const isGhost = state === "Ghost";
  const tone = isGhost
    ? "bg-zinc-300"
    : pct >= 60
      ? "bg-evap-green"
      : pct >= 25
        ? "bg-evap-amber"
        : "bg-evap-red";
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-[10px] text-zinc-500">
        <span className="font-medium">{state}</span>
        <span className="font-mono">
          {energy.toLocaleString()} / {maxEnergy.toLocaleString()}
        </span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-zinc-100">
        <div
          className={`h-full rounded-full transition-all duration-700 ${tone}`}
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

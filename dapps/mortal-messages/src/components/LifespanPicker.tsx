import { HALF_LIFE_PRESETS } from "@/utils/types";

interface Props {
  value: number;
  onChange: (epochs: number) => void;
}

export default function LifespanPicker({ value, onChange }: Props) {
  return (
    <div className="space-y-2">
      <label className="block text-sm font-medium text-zinc-700">
        Half-Life (decay rate)
      </label>
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        {HALF_LIFE_PRESETS.map((preset) => (
          <button
            key={preset.epochs}
            type="button"
            onClick={() => onChange(preset.epochs)}
            className={`rounded-lg border px-3 py-2 text-sm font-medium transition-colors ${
              value === preset.epochs
                ? "border-evap-cyan bg-evap-cyan/10 text-evap-cyan"
                : "border-evap-border bg-evap-surface text-zinc-600 hover:border-zinc-300"
            }`}
          >
            <div>{preset.label}</div>
            <div className="mt-0.5 text-xs opacity-70">{preset.description}</div>
          </button>
        ))}
      </div>
      <div className="flex items-center gap-2 text-xs text-zinc-500">
        <span>Custom epochs:</span>
        <input
          type="number"
          min={1}
          value={value}
          onChange={(e) => onChange(Math.max(1, Number(e.target.value)))}
          className="w-24 rounded border border-evap-border bg-evap-surface px-2 py-1 text-sm"
        />
      </div>
      <div className="rounded-lg border border-evap-border bg-zinc-50 p-3">
        <div className="text-xs text-zinc-500">Estimated lifespan</div>
        <div className="mt-1 text-sm font-medium text-zinc-700">
          ~{formatLifespan(value)} until evaporation (10 half-lives)
        </div>
        <div className="mt-2 flex h-2 overflow-hidden rounded-full bg-zinc-200">
          <div className="h-full rounded-full bg-evap-cyan" style={{ width: "100%" }} />
          <div className="h-full bg-evap-amber" style={{ width: "30%" }} />
          <div className="h-full bg-zinc-400" style={{ width: "10%" }} />
        </div>
        <div className="mt-1 flex justify-between text-[10px] text-zinc-400">
          <span>Active</span>
          <span>Grace</span>
          <span>Ghost</span>
        </div>
      </div>
    </div>
  );
}

function formatLifespan(halfLifeEpochs: number): string {
  const totalMinutes = halfLifeEpochs * 10;
  if (totalMinutes < 60) return `${totalMinutes} min`;
  const hours = totalMinutes / 60;
  if (hours < 24) return `${hours.toFixed(1)} hours`;
  const days = hours / 24;
  if (days < 30) return `${days.toFixed(1)} days`;
  const months = days / 30;
  return `${months.toFixed(1)} months`;
}

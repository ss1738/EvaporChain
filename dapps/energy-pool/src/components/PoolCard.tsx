import type { Pool } from "@/utils/types";
import { EnergyGauge } from "./EnergyGauge";

interface PoolCardProps {
  pool: Pool;
  onSelect: (pool: Pool) => void;
  onStake: (pool: Pool) => void;
}

export function PoolCard({ pool, onSelect, onStake }: PoolCardProps) {
  const strategyLabel =
    pool.strategy === "equal" ? "Equal Split" : "Priority Low-Energy";

  return (
    <div
      className="bg-white rounded-xl border border-evap-border p-5 hover:shadow-md transition cursor-pointer"
      onClick={() => onSelect(pool)}
    >
      {/* Header */}
      <div className="flex items-start justify-between mb-4">
        <div className="flex-1 min-w-0 mr-3">
          <h3 className="text-sm font-bold text-zinc-900 truncate">{pool.name}</h3>
          <p className="text-[10px] text-zinc-400 mt-0.5 line-clamp-2">
            {pool.description}
          </p>
        </div>
        <EnergyGauge percent={pool.health_pct} size={56} strokeWidth={4} />
      </div>

      {/* Stats Grid */}
      <div className="grid grid-cols-3 gap-3 mb-4">
        <div>
          <p className="text-xs font-bold text-evap-cyan">
            {pool.total_energy.toLocaleString()}
          </p>
          <p className="text-[9px] text-zinc-400">Energy Staked</p>
        </div>
        <div>
          <p className="text-xs font-bold text-zinc-700">
            {pool.protected_objects.length}
          </p>
          <p className="text-[9px] text-zinc-400">Objects</p>
        </div>
        <div>
          <p className="text-xs font-bold text-evap-purple">
            {pool.contributor_count}
          </p>
          <p className="text-[9px] text-zinc-400">Contributors</p>
        </div>
      </div>

      {/* Footer */}
      <div className="flex items-center justify-between pt-3 border-t border-evap-border">
        <span className="text-[9px] px-2 py-0.5 rounded-full bg-zinc-100 text-zinc-500">
          {strategyLabel}
        </span>
        <button
          onClick={(e) => {
            e.stopPropagation();
            onStake(pool);
          }}
          className="px-3 py-1.5 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-green text-[10px] font-semibold text-white hover:opacity-90 transition"
        >
          + Stake Energy
        </button>
      </div>
    </div>
  );
}

import { useState, useEffect, useCallback } from "react";
import { api } from "@/utils/api";
import type { UserDashboard } from "@/utils/types";
import { EnergyGauge } from "./EnergyGauge";

interface DashboardProps {
  address: string;
  onSelectPool: (poolId: string) => void;
}

export function Dashboard({ address, onSelectPool }: DashboardProps) {
  const [data, setData] = useState<UserDashboard | null>(null);
  const [loading, setLoading] = useState(true);

  const fetchData = useCallback(async () => {
    try {
      const d = await api.getDashboard(address);
      setData(d);
    } catch {
      // retry on next poll
    } finally {
      setLoading(false);
    }
  }, [address]);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 8000);
    return () => clearInterval(interval);
  }, [fetchData]);

  if (loading) {
    return (
      <div className="flex items-center justify-center py-20">
        <div className="w-8 h-8 border-2 border-evap-cyan/30 border-t-evap-cyan rounded-full animate-spin" />
      </div>
    );
  }

  if (!data) {
    return (
      <div className="flex flex-col items-center justify-center py-20 bg-white rounded-xl border border-evap-border">
        <p className="text-sm text-zinc-500">Could not load dashboard</p>
      </div>
    );
  }

  return (
    <div>
      <h2 className="text-base font-bold text-zinc-900 mb-4">Your Dashboard</h2>

      {/* Summary stats */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 mb-6">
        <div className="bg-white rounded-xl border border-evap-border p-5 text-center">
          <p className="text-2xl font-bold text-evap-cyan">
            {data.total_staked.toLocaleString()}
          </p>
          <p className="text-[10px] text-zinc-400 mt-1">Total Energy Staked</p>
        </div>
        <div className="bg-white rounded-xl border border-evap-border p-5 text-center">
          <div className="flex items-center justify-center gap-1">
            <span className="text-2xl font-bold text-evap-purple">
              {data.total_guardian_points.toLocaleString()}
            </span>
            <span className="text-sm text-zinc-400">{`{*}`}</span>
          </div>
          <p className="text-[10px] text-zinc-400 mt-1">Guardian Points</p>
        </div>
        <div className="bg-white rounded-xl border border-evap-border p-5 text-center">
          <p className="text-2xl font-bold text-evap-green">
            {data.objects_saved}
          </p>
          <p className="text-[10px] text-zinc-400 mt-1">Objects Saved</p>
        </div>
      </div>

      {/* Pool stakes */}
      <h3 className="text-sm font-semibold text-zinc-700 mb-3">Your Pool Stakes</h3>

      {data.pools.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 bg-white rounded-xl border border-evap-border">
          <p className="text-xs text-zinc-500">
            You haven't staked in any pools yet
          </p>
          <p className="text-[10px] text-zinc-400 mt-1">
            Browse pools and stake energy to start earning guardian points
          </p>
        </div>
      ) : (
        <div className="space-y-2">
          {data.pools.map((ps) => (
            <div
              key={ps.pool_id}
              onClick={() => onSelectPool(ps.pool_id)}
              className="bg-white rounded-lg border border-evap-border p-4 flex items-center gap-4 cursor-pointer hover:shadow-sm transition"
            >
              <EnergyGauge percent={ps.health_pct} size={48} strokeWidth={4} />
              <div className="flex-1 min-w-0">
                <p className="text-xs font-semibold text-zinc-900 truncate">
                  {ps.pool_name}
                </p>
                <p className="text-[10px] text-zinc-400">
                  {ps.staked_energy.toLocaleString()} energy staked
                </p>
              </div>
              <div className="text-right shrink-0">
                <p className="text-xs font-bold text-evap-purple">
                  {ps.guardian_points.toLocaleString()}
                </p>
                <p className="text-[9px] text-zinc-400">guardian pts</p>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

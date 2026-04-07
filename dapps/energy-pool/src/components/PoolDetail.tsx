import { useState, useEffect, useCallback } from "react";
import { api } from "@/utils/api";
import type { Pool, Contributor, PoolActivity } from "@/utils/types";
import { EnergyGauge, EnergyBar } from "./EnergyGauge";
import { ContributorRow } from "./ContributorRow";

interface PoolDetailProps {
  poolId: string;
  walletAddress: string | null;
  onBack: () => void;
  onStake: (pool: Pool) => void;
}

type DetailTab = "objects" | "contributors" | "activity";

export function PoolDetail({ poolId, walletAddress, onBack, onStake }: PoolDetailProps) {
  const [pool, setPool] = useState<Pool | null>(null);
  const [contributors, setContributors] = useState<Contributor[]>([]);
  const [activity, setActivity] = useState<PoolActivity[]>([]);
  const [tab, setTab] = useState<DetailTab>("objects");
  const [loading, setLoading] = useState(true);

  const fetchData = useCallback(async () => {
    try {
      const [poolData, contribData, activityData] = await Promise.all([
        api.getPool(poolId),
        api.getContributors(poolId),
        api.getActivity(poolId),
      ]);
      setPool(poolData);
      setContributors(contribData);
      setActivity(activityData);
    } catch {
      // retry on next poll
    } finally {
      setLoading(false);
    }
  }, [poolId]);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 8000);
    return () => clearInterval(interval);
  }, [fetchData]);

  if (loading || !pool) {
    return (
      <div className="flex items-center justify-center py-20">
        <div className="w-8 h-8 border-2 border-evap-cyan/30 border-t-evap-cyan rounded-full animate-spin" />
      </div>
    );
  }

  const stateColor = (state: string) =>
    state === "Active"
      ? "text-evap-green"
      : state === "Grace"
        ? "text-evap-amber"
        : "text-evap-ghost";

  const actionLabel = (action: string) => {
    switch (action) {
      case "stake": return "Staked energy";
      case "unstake": return "Unstaked energy";
      case "distribute": return "Energy distributed";
      case "object_saved": return "Object saved from evaporation";
      case "object_lost": return "Object evaporated";
      default: return action;
    }
  };

  const actionColor = (action: string) => {
    switch (action) {
      case "stake": return "text-evap-green";
      case "unstake": return "text-evap-ember";
      case "distribute": return "text-evap-cyan";
      case "object_saved": return "text-evap-purple";
      case "object_lost": return "text-evap-red";
      default: return "text-zinc-500";
    }
  };

  return (
    <div>
      {/* Back button */}
      <button
        onClick={onBack}
        className="flex items-center gap-1 text-xs text-zinc-500 hover:text-zinc-700 mb-4 transition"
      >
        &larr; Back to Pools
      </button>

      {/* Pool Header */}
      <div className="bg-white rounded-xl border border-evap-border p-6 mb-6">
        <div className="flex items-start gap-5">
          <EnergyGauge percent={pool.health_pct} size={90} strokeWidth={6} label="Health" />
          <div className="flex-1">
            <h2 className="text-lg font-bold text-zinc-900">{pool.name}</h2>
            <p className="text-xs text-zinc-500 mt-1">{pool.description}</p>

            <div className="grid grid-cols-2 sm:grid-cols-4 gap-4 mt-4">
              <div>
                <p className="text-sm font-bold text-evap-cyan">
                  {pool.total_energy.toLocaleString()}
                </p>
                <p className="text-[9px] text-zinc-400">Total Energy</p>
              </div>
              <div>
                <p className="text-sm font-bold text-zinc-700">
                  {pool.protected_objects.length}
                </p>
                <p className="text-[9px] text-zinc-400">Protected Objects</p>
              </div>
              <div>
                <p className="text-sm font-bold text-evap-purple">
                  {pool.contributor_count}
                </p>
                <p className="text-[9px] text-zinc-400">Contributors</p>
              </div>
              <div>
                <p className="text-sm font-bold text-zinc-700 capitalize">
                  {pool.strategy === "equal" ? "Equal Split" : "Priority Low"}
                </p>
                <p className="text-[9px] text-zinc-400">Strategy</p>
              </div>
            </div>
          </div>

          <button
            onClick={() => onStake(pool)}
            className="px-4 py-2 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-green text-xs font-semibold text-white hover:opacity-90 transition shrink-0"
          >
            + Stake Energy
          </button>
        </div>

        {/* Creator */}
        <div className="mt-4 pt-3 border-t border-evap-border flex items-center gap-2">
          <span className="text-[10px] text-zinc-400">Created by</span>
          <span className="text-[10px] font-mono text-zinc-600">
            {pool.creator.slice(0, 8)}...{pool.creator.slice(-6)}
          </span>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex bg-white rounded-lg border border-evap-border p-0.5 mb-4">
        <TabBtn
          label="Protected Objects"
          active={tab === "objects"}
          onClick={() => setTab("objects")}
          count={pool.protected_objects.length}
        />
        <TabBtn
          label="Contributors"
          active={tab === "contributors"}
          onClick={() => setTab("contributors")}
          count={contributors.length}
        />
        <TabBtn
          label="Activity"
          active={tab === "activity"}
          onClick={() => setTab("activity")}
          count={activity.length}
        />
      </div>

      {/* Tab Content */}
      {tab === "objects" && (
        <div className="space-y-2">
          {pool.protected_objects.length === 0 ? (
            <EmptyState message="No protected objects yet" />
          ) : (
            pool.protected_objects.map((obj) => (
              <div
                key={obj.object_id}
                className="bg-white rounded-lg border border-evap-border p-4"
              >
                <div className="flex items-center justify-between mb-2">
                  <div>
                    <span className="text-xs font-semibold text-zinc-900">
                      {obj.name}
                    </span>
                    <span className="text-[9px] text-zinc-400 ml-2">
                      {obj.object_type}
                    </span>
                  </div>
                  <span className={`text-[10px] font-medium ${stateColor(obj.state)}`}>
                    {obj.state}
                  </span>
                </div>
                <EnergyBar current={obj.current_energy} max={obj.max_energy} />
                <div className="flex items-center justify-between mt-2">
                  <span className="text-[9px] text-zinc-400">
                    {obj.current_energy.toLocaleString()} / {obj.max_energy.toLocaleString()} energy
                  </span>
                  <span className="text-[9px] text-evap-cyan">
                    +{obj.energy_received.toLocaleString()} from pool
                  </span>
                </div>
              </div>
            ))
          )}
        </div>
      )}

      {tab === "contributors" && (
        <div className="space-y-2">
          {contributors.length === 0 ? (
            <EmptyState message="No contributors yet. Be the first!" />
          ) : (
            contributors.map((c) => (
              <ContributorRow
                key={c.address}
                contributor={c}
                isCurrentUser={
                  !!walletAddress &&
                  c.address.toLowerCase() === walletAddress.toLowerCase()
                }
              />
            ))
          )}
        </div>
      )}

      {tab === "activity" && (
        <div className="space-y-1">
          {activity.length === 0 ? (
            <EmptyState message="No activity yet" />
          ) : (
            activity.map((a) => (
              <div
                key={a.id}
                className="flex items-center gap-3 px-4 py-2.5 bg-white rounded-lg border border-evap-border"
              >
                <span className={`text-[10px] font-medium w-28 shrink-0 ${actionColor(a.action)}`}>
                  {actionLabel(a.action)}
                </span>
                <span className="text-[10px] font-mono text-zinc-500 truncate">
                  {a.address.slice(0, 8)}...{a.address.slice(-4)}
                </span>
                {a.amount > 0 && (
                  <span className="text-[10px] font-semibold text-zinc-700">
                    {a.amount.toLocaleString()} energy
                  </span>
                )}
                <span className="text-[9px] text-zinc-400 ml-auto shrink-0">
                  Epoch {a.epoch}
                </span>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}

function TabBtn({
  label,
  active,
  onClick,
  count,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
  count: number;
}) {
  return (
    <button
      onClick={onClick}
      className={`px-3 py-1.5 rounded-md text-xs font-medium transition ${
        active ? "bg-zinc-100 text-zinc-900" : "text-zinc-500 hover:text-zinc-700"
      }`}
    >
      {label} <span className="text-zinc-400">({count})</span>
    </button>
  );
}

function EmptyState({ message }: { message: string }) {
  return (
    <div className="flex flex-col items-center justify-center py-12 bg-white rounded-xl border border-evap-border">
      <span className="text-2xl mb-2">--</span>
      <p className="text-xs text-zinc-500">{message}</p>
    </div>
  );
}

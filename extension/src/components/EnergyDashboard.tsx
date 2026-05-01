import { useEffect, useState } from "react";
import { useWallet } from "@/hooks/useWallet";
import { api, type StateObject, type EnergyPortfolio, type EnergyHistory } from "@/utils/api";
import { EnergyBar } from "./EnergyBar";
import { energyPercent, energyColor } from "@/utils/format";
import { BellBeaconCard } from "./BellBeaconCard";
import { HotColdStakeCard } from "./HotColdStakeCard";

export function EnergyDashboard() {
  const { activeAccount, objects, setView, refreshObjects, chainStatus, shardsHealth } = useWallet();
  const [portfolio, setPortfolio] = useState<EnergyPortfolio | null>(null);
  const [history, setHistory] = useState<EnergyHistory | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!activeAccount) return;
    setLoading(true);

    Promise.all([
      refreshObjects(),
      api.getEnergyPortfolio(activeAccount.address).catch(() => null),
      api.getEnergyHistory(activeAccount.address).catch(() => null),
    ]).then(([, p, h]) => {
      setPortfolio(p);
      setHistory(h);
      setLoading(false);
    });
  }, [activeAccount, refreshObjects]);

  if (!activeAccount) return null;

  // Derive metrics from objects store
  const totalEnergy = objects.reduce((sum, o) => sum + o.current_energy, 0);
  const totalMaxEnergy = objects.reduce((sum, o) => sum + o.max_energy, 0);
  const atRisk = objects.filter((o) => {
    const pct = o.max_energy > 0 ? (o.current_energy / o.max_energy) * 100 : 0;
    return pct < 20 && o.state !== "Ghost";
  });
  const expiringToday = objects.filter((o) => {
    const pct = o.max_energy > 0 ? (o.current_energy / o.max_energy) * 100 : 0;
    return pct < 5 && pct > 0 && o.state !== "Ghost";
  });
  const ghosts = objects.filter((o) => o.state === "Ghost");

  // Use portfolio trend if available, else derive
  const trendUp = portfolio ? portfolio.energy_trend >= 0 : totalEnergy > 0;

  // Build epoch chart data from history or mock from objects
  const epochBars = history
    ? history.epochs.slice(-7)
    : generateMockEpochs(totalEnergy, 7);

  const maxBarValue = Math.max(...epochBars.map((e) => e.energy), 1);

  // Weekly report from portfolio or defaults
  const weeklyReport = portfolio
    ? {
        refreshed: portfolio.objects_refreshed_this_week,
        evaporated: portfolio.objects_evaporated_this_week,
        energySpent: portfolio.total_energy_spent_this_week,
        netChange: portfolio.net_energy_change_this_week,
      }
    : {
        refreshed: 0,
        evaporated: ghosts.length,
        energySpent: 0,
        netChange: 0,
      };

  // Sort objects by urgency: ghosts first, then by energy percentage ascending
  const sortedObjects = [...objects].sort((a, b) => {
    if (a.state === "Ghost" && b.state !== "Ghost") return -1;
    if (b.state === "Ghost" && a.state !== "Ghost") return 1;
    const pctA = a.max_energy > 0 ? a.current_energy / a.max_energy : 0;
    const pctB = b.max_energy > 0 ? b.current_energy / b.max_energy : 0;
    return pctA - pctB;
  });

  return (
    <div className="flex flex-col h-full bg-evap-bg">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-evap-border">
        <button
          onClick={() => setView("home")}
          className="text-zinc-400 hover:text-zinc-200 transition text-sm"
        >
          ←
        </button>
        <div className="flex-1">
          <h1 className="text-sm font-semibold text-zinc-200">
            Energy Dashboard
          </h1>
          {chainStatus && (
            <p className="text-[10px] text-zinc-500">
              Block {chainStatus.block_height.toLocaleString()}
              {shardsHealth?.active && shardsHealth.total_shards > 0 && (
                <> · {shardsHealth.total_shards} shard
                {shardsHealth.total_shards === 1 ? "" : "s"}</>
              )}
            </p>
          )}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {/* Secondary chain-surface link */}
        <div className="mx-4 mt-3 flex justify-end">
          <button
            onClick={() => setView("refresh-pool")}
            className="text-[10px] px-2 py-1 rounded text-evap-cyan border border-evap-cyan/30 hover:border-evap-cyan/60 transition"
          >
            Refresh pool ↗
          </button>
        </div>

        {/* Bell-Beacon CHSH card — quantum-randomness certification */}
        <div className="mx-4 mt-3">
          <BellBeaconCard />
        </div>

        {/* Hot/Cold stake — two-temperature validator equilibrium */}
        <div className="mx-4 mt-3">
          <HotColdStakeCard />
        </div>

        {/* Urgency banner */}
        {expiringToday.length > 0 && (
          <div className="mx-4 mt-3 px-3 py-2 rounded-lg bg-evap-amber/10 border border-evap-amber/30">
            <p className="text-[11px] text-evap-amber text-center font-medium">
              ⚠ {expiringToday.length} object{expiringToday.length > 1 ? "s" : ""} expiring today
            </p>
          </div>
        )}

        {/* Summary cards */}
        <div className="grid grid-cols-2 gap-2 px-4 mt-3">
          <SummaryCard
            label="Total Energy"
            value={totalEnergy.toLocaleString()}
            sub={totalMaxEnergy > 0 ? `${energyPercent(totalEnergy, totalMaxEnergy)}% of max` : ""}
            icon={trendUp ? "↑" : "↓"}
            iconColor={trendUp ? "text-evap-green" : "text-evap-red"}
          />
          <SummaryCard
            label="At Risk"
            value={atRisk.length.toString()}
            sub="below 20%"
            highlight={atRisk.length > 0 ? "red" : undefined}
          />
          <SummaryCard
            label="Expiring Today"
            value={expiringToday.length.toString()}
            sub="need refresh"
            highlight={expiringToday.length > 0 ? "amber" : undefined}
          />
          <SummaryCard
            label="Ghosts"
            value={ghosts.length.toString()}
            sub="evaporated"
            icon="💀"
          />
        </div>

        {/* Portfolio energy chart */}
        <div className="mx-4 mt-4 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <p className="text-[10px] text-zinc-400 font-semibold mb-3 uppercase tracking-wider">
            Portfolio Energy — Last 7 Epochs
          </p>
          <div className="flex items-end justify-between gap-1 h-24">
            {epochBars.map((bar, i) => {
              const heightPct = maxBarValue > 0 ? (bar.energy / maxBarValue) * 100 : 0;
              return (
                <div
                  key={i}
                  className="flex-1 flex flex-col items-center gap-1"
                >
                  <div className="w-full flex items-end justify-center h-20">
                    <div
                      className="w-full max-w-[28px] rounded-t transition-all duration-500"
                      style={{
                        height: `${Math.max(heightPct, 3)}%`,
                        backgroundColor:
                          heightPct > 50
                            ? "#22c55e"
                            : heightPct > 20
                            ? "#f59e0b"
                            : "#ef4444",
                      }}
                    />
                  </div>
                  <span className="text-[8px] text-zinc-600">E{bar.epoch}</span>
                </div>
              );
            })}
          </div>
        </div>

        {/* Weekly energy report */}
        <div className="mx-4 mt-3 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <p className="text-[10px] text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
            Weekly Energy Report
          </p>
          <div className="grid grid-cols-2 gap-y-2 gap-x-4">
            <ReportRow label="Objects Refreshed" value={weeklyReport.refreshed.toString()} />
            <ReportRow label="Objects Evaporated" value={weeklyReport.evaporated.toString()} />
            <ReportRow label="Energy Spent" value={weeklyReport.energySpent.toLocaleString()} />
            <ReportRow
              label="Net Change"
              value={`${weeklyReport.netChange >= 0 ? "+" : ""}${weeklyReport.netChange.toLocaleString()}`}
              valueColor={weeklyReport.netChange >= 0 ? "text-evap-green" : "text-evap-red"}
            />
          </div>
        </div>

        {/* Object health list */}
        <div className="mx-4 mt-4 mb-4">
          <p className="text-[10px] text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
            Object Health
          </p>
          {loading && objects.length === 0 ? (
            <p className="text-xs text-zinc-600 text-center py-8">
              Loading objects...
            </p>
          ) : sortedObjects.length === 0 ? (
            <p className="text-xs text-zinc-600 text-center py-8">
              No objects found
            </p>
          ) : (
            <div className="space-y-1">
              {sortedObjects.map((obj) => (
                <ObjectHealthRow key={obj.id} object={obj} />
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ── Sub-components ──

function SummaryCard({
  label,
  value,
  sub,
  icon,
  iconColor,
  highlight,
}: {
  label: string;
  value: string;
  sub?: string;
  icon?: string;
  iconColor?: string;
  highlight?: "red" | "amber";
}) {
  const borderColor =
    highlight === "red"
      ? "border-evap-red/40 bg-evap-red/5"
      : highlight === "amber"
      ? "border-evap-amber/40 bg-evap-amber/5"
      : "border-evap-border bg-evap-surface";

  return (
    <div className={`rounded-lg px-3 py-3 border ${borderColor}`}>
      <p className="text-[10px] text-zinc-500 mb-1">{label}</p>
      <div className="flex items-baseline gap-1">
        <span className="text-lg font-bold text-zinc-200">{value}</span>
        {icon && (
          <span className={`text-sm ${iconColor ?? "text-zinc-500"}`}>
            {icon}
          </span>
        )}
      </div>
      {sub && <p className="text-[10px] text-zinc-600 mt-0.5">{sub}</p>}
    </div>
  );
}

function ReportRow({
  label,
  value,
  valueColor,
}: {
  label: string;
  value: string;
  valueColor?: string;
}) {
  return (
    <div className="flex justify-between items-center">
      <span className="text-[10px] text-zinc-500">{label}</span>
      <span className={`text-[11px] font-semibold ${valueColor ?? "text-zinc-300"}`}>
        {value}
      </span>
    </div>
  );
}

function ObjectHealthRow({ object: obj }: { object: StateObject }) {
  const { refreshObjects } = useWallet();
  const [refreshing, setRefreshing] = useState(false);

  const pct = energyPercent(obj.current_energy, obj.max_energy);
  const isGhost = obj.state === "Ghost";

  const rowBg = isGhost
    ? "bg-evap-ghost/5 border-evap-ghost/20"
    : pct > 50
    ? "bg-evap-green/5 border-evap-green/10"
    : pct > 10
    ? "bg-evap-amber/5 border-evap-amber/10"
    : "bg-evap-red/5 border-evap-red/10";

  const statusIcon = isGhost ? "💀" : pct > 50 ? "●" : pct > 10 ? "●" : "●";
  const statusColor = isGhost
    ? "text-evap-ghost"
    : pct > 50
    ? "text-evap-green"
    : pct > 10
    ? "text-evap-amber"
    : "text-evap-red";

  // Rough epochs remaining estimate based on half-life
  const epochsRemaining = isGhost
    ? 0
    : obj.half_life > 0
    ? Math.max(
        0,
        Math.ceil(
          obj.half_life * Math.log2(Math.max(obj.current_energy, 1))
        )
      )
    : 0;

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      await api.refreshObject(obj.id, Math.ceil(obj.max_energy * 0.5));
      await refreshObjects();
    } catch {
      // Ignore
    }
    setRefreshing(false);
  };

  return (
    <div className={`rounded-lg px-3 py-2.5 border ${rowBg} flex items-center gap-2`}>
      <span className={`text-xs ${statusColor} shrink-0`}>{statusIcon}</span>
      <div className="flex-1 min-w-0">
        <div className="flex items-center justify-between mb-1">
          <span className="text-[11px] font-medium text-zinc-300 truncate">
            {obj.name || obj.id.slice(0, 12)}
          </span>
          <span
            className="text-[10px] font-semibold shrink-0 ml-1"
            style={{ color: energyColor(pct) }}
          >
            {pct}%
          </span>
        </div>
        <div className="flex items-center gap-2">
          <div className="flex-1">
            <EnergyBar
              current={obj.current_energy}
              max={obj.max_energy}
              showLabel={false}
              size="sm"
            />
          </div>
          <span className="text-[9px] text-zinc-600 shrink-0 w-14 text-right">
            {isGhost ? "Ghost" : `${epochsRemaining} epochs`}
          </span>
        </div>
      </div>
      {!isGhost && (
        <button
          onClick={handleRefresh}
          disabled={refreshing}
          className="shrink-0 px-2 py-1 rounded text-[9px] font-medium bg-evap-cyan/10 text-evap-cyan border border-evap-cyan/20 hover:border-evap-cyan/40 transition disabled:opacity-50"
        >
          {refreshing ? "..." : "Refresh"}
        </button>
      )}
    </div>
  );
}

// ── Helpers ──

function generateMockEpochs(
  currentTotal: number,
  count: number
): Array<{ epoch: number; energy: number }> {
  const result: Array<{ epoch: number; energy: number }> = [];
  let energy = currentTotal;
  for (let i = count - 1; i >= 0; i--) {
    result.unshift({
      epoch: i + 1,
      energy: Math.max(0, Math.round(energy)),
    });
    // Simulate decay backwards — each older epoch had ~10% more
    energy = energy * 1.1;
  }
  return result;
}

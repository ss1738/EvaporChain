import { useEffect, useMemo } from "react";
import { RotateCw } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";

/**
 * Refresh-pool dashboard.
 *
 * Surfaces GET /api/refresh_pool — the protocol-owned pool that funds
 * object-refresh payouts. Receives demurrage on idle balances and any
 * patronage payments that aren't routed directly to objects. Per-namespace
 * `RefreshPoolCredit` rows show how much each namespace has accrued and
 * the most recent epoch in which the credit was touched.
 *
 * Field names match crates/evaporchain-node/src/api.rs §RefreshPoolResp
 * (L2823-2834): { total_accrued, credits[{ namespace_hex, accrued,
 * last_touched_epoch }] }.
 */
export function RefreshPoolScreen() {
  const {
    refreshPool,
    chainStatus,
    refreshRefreshPool,
    refreshChainStatus,
    setView,
  } = useWallet();

  useEffect(() => {
    refreshChainStatus();
    refreshRefreshPool();
  }, [refreshChainStatus, refreshRefreshPool]);

  // Most recent epoch update across all credits, for the header line.
  const latestEpoch = useMemo(() => {
    if (!refreshPool || refreshPool.credits.length === 0) return null;
    return refreshPool.credits.reduce(
      (max, c) => (c.last_touched_epoch > max ? c.last_touched_epoch : max),
      0,
    );
  }, [refreshPool]);

  const sortedCredits = useMemo(() => {
    if (!refreshPool) return [];
    return [...refreshPool.credits].sort((a, b) => b.accrued - a.accrued);
  }, [refreshPool]);

  return (
    <div className="flex flex-col h-full bg-evap-bg">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-evap-border">
        <button
          onClick={() => setView("energy-dashboard")}
          className="text-zinc-400 hover:text-zinc-200 transition text-sm"
        >
          ←
        </button>
        <div className="flex-1">
          <h1 className="text-sm font-semibold text-zinc-200">Refresh Pool</h1>
          <p className="text-xs text-zinc-500">
            Where your demurrage and patronage payments accumulate to fund object refreshes.
          </p>
        </div>
        <button
          onClick={() => refreshRefreshPool()}
          className="text-xs px-2 py-1 rounded text-evap-cyan border border-evap-cyan/30 hover:border-evap-cyan/60 transition"
        ><RotateCw className="w-3.5 h-3.5" strokeWidth={1.5} /></button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {/* Total card */}
        <div className="mx-4 mt-3 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <p className="text-xs text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
            Total Pool Balance
          </p>
          <div className="flex items-baseline gap-2">
            <span className="text-2xl font-bold text-zinc-100">
              {(refreshPool?.total_accrued ?? 0).toLocaleString()}
            </span>
            <span className="text-xs text-zinc-500">EVAP</span>
          </div>
          <div className="grid grid-cols-2 gap-2 mt-3">
            <Stat
              label="Namespaces"
              value={refreshPool ? refreshPool.credits.length.toString() : "—"}
            />
            <Stat
              label="Last update"
              value={latestEpoch != null ? `Epoch ${latestEpoch}` : "—"}
            />
          </div>
          {chainStatus && (
            <p className="text-[9px] text-zinc-600 mt-2">
              Current epoch: {chainStatus.epoch.toLocaleString()}
            </p>
          )}
        </div>

        {/* Per-namespace table */}
        <div className="mx-4 mt-4 mb-4">
          <p className="text-xs text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
            Per-namespace credits
          </p>
          {!refreshPool ? (
            <p className="text-xs text-zinc-600 text-center py-8">
              Loading pool credits…
            </p>
          ) : sortedCredits.length === 0 ? (
            <p className="text-xs text-zinc-600 text-center py-8">
              No credits accrued yet.
            </p>
          ) : (
            <div className="rounded-lg border border-evap-border overflow-hidden">
              <div className="grid grid-cols-12 gap-2 px-3 py-2 bg-evap-surface border-b border-evap-border">
                <span className="col-span-6 text-[9px] uppercase tracking-wider text-zinc-500">
                  Namespace
                </span>
                <span className="col-span-3 text-[9px] uppercase tracking-wider text-zinc-500 text-right">
                  Accrued
                </span>
                <span className="col-span-3 text-[9px] uppercase tracking-wider text-zinc-500 text-right">
                  Last epoch
                </span>
              </div>
              <div className="divide-y divide-evap-border">
                {sortedCredits.map((c) => (
                  <div
                    key={c.namespace_hex}
                    className="grid grid-cols-12 gap-2 px-3 py-2 bg-evap-bg hover:bg-evap-surface/60 transition"
                  >
                    <span
                      className="col-span-6 text-xs font-mono text-zinc-300 truncate"
                      title={c.namespace_hex}
                    >
                      {c.namespace_hex.slice(0, 16)}…
                    </span>
                    <span className="col-span-3 text-xs font-semibold text-zinc-200 text-right tabular-nums">
                      {c.accrued.toLocaleString()}
                    </span>
                    <span className="col-span-3 text-xs text-zinc-500 text-right tabular-nums">
                      {c.last_touched_epoch.toLocaleString()}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>

        <div className="mx-4 mb-4 px-3 py-2 rounded-lg bg-evap-cyan/5 border border-evap-cyan/20">
          <p className="text-[9px] text-zinc-400 leading-snug">
            The refresh pool is funded by two flows: (1) demurrage levied on idle balances
            piecewise-log above the threshold, and (2) excess patronage donations that
            outlast their target object's lifetime. Object refreshes draw from this pool
            at the protocol level — you never pay refresh fees twice.
          </p>
        </div>
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="text-[9px] text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-sm font-semibold text-zinc-200 mt-0.5 tabular-nums">{value}</p>
    </div>
  );
}

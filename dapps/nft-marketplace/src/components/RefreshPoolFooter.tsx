import { useEffect, useState } from "react";
import { api } from "@/utils/api";
import type { RefreshPoolStatus } from "@/utils/types";

// Footer card showing the protocol-owned refresh pool's current accrued
// total + the top namespace credit. Visitors see what funds NFT survival
// across the chain.
export function RefreshPoolFooter() {
  const [status, setStatus] = useState<RefreshPoolStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    const fetchStatus = () => {
      api
        .getRefreshPool()
        .then((res) => {
          if (!cancelled) setStatus(res);
        })
        .catch(() => {
          /* swallow — pool widget is informational */
        });
    };
    fetchStatus();
    const t = setInterval(fetchStatus, 15_000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, []);

  if (!status) return null;

  const topCredit = status.credits.length
    ? [...status.credits].sort((a, b) => b.accrued - a.accrued)[0]
    : null;

  return (
    <div className="mt-6 rounded-xl border border-evap-border bg-white p-4">
      <div className="flex items-center justify-between">
        <div>
          <p className="text-xs font-semibold text-zinc-700">Protocol Refresh Pool</p>
          <p className="text-[10px] text-zinc-400">
            EVAP held by the chain to fund object refreshes — fed by demurrage and
            patronage payments.
          </p>
        </div>
        <p className="text-lg font-bold text-evap-cyan">
          {status.total_accrued.toLocaleString()} EVAP
        </p>
      </div>
      {topCredit && (
        <p className="mt-2 text-[10px] text-zinc-400">
          Top namespace:{" "}
          <span className="font-mono">{topCredit.namespace_hex.slice(0, 12)}…</span> ·{" "}
          {topCredit.accrued.toLocaleString()} EVAP · last touched epoch{" "}
          {topCredit.last_touched_epoch}
        </p>
      )}
      <p className="mt-1 text-[10px] text-zinc-400">
        {status.credits.length} namespaces · funds your NFTs&rsquo; survival.
      </p>
    </div>
  );
}

"use client";

import { useEffect, useState } from "react";
import {
  analyticsApi,
  pollWithInterval,
  type FinalityGap,
  type NetworkHealth,
} from "@/lib/api";
import { StatTile } from "@/components/StatTile";
import { FinalityGapChart } from "@/components/FinalityGapChart";
import { UnfinalisedTail } from "@/components/UnfinalisedTail";

export default function FinalityPage() {
  const [data, setData] = useState<{ gap: FinalityGap; health: NetworkHealth } | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    return pollWithInterval(
      async () => {
        const [gap, health] = await Promise.all([analyticsApi.finalityGap(), analyticsApi.networkHealth()]);
        return { gap, health };
      },
      (v) => {
        setData(v);
        setErr(null);
      },
      (e) => setErr(e.message),
      15_000,
    );
  }, []);

  if (err && !data) return <p className="py-20 text-center text-sm text-evap-red">{err}</p>;
  if (!data) return <p className="py-20 text-center text-sm text-zinc-500">Loading…</p>;
  const { gap, health } = data;

  const samples = gap.recent_gaps;
  const meanGap = samples.length > 0 ? samples.reduce((s, g) => s + g.gap_seconds, 0) / samples.length : 0;

  return (
    <div className="space-y-5">
      <header>
        <h1 className="text-xl font-bold text-zinc-900">Finality Health</h1>
        <p className="mt-1 text-xs text-zinc-500">
          Per-height commit→finalise gaps from the consensus engine's ring buffer (see{" "}
          <span className="font-mono">/api/finality/gap</span>).
        </p>
      </header>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <StatTile label="Block height" value={health.block_height.toLocaleString()} />
        <StatTile label="Finalised height" value={health.finalised_height.toLocaleString()} hint={`lag ${health.finality_lag_blocks} blocks`} />
        <StatTile
          label="Worst pending gap"
          value={`${gap.worst_gap_seconds.toFixed(2)}s`}
          tone={gap.worst_gap_seconds > 30 ? "text-evap-red" : gap.worst_gap_seconds > 5 ? "text-evap-amber" : "text-evap-green"}
          hint={gap.worst_gap_seconds > 30 ? "alert: EvapFinalityStalled" : "ok"}
        />
        <StatTile label="Recent mean gap" value={`${meanGap.toFixed(2)}s`} hint={`${samples.length} samples`} />
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <FinalityGapChart gaps={gap} />
        <UnfinalisedTail gaps={gap} />
      </div>
    </div>
  );
}

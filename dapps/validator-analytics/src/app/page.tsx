"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { Activity, AlertTriangle, Boxes, Coins, Network, ShieldX } from "lucide-react";
import {
  analyticsApi,
  fmtEvap,
  pollWithInterval,
  shortAddr,
  uptimeProxy,
  type FinalityGap,
  type NetworkHealth,
  type ValidatorRow,
  type ValidatorsResponse,
} from "@/lib/api";
import { StatTile } from "@/components/StatTile";

interface Snapshot {
  vs: ValidatorsResponse;
  health: NetworkHealth;
  gap: FinalityGap;
}

export default function OverviewPage() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    return pollWithInterval(
      async () => {
        const [vs, health, gap] = await Promise.all([
          analyticsApi.validators(),
          analyticsApi.networkHealth(),
          analyticsApi.finalityGap(),
        ]);
        return { vs, health, gap };
      },
      (v) => {
        setSnap(v);
        setError(null);
      },
      (e) => setError(e.message),
      15_000,
    );
  }, []);

  if (error && !snap) {
    return <ErrorBanner message={error} />;
  }
  if (!snap) {
    return <p className="py-20 text-center text-sm text-zinc-500">Loading…</p>;
  }

  const { vs, health, gap } = snap;
  const uptime = uptimeProxy(vs.validators);
  const byStake = [...vs.validators].sort((a, b) => b.effective_stake - a.effective_stake).slice(0, 10);
  const byBlocks = [...vs.validators].sort((a, b) => b.blocks_produced - a.blocks_produced).slice(0, 10);
  const byUptime = [...vs.validators]
    .sort((a, b) => (uptime.get(b.id) ?? 0) - (uptime.get(a.id) ?? 0))
    .slice(0, 10);
  const slashed = vs.validators
    .filter((v) => v.total_slashed > 0)
    .sort((a, b) => b.total_slashed - a.total_slashed)
    .slice(0, 10);

  return (
    <div className="space-y-6">
      <header>
        <h1 className="text-xl font-bold text-zinc-900">Network Overview</h1>
        <p className="mt-1 text-xs text-zinc-500">
          Aggregate validator state, finality status, and the leaderboards an oncall would scan first.
        </p>
      </header>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <StatTile
          label="Block height"
          value={health.block_height.toLocaleString()}
          hint={`${health.status} · ${health.last_block_age_secs ?? "?"}s old`}
          tone={health.status === "healthy" ? "text-evap-green" : "text-evap-amber"}
          icon={<Activity size={14} className="text-zinc-400" />}
        />
        <StatTile
          label="Validators"
          value={`${vs.count - vs.jailed_count} / ${vs.count}`}
          hint={`${vs.bls_registered_count} BLS · ${vs.jailed_count} jailed`}
          icon={<Boxes size={14} className="text-zinc-400" />}
        />
        <StatTile
          label="Total stake"
          value={fmtEvap(vs.total_stake)}
          hint={`Effective: ${fmtEvap(vs.total_effective_stake)}`}
          icon={<Coins size={14} className="text-zinc-400" />}
        />
        <StatTile
          label="Finality lag"
          value={`${health.finality_lag_blocks}`}
          hint={`Worst gap: ${gap.worst_gap_seconds.toFixed(1)}s`}
          tone={gap.worst_gap_seconds > 30 ? "text-evap-red" : "text-zinc-900"}
          icon={<AlertTriangle size={14} className="text-zinc-400" />}
        />
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Leaderboard title="Top 10 by stake" icon={<Coins size={14} />} rows={byStake} extract={(v) => fmtEvap(v.effective_stake)} />
        <Leaderboard title="Top 10 by blocks produced" icon={<Network size={14} />} rows={byBlocks} extract={(v) => v.blocks_produced.toLocaleString()} />
        <Leaderboard title="Top 10 by uptime proxy" icon={<Activity size={14} />} rows={byUptime} extract={(v) => `${((uptime.get(v.id) ?? 0) * 100).toFixed(1)}%`} />
        <Leaderboard
          title="Slash leaderboard"
          icon={<ShieldX size={14} />}
          rows={slashed}
          extract={(v) => fmtEvap(v.total_slashed)}
          empty="No slashes recorded."
        />
      </div>
    </div>
  );
}

function Leaderboard({
  title,
  icon,
  rows,
  extract,
  empty,
}: {
  title: string;
  icon?: React.ReactNode;
  rows: ValidatorRow[];
  extract: (v: ValidatorRow) => string;
  empty?: string;
}) {
  return (
    <div className="rounded-xl border border-evap-border bg-white p-4">
      <div className="flex items-center gap-2">
        {icon}
        <p className="text-xs font-semibold text-zinc-900">{title}</p>
      </div>
      {rows.length === 0 ? (
        <p className="mt-3 text-xs text-evap-green">{empty ?? "—"}</p>
      ) : (
        <table className="mt-3 w-full text-[11px]">
          <tbody>
            {rows.map((v) => (
              <tr key={v.id} className="border-t border-evap-border/60">
                <td className="py-1.5 font-mono text-zinc-500">#{v.id}</td>
                <td className="py-1.5 font-mono text-zinc-700">{shortAddr(v.address)}</td>
                <td className="py-1.5 text-right text-zinc-900">{extract(v)}</td>
                <td className="py-1.5 text-right">
                  <Link href={`/validators/${v.id}`} className="text-evap-cyan hover:underline">
                    →
                  </Link>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function ErrorBanner({ message }: { message: string }) {
  return (
    <div className="rounded-xl border border-evap-red/30 bg-red-50 p-4">
      <p className="text-sm font-semibold text-evap-red">Failed to reach node</p>
      <p className="mt-1 font-mono text-[11px] text-evap-red/80">{message}</p>
      <p className="mt-2 text-[11px] text-zinc-500">
        Check that <span className="font-mono">EVAPORCHAIN_RPC</span> points at a reachable node.
      </p>
    </div>
  );
}

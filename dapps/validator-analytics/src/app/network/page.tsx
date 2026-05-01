"use client";

import { useEffect, useState } from "react";
import {
  analyticsApi,
  pollWithInterval,
  type NetworkHealth,
  type PeersResponse,
} from "@/lib/api";
import { StatTile } from "@/components/StatTile";
import { PeerScoreBadge } from "@/components/PeerScoreBadge";

export default function NetworkPage() {
  const [data, setData] = useState<{
    health: NetworkHealth;
    peers: PeersResponse;
    rejections: Record<string, number>;
    activeBans: number;
  } | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    return pollWithInterval(
      async () => {
        const [health, peers, metrics] = await Promise.all([
          analyticsApi.networkHealth(),
          analyticsApi.networkPeers(),
          analyticsApi.metrics(),
        ]);
        return {
          health,
          peers,
          rejections: metrics.rejections,
          activeBans: metrics.activeBans,
        };
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
  const { health, peers, rejections, activeBans } = data;

  const subnetCount = new Map<string, number>();
  for (const p of peers.peers) {
    const k = p.subnet ?? "unknown";
    subnetCount.set(k, (subnetCount.get(k) ?? 0) + 1);
  }
  const totalRej = Object.values(rejections).reduce((s, v) => s + v, 0);

  return (
    <div className="space-y-5">
      <header>
        <h1 className="text-xl font-bold text-zinc-900">Network Health</h1>
        <p className="mt-1 text-xs text-zinc-500">
          Live peer set, subnet distribution and Sybil-resistance counters
          (<span className="font-mono">/api/network/peers</span> + <span className="font-mono">/metrics</span>).
        </p>
      </header>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <StatTile label="Peers" value={peers.count} hint={`${health.status}`} />
        <StatTile label="Active bans" value={activeBans} hint="evap_active_bans" />
        <StatTile label="Subnets" value={subnetCount.size} hint="distinct /16 buckets" />
        <StatTile label="Inbound rejections" value={totalRej} hint="cumulative" />
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <div className="rounded-xl border border-evap-border bg-white p-4">
          <p className="text-xs font-semibold text-zinc-900">Subnet distribution</p>
          {subnetCount.size === 0 ? (
            <p className="mt-2 text-[11px] text-zinc-500">No peers connected.</p>
          ) : (
            <table className="mt-3 w-full text-[11px]">
              <thead>
                <tr className="text-left text-[9px] uppercase text-zinc-400">
                  <th className="py-1">Subnet</th>
                  <th className="py-1 text-right">Peers</th>
                </tr>
              </thead>
              <tbody>
                {[...subnetCount.entries()]
                  .sort((a, b) => b[1] - a[1])
                  .map(([s, c]) => (
                    <tr key={s} className="border-t border-evap-border/60">
                      <td className="py-1 font-mono">{s}</td>
                      <td className="py-1 text-right">{c}</td>
                    </tr>
                  ))}
              </tbody>
            </table>
          )}
        </div>

        <div className="rounded-xl border border-evap-border bg-white p-4">
          <p className="text-xs font-semibold text-zinc-900">Inbound rejections by reason</p>
          {Object.keys(rejections).length === 0 ? (
            <p className="mt-2 text-[11px] text-zinc-500">No rejection counters available (metrics may be admin-gated).</p>
          ) : (
            <table className="mt-3 w-full text-[11px]">
              <tbody>
                {Object.entries(rejections).map(([k, v]) => (
                  <tr key={k} className="border-t border-evap-border/60">
                    <td className="py-1 font-mono text-zinc-700">{k}</td>
                    <td className="py-1 text-right font-mono">{v}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>

      <div className="rounded-xl border border-evap-border bg-white p-4">
        <p className="text-xs font-semibold text-zinc-900">Connected peers</p>
        {peers.peers.length === 0 ? (
          <p className="mt-2 text-[11px] text-zinc-500">No peers connected.</p>
        ) : (
          <div className="mt-3 max-h-96 overflow-y-auto">
            <table className="w-full text-[11px]">
              <thead className="bg-zinc-50">
                <tr className="text-left text-[9px] uppercase text-zinc-400">
                  <th className="px-2 py-1">peer_id</th>
                  <th className="px-2 py-1">ip</th>
                  <th className="px-2 py-1">subnet</th>
                  <th className="px-2 py-1 text-right">score</th>
                  <th className="px-2 py-1 text-right">age</th>
                </tr>
              </thead>
              <tbody>
                {peers.peers.map((p) => (
                  <tr key={p.peer_id} className="border-t border-evap-border/60">
                    <td className="px-2 py-1 font-mono">{p.peer_id.slice(0, 16)}…</td>
                    <td className="px-2 py-1 font-mono">{p.ip ?? "—"}</td>
                    <td className="px-2 py-1 font-mono">{p.subnet ?? "—"}</td>
                    <td className="px-2 py-1 text-right"><PeerScoreBadge score={p.score} /></td>
                    <td className="px-2 py-1 text-right font-mono">{p.age_seconds}s</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}

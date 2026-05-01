"use client";

import { use, useEffect, useState } from "react";
import Link from "next/link";
import {
  analyticsApi,
  fmtEvap,
  pollWithInterval,
  shortAddr,
  type PeersResponse,
  type ValidatorDelegationsResponse,
  type ValidatorRow,
} from "@/lib/api";
import { StatTile } from "@/components/StatTile";
import { Sparkline } from "@/components/Sparkline";
import { DelegationFlow } from "@/components/DelegationFlow";
import { SlashTimeline } from "@/components/SlashTimeline";
import { PeerScoreBadge } from "@/components/PeerScoreBadge";

interface DetailSnapshot {
  v: ValidatorRow | null;
  delegations: ValidatorDelegationsResponse;
  peers: PeersResponse;
  blocks: number[];
  meanSecs: number;
}

export default function ValidatorDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id: idStr } = use(params);
  const id = Number(idStr);
  const [snap, setSnap] = useState<DetailSnapshot | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (!Number.isFinite(id)) {
      setErr(`Invalid validator id: ${idStr}`);
      return;
    }
    return pollWithInterval(
      async () => {
        const [vs, delegations, peers, metrics] = await Promise.all([
          analyticsApi.validators(),
          analyticsApi.validatorDelegations(id),
          analyticsApi.networkPeers(),
          analyticsApi.metrics(),
        ]);
        const v = vs.validators.find((x) => x.id === id) ?? null;
        const hist = metrics.perValidator.find((p) => p.producer === `validator-${id}`);
        return {
          v,
          delegations,
          peers,
          blocks: hist ? hist.buckets.map((b) => b.cum) : [],
          meanSecs: hist?.meanSecs ?? 0,
        };
      },
      (s) => {
        setSnap(s);
        setErr(null);
      },
      (e) => setErr(e.message),
      15_000,
    );
  }, [id, idStr]);

  if (err && !snap) {
    return <p className="py-20 text-center text-sm text-evap-red">{err}</p>;
  }
  if (!snap) {
    return <p className="py-20 text-center text-sm text-zinc-500">Loading…</p>;
  }
  const { v, delegations, peers, blocks, meanSecs } = snap;
  if (!v) {
    return (
      <div className="py-20 text-center">
        <p className="text-sm text-zinc-700">Validator #{id} not found.</p>
        <Link href="/validators" className="mt-2 inline-block text-xs text-evap-cyan hover:underline">
          ← back to all validators
        </Link>
      </div>
    );
  }

  // Best-effort subnet via address-prefix → peer match. PeerInfo has no
  // validator_id field, so this is heuristic only and may be empty.
  const peerHint = peers.peers.find((p) =>
    v.address.toLowerCase().includes((p.peer_id || "").toLowerCase().slice(0, 6)),
  );

  return (
    <div className="space-y-5">
      <header>
        <Link href="/validators" className="text-[11px] text-evap-cyan hover:underline">
          ← all validators
        </Link>
        <h1 className="mt-1 text-xl font-bold text-zinc-900">
          Validator #{v.id}{" "}
          <span className="text-xs font-mono text-zinc-500">{shortAddr(v.address)}</span>
        </h1>
        <p className="mt-1 text-xs text-zinc-500">
          {v.jailed ? "Jailed" : v.bls_registered ? "Active" : "BLS not registered"} · health {v.health_score.toFixed(2)}
        </p>
      </header>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <StatTile label="Stake" value={fmtEvap(v.stake)} />
        <StatTile label="Effective stake" value={fmtEvap(v.effective_stake)} hint="own + delegated" />
        <StatTile
          label="Delegated"
          value={fmtEvap(delegations.total_delegated)}
          hint={`${delegations.delegation_count} delegators`}
        />
        <StatTile
          label="Blocks produced"
          value={v.blocks_produced.toLocaleString()}
          hint={meanSecs > 0 ? `mean ${meanSecs.toFixed(3)}s` : "no recent samples"}
        />
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <div className="rounded-xl border border-evap-border bg-white p-4">
          <p className="text-xs font-semibold text-zinc-900">Block-production timeline</p>
          <p className="mt-1 font-mono text-[10px] text-zinc-400">
            evap_block_production_seconds_bucket · producer=&quot;validator-{id}&quot;.
            Cumulative bucket-count series; flat right-tail = healthy.
          </p>
          <div className="mt-3 flex items-center gap-3">
            <Sparkline values={blocks} width={300} height={48} />
            <span className="font-mono text-[11px] text-zinc-500">
              {blocks.length} buckets
            </span>
          </div>
          <p className="mt-2 text-[10px] text-zinc-400">
            Bell-Beacon S-value: <span className="font-mono">—</span>{" "}
            (chain exposes ad-hoc POST /api/bell_beacon, no per-validator gauge).
          </p>
        </div>

        <div className="rounded-xl border border-evap-border bg-white p-4">
          <p className="text-xs font-semibold text-zinc-900">Stake breakdown</p>
          <div className="mt-3 space-y-1">
            <Bar label="Own stake" value={Math.max(0, v.stake)} max={v.effective_stake || v.stake} tone="bg-evap-cyan" />
            <Bar label="Delegated" value={delegations.total_delegated} max={v.effective_stake || v.stake} tone="bg-evap-purple" />
          </div>
          <p className="mt-3 text-[10px] text-zinc-400">
            Hot/cold split is gated behind POST /api/hot_cold_stake/* and not exposed as a gauge.
          </p>
        </div>

        <DelegationFlow rows={delegations.delegations} />
        <SlashTimeline totalSlashed={v.total_slashed} />

        <div className="rounded-xl border border-evap-border bg-white p-4 lg:col-span-2">
          <p className="text-xs font-semibold text-zinc-900">Peer / network position</p>
          {peerHint ? (
            <table className="mt-2 w-full text-[11px]">
              <tbody>
                <tr><td className="py-1 text-zinc-500">peer_id</td><td className="py-1 font-mono">{peerHint.peer_id}</td></tr>
                <tr><td className="py-1 text-zinc-500">subnet</td><td className="py-1 font-mono">{peerHint.subnet ?? "—"}</td></tr>
                <tr><td className="py-1 text-zinc-500">ip</td><td className="py-1 font-mono">{peerHint.ip ?? "—"}</td></tr>
                <tr><td className="py-1 text-zinc-500">score</td><td className="py-1"><PeerScoreBadge score={peerHint.score} /></td></tr>
                <tr><td className="py-1 text-zinc-500">age</td><td className="py-1 font-mono">{peerHint.age_seconds}s</td></tr>
              </tbody>
            </table>
          ) : (
            <p className="mt-2 text-[11px] text-zinc-500">
              No matching peer view (validator-id ↔ peer-id mapping is not exposed). See the{" "}
              <Link href="/network" className="text-evap-cyan hover:underline">network page</Link>
              {" "}for the full peer list.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

function Bar({ label, value, max, tone }: { label: string; value: number; max: number; tone: string }) {
  const pct = max > 0 ? (value / max) * 100 : 0;
  return (
    <div>
      <div className="flex items-baseline justify-between">
        <span className="text-[11px] text-zinc-500">{label}</span>
        <span className="font-mono text-[11px] text-zinc-700">{fmtEvap(value)}</span>
      </div>
      <div className="mt-1 h-2 w-full rounded-full bg-zinc-100">
        <div className={`h-full rounded-full ${tone}`} style={{ width: `${Math.min(100, pct)}%` }} />
      </div>
    </div>
  );
}

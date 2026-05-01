"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { Plus } from "lucide-react";
import { ProposalCard } from "@/components/ProposalCard";
import { ChainStatus } from "@/components/ChainStatus";
import type { Proposal, ProposalState } from "@/lib/types";

const COLUMNS: Array<{
  state: ProposalState;
  title: string;
  blurb: string;
  accent: string;
}> = [
  {
    state: "open",
    title: "Open",
    blurb: "Drafted. Accepting validator endorsements.",
    accent: "text-evap-violet",
  },
  {
    state: "active",
    title: "Active",
    blurb: "Quorum met. Submitted on-chain.",
    accent: "text-evap-emerald",
  },
  {
    state: "historical",
    title: "Historical",
    blurb: "Rejected, superseded, or withdrawn.",
    accent: "text-zinc-500",
  },
];

export default function ProposalListPage() {
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const res = await fetch("/api/proposals");
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const j = (await res.json()) as { proposals: Proposal[] };
        if (cancelled) return;
        setProposals(j.proposals);
        setError(null);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : "load failed");
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    load();
    const t = setInterval(load, 8_000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, []);

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-xl font-bold text-zinc-900">Governance Proposals</h1>
          <p className="mt-1 text-xs text-zinc-500 max-w-2xl">
            Stake-quorum amendments to the EvaporChain L1. Anyone can browse;
            active validators endorse; once endorsed stake clears the required
            quorum, anyone can broadcast the amendment to the chain.
          </p>
        </div>
        <Link
          href="/proposals/new"
          className="flex shrink-0 items-center gap-1.5 rounded-lg bg-gradient-to-r from-evap-violet to-evap-cyan px-4 py-2 text-xs font-semibold text-white hover:opacity-90"
        >
          <Plus className="h-3.5 w-3.5" />
          Draft proposal
        </Link>
      </div>

      <ChainStatus />

      {error && (
        <div className="rounded-lg border border-evap-red/30 bg-red-50 px-3 py-2 text-sm text-evap-red">
          {error}
        </div>
      )}

      {loading ? (
        <div className="flex items-center justify-center py-20">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-evap-violet/30 border-t-evap-violet" />
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
          {COLUMNS.map((col) => {
            const items = proposals.filter((p) => p.state === col.state);
            return (
              <section key={col.state}>
                <header className="mb-3">
                  <div className="flex items-center justify-between">
                    <h2 className={`text-sm font-bold ${col.accent}`}>
                      {col.title}
                    </h2>
                    <span className="rounded-full bg-zinc-100 px-2 py-0.5 text-[10px] font-mono tabular-nums text-zinc-600">
                      {items.length}
                    </span>
                  </div>
                  <p className="mt-0.5 text-[10px] text-zinc-400">{col.blurb}</p>
                </header>
                {items.length === 0 ? (
                  <div className="rounded-xl border border-dashed border-evap-border bg-white py-10 text-center">
                    <p className="text-[11px] text-zinc-400">No proposals.</p>
                  </div>
                ) : (
                  <div className="space-y-3">
                    {items.map((p) => (
                      <ProposalCard key={p.id} proposal={p} />
                    ))}
                  </div>
                )}
              </section>
            );
          })}
        </div>
      )}
    </div>
  );
}

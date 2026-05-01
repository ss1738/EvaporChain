"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import { ArrowLeft, GitBranch, Cpu } from "lucide-react";
import { EndorsementProgress } from "@/components/EndorsementProgress";
import { EndorseButton } from "@/components/EndorseButton";
import { SubmitButton } from "@/components/SubmitButton";
import type {
  ForkChoicePayload,
  Proposal,
  UpgradeContractPayload,
} from "@/lib/types";

export default function ProposalDetailPage() {
  const params = useParams<{ id: string }>();
  const id = decodeURIComponent(params.id);
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      const res = await fetch(`/api/proposals/${encodeURIComponent(id)}`);
      if (!res.ok) {
        if (res.status === 404) throw new Error("Proposal not found");
        throw new Error(`HTTP ${res.status}`);
      }
      const j = (await res.json()) as { proposal: Proposal };
      setProposal(j.proposal);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "load failed");
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    load();
    const t = setInterval(load, 8_000);
    return () => clearInterval(t);
  }, [load]);

  const endorsedStake = useMemo(
    () => (proposal ? proposal.endorsements.reduce((s, e) => s + e.stake, 0) : 0),
    [proposal],
  );

  if (loading) {
    return (
      <div className="flex items-center justify-center py-20">
        <div className="h-8 w-8 animate-spin rounded-full border-2 border-evap-violet/30 border-t-evap-violet" />
      </div>
    );
  }

  if (error || !proposal) {
    return (
      <div className="space-y-3">
        <Link
          href="/"
          className="inline-flex items-center gap-1 text-xs text-zinc-500 hover:text-zinc-900"
        >
          <ArrowLeft className="h-3 w-3" /> Back
        </Link>
        <div className="rounded-lg border border-evap-red/30 bg-red-50 px-3 py-2 text-sm text-evap-red">
          {error ?? "missing proposal"}
        </div>
      </div>
    );
  }

  const Icon = proposal.type === "fork_choice" ? GitBranch : Cpu;

  return (
    <div className="space-y-6">
      <Link
        href="/"
        className="inline-flex items-center gap-1 text-xs text-zinc-500 hover:text-zinc-900"
      >
        <ArrowLeft className="h-3 w-3" /> All proposals
      </Link>

      <header className="rounded-xl border border-evap-border bg-white p-5">
        <div className="flex items-start gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-violet-50 text-evap-violet">
            <Icon className="h-5 w-5" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[10px] font-medium uppercase tracking-wider text-zinc-500">
              {proposal.type === "fork_choice" ? "Fork-choice mode" : "Contract upgrade"}
            </p>
            <h1 className="mt-0.5 text-lg font-bold text-zinc-900">{proposal.title}</h1>
            <p className="mt-0.5 text-[10px] font-mono text-zinc-400">
              #{proposal.id}
            </p>
          </div>
          <span
            className={`rounded-full border px-2 py-0.5 text-[10px] font-medium uppercase tracking-wider ${
              proposal.state === "open"
                ? "border-evap-violet/30 bg-violet-50 text-evap-violet"
                : proposal.state === "active"
                  ? "border-evap-emerald/30 bg-emerald-50 text-evap-emerald"
                  : "border-zinc-200 bg-zinc-100 text-zinc-500"
            }`}
          >
            {proposal.state}
          </span>
        </div>

        {proposal.rationale && (
          <div className="mt-4 whitespace-pre-wrap rounded-lg bg-zinc-50 px-3 py-3 text-xs text-zinc-700">
            {proposal.rationale}
          </div>
        )}
      </header>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2 space-y-6">
          <section className="rounded-xl border border-evap-border bg-white p-5">
            <p className="mb-3 text-[10px] font-semibold uppercase tracking-wider text-zinc-500">
              Stake quorum
            </p>
            <EndorsementProgress
              endorsed={endorsedStake}
              required={proposal.required_stake}
            />
          </section>

          <section className="rounded-xl border border-evap-border bg-white p-5">
            <p className="mb-3 text-[10px] font-semibold uppercase tracking-wider text-zinc-500">
              Payload
            </p>
            <PayloadView proposal={proposal} />
          </section>

          <section className="rounded-xl border border-evap-border bg-white p-5">
            <p className="mb-3 text-[10px] font-semibold uppercase tracking-wider text-zinc-500">
              Endorsers ({proposal.endorsements.length})
            </p>
            {proposal.endorsements.length === 0 ? (
              <p className="text-[11px] text-zinc-400">
                No endorsements yet. Validators with active stake can pile on
                until the quorum bar is hit.
              </p>
            ) : (
              <div className="space-y-2">
                {proposal.endorsements.map((e) => (
                  <div
                    key={e.address}
                    className="flex items-center justify-between rounded-lg border border-evap-border bg-zinc-50 px-3 py-2"
                  >
                    <div className="min-w-0 flex-1">
                      <p className="truncate font-mono text-[11px] text-zinc-900">
                        {e.address}
                      </p>
                      <p className="text-[10px] text-zinc-500">
                        {new Date(e.signed_at).toLocaleString()}
                        {e.signature_hex ? " · signed" : " · paste-mode"}
                      </p>
                    </div>
                    <span className="ml-3 shrink-0 font-mono text-xs font-semibold text-zinc-900 tabular-nums">
                      {e.stake.toLocaleString()}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </section>
        </div>

        <div className="space-y-4">
          {proposal.state === "open" && (
            <EndorseButton proposal={proposal} onEndorsed={load} />
          )}
          <div className="rounded-xl border border-evap-border bg-white p-4">
            <p className="mb-3 text-[10px] font-semibold uppercase tracking-wider text-zinc-500">
              Broadcast
            </p>
            <SubmitButton
              proposal={proposal}
              endorsedStake={endorsedStake}
              onSubmitted={load}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

function PayloadView({ proposal }: { proposal: Proposal }) {
  if (proposal.type === "fork_choice") {
    const p = proposal.payload as ForkChoicePayload;
    return (
      <div className="space-y-2 text-xs">
        <Row label="Mode" value={p.mode} />
        <Row label="Attractors" value={String(p.attractors.length)} />
        {p.attractors.length > 0 && (
          <pre className="mt-2 overflow-x-auto rounded-lg bg-zinc-50 p-3 font-mono text-[10px] text-zinc-700">
            {JSON.stringify(p.attractors, null, 2)}
          </pre>
        )}
      </div>
    );
  }
  const p = proposal.payload as UpgradeContractPayload;
  return (
    <div className="space-y-2 text-xs">
      <Row label="Owner" value={p.owner} mono />
      <Row label="Contract id" value={String(p.contract_id)} />
      <Row label="Nonce" value={String(p.nonce)} />
      <Row label="Bytecode hash" value={p.new_bytecode_hash_hex} mono />
      <Row label="Bytecode length" value={`${p.new_bytecode_hex.length / 2} bytes`} />
    </div>
  );
}

function Row({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="flex items-center justify-between gap-3 border-b border-zinc-100 py-1.5 last:border-0">
      <span className="text-[10px] uppercase tracking-wider text-zinc-500">
        {label}
      </span>
      <span
        className={`max-w-[60%] truncate text-right text-xs text-zinc-900 ${mono ? "font-mono" : ""}`}
      >
        {value}
      </span>
    </div>
  );
}

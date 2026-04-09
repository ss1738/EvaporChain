import { useState, useEffect } from "react";
import { getProposal, getVotes } from "@/utils/api";
import type { Proposal, Vote } from "@/utils/types";

function timeRemaining(expiry: number): string {
  const diff = expiry - Date.now();
  if (diff <= 0) return "Expired";
  const d = Math.floor(diff / 86_400_000);
  const h = Math.floor((diff % 86_400_000) / 3_600_000);
  const m = Math.floor((diff % 3_600_000) / 60_000);
  if (d > 0) return `${d}d ${h}h ${m}m`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

interface Props {
  proposalId: string;
  onBack: () => void;
  onVote: (p: Proposal) => void;
  onBoost: (p: Proposal) => void;
}

export function ProposalDetail({ proposalId, onBack, onVote, onBoost }: Props) {
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [votes, setVotes] = useState<Vote[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    Promise.allSettled([getProposal(proposalId), getVotes(proposalId)])
      .then(([pRes, vRes]) => {
        if (pRes.status === "fulfilled") setProposal(pRes.value);
        if (vRes.status === "fulfilled") setVotes(Array.isArray(vRes.value) ? vRes.value : []);
      })
      .finally(() => setLoading(false));
  }, [proposalId]);

  if (loading) {
    return (
      <div className="flex items-center justify-center py-20">
        <div className="w-6 h-6 border-2 border-evap-cyan/30 border-t-evap-cyan rounded-full animate-spin" />
      </div>
    );
  }

  if (!proposal) {
    return (
      <div className="text-center py-20">
        <p className="text-sm text-zinc-400">Proposal not found</p>
        <button onClick={onBack} className="text-sm text-evap-cyan mt-2 hover:underline">
          Back
        </button>
      </div>
    );
  }

  const p = proposal;
  const totalVotes = p.votes_for + p.votes_against;
  const forPct = totalVotes > 0 ? (p.votes_for / totalVotes) * 100 : 50;
  const energyPct = p.max_energy > 0 ? (p.current_energy / p.max_energy) * 100 : 0;
  const quorumPct = p.quorum > 0 ? Math.min(100, (totalVotes / p.quorum) * 100) : 0;

  return (
    <div>
      {/* Back button */}
      <button
        onClick={onBack}
        className="flex items-center gap-1.5 text-sm text-zinc-400 hover:text-zinc-600 mb-4 transition-colors"
      >
        ← Back to Proposals
      </button>

      {/* Header */}
      <div className="bg-white rounded-xl border border-evap-border p-6 mb-4">
        <div className="flex items-center gap-2 mb-2">
          <span className={`text-[10px] px-2 py-0.5 rounded-full font-medium ${
            p.status === "active" ? "bg-evap-cyan/10 text-evap-cyan"
            : p.status === "passed" ? "bg-evap-green/10 text-evap-green"
            : p.status === "rejected" ? "bg-evap-red/10 text-evap-red"
            : "bg-zinc-100 text-zinc-400"
          }`}>
            {p.status.charAt(0).toUpperCase() + p.status.slice(1)}
          </span>
          <span className="text-[10px] text-zinc-400">ID: {p.id}</span>
        </div>
        <h1 className="text-xl font-bold text-zinc-900 mb-2">{p.title}</h1>
        <p className="text-sm text-zinc-600 leading-relaxed">{p.description}</p>

        <div className="mt-4 flex items-center gap-4 text-[10px] text-zinc-400">
          <span>
            Proposed by{" "}
            <span className="font-mono text-zinc-600">
              {p.proposer.slice(0, 8)}...{p.proposer.slice(-6)}
            </span>
          </span>
          <span>
            {new Date(p.created_at).toLocaleDateString()}
          </span>
          {p.status === "active" && (
            <span className="text-evap-amber font-medium">
              {timeRemaining(p.estimated_expiry)} remaining
            </span>
          )}
        </div>

        {p.status === "active" && (
          <div className="mt-4 flex items-center gap-2">
            <button
              onClick={() => onVote(p)}
              className="px-4 py-2 rounded-lg bg-evap-cyan text-white text-xs font-medium hover:bg-evap-cyan/90 transition-colors"
            >
              Cast Vote
            </button>
            <button
              onClick={() => onBoost(p)}
              className="px-4 py-2 rounded-lg bg-evap-amber/10 text-evap-amber text-xs font-medium hover:bg-evap-amber/20 transition-colors"
            >
              Boost Energy
            </button>
          </div>
        )}
      </div>

      {/* Stats Grid */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-4">
        <div className="bg-white rounded-xl border border-evap-border p-4">
          <p className="text-[9px] text-zinc-400 uppercase tracking-wider mb-1">Votes For</p>
          <p className="text-lg font-bold text-evap-green">{p.votes_for.toLocaleString()}</p>
        </div>
        <div className="bg-white rounded-xl border border-evap-border p-4">
          <p className="text-[9px] text-zinc-400 uppercase tracking-wider mb-1">Votes Against</p>
          <p className="text-lg font-bold text-evap-red">{p.votes_against.toLocaleString()}</p>
        </div>
        <div className="bg-white rounded-xl border border-evap-border p-4">
          <p className="text-[9px] text-zinc-400 uppercase tracking-wider mb-1">Energy</p>
          <p className="text-lg font-bold text-evap-cyan">
            {p.current_energy.toLocaleString()}
          </p>
          <p className="text-[10px] text-zinc-400">/ {p.max_energy.toLocaleString()}</p>
        </div>
        <div className="bg-white rounded-xl border border-evap-border p-4">
          <p className="text-[9px] text-zinc-400 uppercase tracking-wider mb-1">Half-Life</p>
          <p className="text-lg font-bold text-evap-purple">{p.half_life}</p>
          <p className="text-[10px] text-zinc-400">epochs</p>
        </div>
      </div>

      {/* Vote Distribution */}
      <div className="bg-white rounded-xl border border-evap-border p-5 mb-4">
        <h2 className="text-sm font-semibold text-zinc-900 mb-3">Vote Distribution</h2>
        <div className="space-y-3">
          {/* For/Against bar */}
          <div>
            <div className="flex justify-between text-[10px] mb-1">
              <span className="text-evap-green font-medium">For — {Math.round(forPct)}%</span>
              <span className="text-evap-red font-medium">Against — {Math.round(100 - forPct)}%</span>
            </div>
            <div className="h-3 rounded-full bg-evap-red/20 overflow-hidden">
              <div className="h-full rounded-full bg-evap-green" style={{ width: `${forPct}%` }} />
            </div>
          </div>

          {/* Energy bar */}
          <div>
            <div className="flex justify-between text-[10px] mb-1">
              <span className="text-zinc-500">Energy Remaining</span>
              <span className={`font-medium ${energyPct > 50 ? "text-evap-cyan" : energyPct > 20 ? "text-evap-amber" : "text-evap-red"}`}>
                {Math.round(energyPct)}%
              </span>
            </div>
            <div className="h-2 rounded-full bg-zinc-100 overflow-hidden">
              <div
                className={`h-full rounded-full ${energyPct > 50 ? "bg-evap-cyan" : energyPct > 20 ? "bg-evap-amber" : "bg-evap-red"}`}
                style={{ width: `${energyPct}%` }}
              />
            </div>
          </div>

          {/* Quorum bar */}
          <div>
            <div className="flex justify-between text-[10px] mb-1">
              <span className="text-zinc-500">Quorum Progress</span>
              <span className={`font-medium ${p.quorum_reached ? "text-evap-green" : "text-evap-purple"}`}>
                {p.quorum_reached ? "Reached" : `${Math.round(quorumPct)}%`}
              </span>
            </div>
            <div className="h-2 rounded-full bg-zinc-100 overflow-hidden">
              <div
                className={`h-full rounded-full ${p.quorum_reached ? "bg-evap-green" : "bg-evap-purple"}`}
                style={{ width: `${quorumPct}%` }}
              />
            </div>
          </div>
        </div>

        <p className="text-[10px] text-zinc-400 mt-3">
          Energy decays via E(t) = {p.max_energy.toLocaleString()} × 2^(-t/{p.half_life}).
          Boost energy to keep the proposal alive until quorum is reached.
        </p>
      </div>

      {/* Vote History */}
      <div className="bg-white rounded-xl border border-evap-border overflow-hidden">
        <div className="px-5 py-3 border-b border-evap-border">
          <h2 className="text-sm font-semibold text-zinc-900">
            Vote History ({votes.length})
          </h2>
        </div>
        {votes.length === 0 ? (
          <div className="px-5 py-8 text-center">
            <p className="text-sm text-zinc-400">No votes yet — be the first</p>
          </div>
        ) : (
          <div className="divide-y divide-evap-border">
            {votes.map((v, i) => (
              <div key={i} className="flex items-center justify-between px-5 py-3">
                <div className="flex items-center gap-3">
                  <div className={`w-6 h-6 rounded-full flex items-center justify-center ${
                    v.direction === "for" ? "bg-evap-green/10" : "bg-evap-red/10"
                  }`}>
                    <span className={`text-[10px] font-bold ${
                      v.direction === "for" ? "text-evap-green" : "text-evap-red"
                    }`}>
                      {v.direction === "for" ? "✓" : "✕"}
                    </span>
                  </div>
                  <div>
                    <p className="text-xs font-mono text-zinc-600">
                      {v.voter.slice(0, 8)}...{v.voter.slice(-4)}
                    </p>
                    <p className="text-[10px] text-zinc-400">
                      Weight: {v.weight}
                      {v.energy_boost > 0 && ` · +${v.energy_boost} energy`}
                    </p>
                  </div>
                </div>
                <span className="text-[10px] text-zinc-400">
                  {new Date(v.timestamp).toLocaleString()}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

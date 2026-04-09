import type { Proposal } from "@/utils/types";

function statusBadge(status: string) {
  switch (status) {
    case "active":
      return { bg: "bg-evap-cyan/10", text: "text-evap-cyan", label: "Active" };
    case "passed":
      return { bg: "bg-evap-green/10", text: "text-evap-green", label: "Passed" };
    case "rejected":
      return { bg: "bg-evap-red/10", text: "text-evap-red", label: "Rejected" };
    case "expired":
    case "evaporated":
      return { bg: "bg-zinc-100", text: "text-zinc-400", label: "Evaporated" };
    default:
      return { bg: "bg-zinc-100", text: "text-zinc-400", label: status };
  }
}

function categoryColor(cat: string): string {
  switch (cat) {
    case "parameter": return "bg-evap-purple/10 text-evap-purple";
    case "treasury": return "bg-evap-amber/10 text-evap-amber";
    case "upgrade": return "bg-evap-cyan/10 text-evap-cyan";
    default: return "bg-zinc-100 text-zinc-500";
  }
}

function timeRemaining(expiry: number): string {
  const diff = expiry - Date.now();
  if (diff <= 0) return "Expired";
  const h = Math.floor(diff / 3_600_000);
  const m = Math.floor((diff % 3_600_000) / 60_000);
  if (h > 24) return `${Math.floor(h / 24)}d ${h % 24}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

interface Props {
  proposal: Proposal;
  onSelect: (p: Proposal) => void;
  onVote: (p: Proposal) => void;
  onBoost: (p: Proposal) => void;
}

export function ProposalCard({ proposal: p, onSelect, onVote, onBoost }: Props) {
  const badge = statusBadge(p.status);
  const totalVotes = p.votes_for + p.votes_against;
  const forPct = totalVotes > 0 ? (p.votes_for / totalVotes) * 100 : 50;
  const energyPct = p.max_energy > 0 ? (p.current_energy / p.max_energy) * 100 : 0;
  const quorumPct = p.quorum > 0 ? Math.min(100, (totalVotes / p.quorum) * 100) : 0;

  return (
    <div
      className="bg-white rounded-xl border border-evap-border p-5 hover:shadow-sm transition-shadow cursor-pointer"
      onClick={() => onSelect(p)}
    >
      {/* Header */}
      <div className="flex items-start justify-between gap-3 mb-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 mb-1">
            <span className={`text-[9px] px-1.5 py-0.5 rounded-full font-medium ${badge.bg} ${badge.text}`}>
              {badge.label}
            </span>
            <span className={`text-[9px] px-1.5 py-0.5 rounded-full font-medium ${categoryColor(p.category)}`}>
              {p.category}
            </span>
          </div>
          <h3 className="text-sm font-semibold text-zinc-900 truncate">{p.title}</h3>
          <p className="text-[10px] text-zinc-400 mt-0.5 truncate">{p.description}</p>
        </div>
      </div>

      {/* Vote bar */}
      <div className="mb-3">
        <div className="flex items-center justify-between text-[10px] mb-1">
          <span className="text-evap-green font-medium">For {p.votes_for.toLocaleString()}</span>
          <span className="text-evap-red font-medium">Against {p.votes_against.toLocaleString()}</span>
        </div>
        <div className="h-1.5 rounded-full bg-evap-red/20 overflow-hidden">
          <div
            className="h-full rounded-full bg-evap-green transition-all duration-500"
            style={{ width: `${forPct}%` }}
          />
        </div>
      </div>

      {/* Energy + Quorum */}
      <div className="grid grid-cols-2 gap-3 mb-3">
        <div>
          <p className="text-[9px] text-zinc-400 uppercase tracking-wider mb-0.5">Energy</p>
          <div className="h-1 rounded-full bg-zinc-100 overflow-hidden">
            <div
              className={`h-full rounded-full transition-all duration-500 ${
                energyPct > 50 ? "bg-evap-cyan" : energyPct > 20 ? "bg-evap-amber" : "bg-evap-red"
              }`}
              style={{ width: `${energyPct}%` }}
            />
          </div>
          <p className="text-[10px] text-zinc-500 mt-0.5">{Math.round(energyPct)}%</p>
        </div>
        <div>
          <p className="text-[9px] text-zinc-400 uppercase tracking-wider mb-0.5">Quorum</p>
          <div className="h-1 rounded-full bg-zinc-100 overflow-hidden">
            <div
              className={`h-full rounded-full transition-all duration-500 ${
                p.quorum_reached ? "bg-evap-green" : "bg-evap-purple"
              }`}
              style={{ width: `${quorumPct}%` }}
            />
          </div>
          <p className="text-[10px] text-zinc-500 mt-0.5">
            {p.quorum_reached ? "Reached" : `${Math.round(quorumPct)}%`}
          </p>
        </div>
      </div>

      {/* Footer */}
      <div className="flex items-center justify-between pt-3 border-t border-evap-border">
        <div className="flex items-center gap-2 text-[10px] text-zinc-400">
          <span>by {p.proposer.slice(0, 6)}...{p.proposer.slice(-4)}</span>
          {p.status === "active" && (
            <span className="text-evap-amber">
              {timeRemaining(p.estimated_expiry)} left
            </span>
          )}
        </div>
        {p.status === "active" && (
          <div className="flex items-center gap-1.5" onClick={(e) => e.stopPropagation()}>
            <button
              onClick={() => onVote(p)}
              className="px-2.5 py-1 rounded-lg bg-evap-cyan/10 text-evap-cyan text-[10px] font-medium hover:bg-evap-cyan/20 transition-colors"
            >
              Vote
            </button>
            <button
              onClick={() => onBoost(p)}
              className="px-2.5 py-1 rounded-lg bg-evap-amber/10 text-evap-amber text-[10px] font-medium hover:bg-evap-amber/20 transition-colors"
            >
              Boost
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

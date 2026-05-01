import Link from "next/link";
import { Cpu, GitBranch } from "lucide-react";
import type { Proposal } from "@/lib/types";
import { EndorsementProgress } from "./EndorsementProgress";

interface Props {
  proposal: Proposal;
}

const STATE_TONE: Record<Proposal["state"], string> = {
  open: "border-evap-violet/30 bg-violet-50 text-evap-violet",
  active: "border-evap-emerald/30 bg-emerald-50 text-evap-emerald",
  historical: "border-zinc-200 bg-zinc-100 text-zinc-500",
};

const TYPE_LABEL: Record<Proposal["type"], string> = {
  fork_choice: "Fork-choice mode",
  upgrade_contract: "Contract upgrade",
};

export function ProposalCard({ proposal }: Props) {
  const endorsed = proposal.endorsements.reduce((s, e) => s + (e.stake || 0), 0);
  const Icon = proposal.type === "fork_choice" ? GitBranch : Cpu;

  return (
    <Link
      href={`/proposals/${encodeURIComponent(proposal.id)}`}
      className="block rounded-xl border border-evap-border bg-white p-4 transition hover:shadow-md"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5 text-[10px] font-medium text-zinc-500">
            <Icon className="h-3 w-3" />
            <span>{TYPE_LABEL[proposal.type]}</span>
          </div>
          <p className="mt-1 truncate text-sm font-semibold text-zinc-900">
            {proposal.title}
          </p>
          <p className="mt-0.5 text-[10px] font-mono text-zinc-400">
            #{proposal.id.slice(0, 12)}
          </p>
        </div>
        <span
          className={`rounded-full border px-2 py-0.5 text-[10px] font-medium uppercase tracking-wider ${STATE_TONE[proposal.state]}`}
        >
          {proposal.state}
        </span>
      </div>

      <div className="mt-3">
        <EndorsementProgress
          endorsed={endorsed}
          required={proposal.required_stake}
          compact
        />
        <div className="mt-1.5 flex items-center justify-between text-[10px] text-zinc-500">
          <span>
            {proposal.endorsements.length} endorser
            {proposal.endorsements.length === 1 ? "" : "s"}
          </span>
          <span className="font-mono tabular-nums">
            {endorsed.toLocaleString()} / {proposal.required_stake.toLocaleString()} EVAP
          </span>
        </div>
      </div>
    </Link>
  );
}

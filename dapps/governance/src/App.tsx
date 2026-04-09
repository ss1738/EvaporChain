import { useState, useEffect, useCallback } from "react";
import { getProposals, getGovernanceStats, getStatus } from "@/utils/api";
import type { Proposal, GovernanceStats, ChainStatus } from "@/utils/types";
import { useWalletConnect } from "@/hooks/useWalletConnect";
import { ProposalCard } from "@/components/ProposalCard";
import { ProposalDetail } from "@/components/ProposalDetail";
import { VoteModal } from "@/components/VoteModal";
import { BoostModal } from "@/components/BoostModal";
import { CreateProposalModal } from "@/components/CreateProposalModal";
import { DelegateModal } from "@/components/DelegateModal";

type Tab = "all" | "active" | "passed" | "evaporated";
type View = "list" | "detail";

export function App() {
  const wallet = useWalletConnect();
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [stats, setStats] = useState<GovernanceStats | null>(null);
  const [chainStatus, setChainStatus] = useState<ChainStatus | null>(null);
  const [tab, setTab] = useState<Tab>("all");
  const [view, setView] = useState<View>("list");
  const [selectedProposalId, setSelectedProposalId] = useState<string | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [showDelegate, setShowDelegate] = useState(false);
  const [voteTarget, setVoteTarget] = useState<Proposal | null>(null);
  const [boostTarget, setBoostTarget] = useState<Proposal | null>(null);
  const [loading, setLoading] = useState(true);

  const fetchData = useCallback(async () => {
    try {
      const [propData, statsData, statusData] = await Promise.allSettled([
        getProposals(),
        getGovernanceStats(),
        getStatus(),
      ]);
      if (propData.status === "fulfilled") setProposals(Array.isArray(propData.value) ? propData.value : []);
      if (statsData.status === "fulfilled") setStats(statsData.value);
      if (statusData.status === "fulfilled") setChainStatus(statusData.value);
    } catch {
      // retry on next poll
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 8000);
    return () => clearInterval(interval);
  }, [fetchData]);

  const handleSelectProposal = (p: Proposal) => {
    setSelectedProposalId(p.id);
    setView("detail");
  };

  const filtered = proposals.filter((p) => {
    if (tab === "active") return p.status === "active";
    if (tab === "passed") return p.status === "passed" || p.status === "rejected";
    if (tab === "evaporated") return p.status === "evaporated" || p.status === "expired";
    return true;
  });

  const activeCount = proposals.filter((p) => p.status === "active").length;
  const passedCount = proposals.filter((p) => p.status === "passed").length;
  const evaporatedCount = proposals.filter((p) => p.status === "evaporated" || p.status === "expired").length;

  return (
    <div className="min-h-screen bg-evap-bg">
      {/* Navbar */}
      <nav className="bg-white border-b border-evap-border sticky top-0 z-40">
        <div className="max-w-5xl mx-auto px-4 py-3 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-evap-amber to-evap-purple flex items-center justify-center">
              <span className="text-sm font-bold text-white">G</span>
            </div>
            <div>
              <h1 className="text-sm font-bold text-zinc-900">Governance</h1>
              <p className="text-[10px] text-zinc-400">EvaporChain DAO</p>
            </div>
          </div>

          <div className="flex items-center gap-3">
            {chainStatus && (
              <div className="hidden sm:flex items-center gap-3 text-[10px] text-zinc-400">
                <span className="flex items-center gap-1">
                  <span className="w-1.5 h-1.5 rounded-full bg-evap-green animate-pulse" />
                  Block {chainStatus.block_height.toLocaleString()}
                </span>
                <span>Epoch {chainStatus.epoch.toLocaleString()}</span>
              </div>
            )}

            {wallet.connected ? (
              <div className="flex items-center gap-2">
                <span className="text-[10px] text-zinc-500 font-mono hidden sm:inline">
                  {wallet.address?.slice(0, 6)}...{wallet.address?.slice(-4)}
                </span>
                <button
                  onClick={() => setShowDelegate(true)}
                  className="text-[10px] text-evap-purple hover:text-evap-purple/80 px-2 py-1 rounded border border-evap-border hidden sm:block"
                >
                  Delegate
                </button>
                <button
                  onClick={wallet.disconnect}
                  className="text-[10px] text-zinc-400 hover:text-zinc-600 px-2 py-1 rounded border border-evap-border"
                >
                  Disconnect
                </button>
              </div>
            ) : (
              <button
                onClick={wallet.connect}
                disabled={wallet.connecting}
                className="px-4 py-2 rounded-lg bg-gradient-to-r from-evap-amber to-evap-purple text-xs font-semibold text-white hover:opacity-90 transition disabled:opacity-50"
              >
                {wallet.connecting ? "Connecting..." : "Connect Wallet"}
              </button>
            )}
          </div>
        </div>
      </nav>

      {/* Main Content */}
      <div className="max-w-5xl mx-auto px-4 py-6">
        {view === "list" && (
          <>
            {/* Stats */}
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-6">
              <StatCard
                label="Active Proposals"
                value={stats?.active_proposals ?? activeCount}
                color="text-evap-cyan"
              />
              <StatCard
                label="Passed"
                value={stats?.passed_proposals ?? passedCount}
                color="text-evap-green"
              />
              <StatCard
                label="Evaporated"
                value={stats?.evaporated_proposals ?? evaporatedCount}
                color="text-zinc-400"
              />
              <StatCard
                label="Participation"
                value={stats ? `${(stats.participation_rate * 100).toFixed(0)}%` : "—"}
                color="text-evap-purple"
              />
            </div>

            {/* Tabs + Create */}
            <div className="flex items-center justify-between mb-4">
              <div className="flex bg-white rounded-lg border border-evap-border p-0.5">
                {([
                  { key: "all" as Tab, label: "All", count: proposals.length },
                  { key: "active" as Tab, label: "Active", count: activeCount },
                  { key: "passed" as Tab, label: "Decided", count: passedCount },
                  { key: "evaporated" as Tab, label: "Evaporated", count: evaporatedCount },
                ]).map((t) => (
                  <button
                    key={t.key}
                    onClick={() => setTab(t.key)}
                    className={`px-3 py-1.5 rounded-md text-xs font-medium transition ${
                      tab === t.key ? "bg-zinc-100 text-zinc-900" : "text-zinc-500 hover:text-zinc-700"
                    }`}
                  >
                    {t.label} <span className="text-zinc-400">({t.count})</span>
                  </button>
                ))}
              </div>

              <button
                onClick={() => setShowCreate(true)}
                className="px-4 py-2 rounded-lg bg-gradient-to-r from-evap-purple to-evap-cyan text-xs font-semibold text-white hover:opacity-90 transition"
              >
                + New Proposal
              </button>
            </div>

            {/* Proposal Grid */}
            {loading ? (
              <div className="flex items-center justify-center py-20">
                <div className="w-8 h-8 border-2 border-evap-cyan/30 border-t-evap-cyan rounded-full animate-spin" />
              </div>
            ) : filtered.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-20 bg-white rounded-xl border border-evap-border">
                <span className="text-3xl mb-3">
                  {tab === "evaporated" ? "👻" : "📋"}
                </span>
                <p className="text-sm text-zinc-500">
                  {tab === "evaporated" ? "No evaporated proposals" :
                   tab === "active" ? "No active proposals" :
                   "No proposals yet"}
                </p>
                <p className="text-xs text-zinc-400 mt-1">
                  Create a proposal to start governing the chain
                </p>
              </div>
            ) : (
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                {filtered.map((p) => (
                  <ProposalCard
                    key={p.id}
                    proposal={p}
                    onSelect={handleSelectProposal}
                    onVote={setVoteTarget}
                    onBoost={setBoostTarget}
                  />
                ))}
              </div>
            )}

            {/* Wallet hint */}
            {!wallet.connected && (
              <div className="mt-6 px-4 py-3 rounded-lg bg-evap-amber/5 border border-evap-amber/20 text-center">
                <p className="text-xs text-evap-amber">
                  Connect your EvaporChain Wallet to create proposals, vote, and delegate
                </p>
                {wallet.error && (
                  <p className="text-[10px] text-evap-red mt-1">{wallet.error}</p>
                )}
              </div>
            )}

            {/* Explainer */}
            <div className="mt-8 bg-white rounded-xl border border-evap-border p-5">
              <h3 className="text-sm font-semibold text-zinc-900 mb-2">
                How Mortal Governance Works
              </h3>
              <div className="grid sm:grid-cols-3 gap-4 text-[10px] text-zinc-500">
                <div>
                  <p className="font-medium text-zinc-700 mb-0.5">1. Propose</p>
                  <p>Create a proposal with initial energy and a decay rate. Higher energy = more time for voting.</p>
                </div>
                <div>
                  <p className="font-medium text-zinc-700 mb-0.5">2. Vote & Boost</p>
                  <p>Vote for or against. Boost energy to keep important proposals alive until quorum is reached.</p>
                </div>
                <div>
                  <p className="font-medium text-zinc-700 mb-0.5">3. Resolve or Evaporate</p>
                  <p>Proposals that reach quorum are decided. Those that don&apos;t evaporate — cleaning up governance debt.</p>
                </div>
              </div>
            </div>
          </>
        )}

        {view === "detail" && selectedProposalId && (
          <ProposalDetail
            proposalId={selectedProposalId}
            onBack={() => setView("list")}
            onVote={setVoteTarget}
            onBoost={setBoostTarget}
          />
        )}
      </div>

      {/* Footer */}
      <footer className="border-t border-evap-border mt-12 py-6">
        <div className="max-w-5xl mx-auto px-4 flex items-center justify-between">
          <p className="text-[10px] text-zinc-400">
            Governance — Powered by EvaporChain
          </p>
          <p className="text-[10px] text-zinc-400">
            Proposals decay. Only active governance survives.
          </p>
        </div>
      </footer>

      {/* Modals */}
      {showCreate && (
        <CreateProposalModal
          proposerAddress={wallet.address}
          onClose={() => setShowCreate(false)}
          onCreated={fetchData}
        />
      )}
      {voteTarget && (
        <VoteModal
          proposal={voteTarget}
          voterAddress={wallet.address}
          onClose={() => setVoteTarget(null)}
          onVoted={fetchData}
        />
      )}
      {boostTarget && (
        <BoostModal
          proposal={boostTarget}
          onClose={() => setBoostTarget(null)}
          onBoosted={fetchData}
        />
      )}
      {showDelegate && wallet.address && (
        <DelegateModal
          fromAddress={wallet.address}
          onClose={() => setShowDelegate(false)}
          onDelegated={fetchData}
        />
      )}
    </div>
  );
}

function StatCard({
  label,
  value,
  color,
}: {
  label: string;
  value: number | string;
  color: string;
}) {
  return (
    <div className="bg-white rounded-xl border border-evap-border px-4 py-3">
      <p className={`text-xl font-bold ${color}`}>{typeof value === "number" ? value.toLocaleString() : value}</p>
      <p className="text-[10px] text-zinc-400">{label}</p>
    </div>
  );
}

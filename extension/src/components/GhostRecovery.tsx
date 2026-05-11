import { useEffect, useState, useMemo } from "react";
import { ArrowLeft, Ghost, Sparkles } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import { formatBalance } from "@/utils/format";
import { Header } from "./Header";
import { api } from "@/utils/api";
import type { GhostObject, GhostDetail } from "@/utils/api";

type DetailPanel = GhostDetail | null;

export function GhostRecovery() {
  const {
    ghosts, refreshGhosts, resurrectGhost,
    setView, loading, balance,
  } = useWallet();

  const [selectedDetail, setSelectedDetail] = useState<DetailPanel>(null);
  const [resurrecting, setResurrecting] = useState<string | null>(null);
  const [energyInput, setEnergyInput] = useState<number>(0);

  useEffect(() => {
    refreshGhosts();
    const interval = setInterval(refreshGhosts, 15_000);
    return () => clearInterval(interval);
  }, [refreshGhosts]);

  const stats = useMemo(() => {
    const recoverable = ghosts.filter(g => g.proof_status !== "expired").length;
    const expired = ghosts.filter(g => g.proof_status === "expired").length;
    return { recoverable, expired, total: ghosts.length };
  }, [ghosts]);

  const sortedGhosts = useMemo(() => {
    return [...ghosts].sort((a, b) => {
      // Recoverable first, then by recovery window remaining
      if (a.proof_status === "expired" && b.proof_status !== "expired") return 1;
      if (a.proof_status !== "expired" && b.proof_status === "expired") return -1;
      return a.recovery_window_remaining - b.recovery_window_remaining;
    });
  }, [ghosts]);

  const handleSelectGhost = async (ghost: GhostObject) => {
    try {
      const detail = await api.getGhostDetail(ghost.id);
      setSelectedDetail(detail);
      setEnergyInput(ghost.recovery_cost);
    } catch {
      // If detail fails, construct a partial view
      setSelectedDetail(null);
    }
  };

  const handleResurrect = async (id: string, energy: number) => {
    setResurrecting(id);
    try {
      await resurrectGhost(id, energy);
      setSelectedDetail(null);
      setResurrecting(null);
    } catch {
      setResurrecting(null);
    }
  };

  // Detail panel view
  if (selectedDetail) {
    return (
      <div className="flex flex-col h-full">
        <Header />

        <div className="px-4 pt-4 pb-2 flex items-center justify-between">
          <div>
            <h2 className="text-lg font-semibold text-zinc-100">Ghost Detail</h2>
            <p className="text-xs text-zinc-500 italic">{selectedDetail.name || "Unknown Object"}</p>
          </div>
          <button
            onClick={() => setSelectedDetail(null)}
            className="text-xs text-zinc-500 hover:text-zinc-300"
          >
            <><ArrowLeft className="inline w-3.5 h-3.5 mr-1 -mt-0.5" strokeWidth={1.5} />Back</>
          </button>
        </div>

        <div className="flex-1 overflow-y-auto px-4 pb-4 space-y-3">
          {/* Proof status banner */}
          <ProofStatusBanner status={selectedDetail.proof_status} />

          {/* Metadata */}
          <div className="px-3 py-3 rounded-lg bg-evap-surface border border-evap-border space-y-2">
            <MetaRow label="Object ID" value={`${selectedDetail.id.slice(0, 12)}...${selectedDetail.id.slice(-8)}`} mono />
            <MetaRow label="Mint Date" value={selectedDetail.mint_date} />
            <MetaRow label="Evaporation Date" value={selectedDetail.evaporation_date} />
            <MetaRow label="Evaporated" value={`${selectedDetail.epochs_since_evaporation} epochs ago`} />
            <MetaRow label="Original Energy" value={`${formatBalance(selectedDetail.original_energy)} / ${formatBalance(selectedDetail.max_energy)}`} />
            <MetaRow label="Half-life" value={`${selectedDetail.half_life} epochs`} />
            <MetaRow label="Merkle Proof" value={selectedDetail.merkle_proof.slice(0, 16) + "..."} mono />
          </div>

          {/* Custom metadata */}
          {selectedDetail.metadata && Object.keys(selectedDetail.metadata).length > 0 && (
            <div className="px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
              <p className="text-xs font-semibold text-zinc-400 mb-2">Metadata</p>
              {Object.entries(selectedDetail.metadata).map(([key, value]) => (
                <MetaRow key={key} label={key} value={value} />
              ))}
            </div>
          )}

          {/* Energy history chart (text-based) */}
          {selectedDetail.energy_history && selectedDetail.energy_history.length > 0 && (
            <div className="px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
              <p className="text-xs font-semibold text-zinc-400 mb-2">Energy Over Lifetime</p>
              <div className="space-y-1">
                {selectedDetail.energy_history.slice(-10).map((entry) => (
                  <div key={entry.epoch} className="flex items-center gap-2">
                    <span className="text-[8px] text-zinc-600 w-10 text-right">E{entry.epoch}</span>
                    <div className="flex-1 h-2 bg-evap-border rounded-full overflow-hidden">
                      <div
                        className="h-2 rounded-full transition-all"
                        style={{
                          width: `${Math.max(entry.percent, 1)}%`,
                          backgroundColor: entry.percent > 50 ? "#22c55e" : entry.percent > 20 ? "#f59e0b" : "#ef4444",
                        }}
                      />
                    </div>
                    <span className="text-[8px] text-zinc-500 w-8">{entry.percent}%</span>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Resurrect CTA */}
          {selectedDetail.proof_status !== "expired" && (
            <div className="px-3 py-3 rounded-lg bg-evap-green/5 border border-evap-green/20">
              <p className="text-xs font-semibold text-zinc-200 mb-2">Resurrect This Object</p>

              <div className="flex items-center gap-2 mb-2">
                <label className="text-xs text-zinc-400">Energy deposit:</label>
                <input
                  type="number"
                  min={selectedDetail.recovery_cost}
                  max={selectedDetail.max_energy}
                  value={energyInput}
                  onChange={e => setEnergyInput(Number(e.target.value))}
                  className="flex-1 px-2 py-1 rounded bg-evap-surface border border-evap-border text-xs text-zinc-200 outline-none focus:border-evap-cyan/40"
                />
                <span className="text-xs text-zinc-500">EVAP</span>
              </div>

              <p className="text-[10px] text-zinc-500 mb-2">
                Minimum cost: {formatBalance(selectedDetail.recovery_cost)} EVAP
              </p>

              <button
                onClick={() => handleResurrect(selectedDetail.id, energyInput)}
                disabled={loading || balance < energyInput || energyInput < selectedDetail.recovery_cost}
                className="w-full py-2.5 rounded-lg text-xs font-semibold bg-evap-green text-black hover:bg-evap-green/90 transition disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {resurrecting === selectedDetail.id
                  ? "Resurrecting..."
                  : balance < energyInput
                  ? "Insufficient balance"
                  : `Resurrect for ${formatBalance(energyInput)} EVAP`}
              </button>
            </div>
          )}

          {selectedDetail.proof_status === "expired" && (
            <div className="px-3 py-3 rounded-lg bg-red-500/5 border border-red-500/20">
              <p className="text-xs font-semibold text-red-400">Unrecoverable</p>
              <p className="text-xs text-zinc-500 mt-1">
                The Merkle proof for this object has expired. It can no longer be resurrected.
              </p>
            </div>
          )}
        </div>
      </div>
    );
  }

  // Main ghost list view
  return (
    <div className="flex flex-col h-full">
      <Header />

      <div className="px-4 pt-4 pb-2 flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold text-zinc-100">
            Ghost Recovery <span className="text-base"><Ghost className="w-3.5 h-3.5" strokeWidth={1.5} /></span>
          </h2>
          <p className="text-xs text-zinc-500">Recover evaporated objects</p>
        </div>
        <button
          onClick={() => setView("home")}
          className="text-xs text-zinc-500 hover:text-zinc-300"
        >
          <><ArrowLeft className="inline w-3.5 h-3.5 mr-1 -mt-0.5" strokeWidth={1.5} />Back</>
        </button>
      </div>

      {/* Stats header */}
      {ghosts.length > 0 && (
        <div className="mx-4 mb-3 px-3 py-2.5 rounded-lg bg-evap-surface border border-evap-border">
          <div className="grid grid-cols-3 gap-2">
            <GhostStat label="Recoverable" value={stats.recoverable} color="text-evap-green" />
            <GhostStat label="Expired" value={stats.expired} color="text-red-400" />
            <GhostStat label="Total Ghosts" value={stats.total} color="text-zinc-300" />
          </div>
        </div>
      )}

      {/* Ghost list */}
      <div className="flex-1 overflow-y-auto px-4 pb-4 space-y-2">
        {ghosts.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-12">
            <span className="text-3xl mb-3"><Sparkles className="w-3.5 h-3.5" strokeWidth={1.5} /></span>
            <p className="text-sm text-zinc-400">No ghosts</p>
            <p className="text-xs text-zinc-600 mt-1">All your objects are alive!</p>
          </div>
        ) : (
          sortedGhosts.map(ghost => (
            <GhostCard
              key={ghost.id}
              ghost={ghost}
              onTap={() => handleSelectGhost(ghost)}
              onResurrect={() => handleResurrect(ghost.id, ghost.recovery_cost)}
              isResurrecting={resurrecting === ghost.id}
              canAfford={balance >= ghost.recovery_cost}
              loading={loading}
            />
          ))
        )}
      </div>
    </div>
  );
}

/* ── Sub-components ── */

function GhostCard({
  ghost, onTap, onResurrect, isResurrecting, canAfford, loading,
}: {
  ghost: GhostObject;
  onTap: () => void;
  onResurrect: () => void;
  isResurrecting: boolean;
  canAfford: boolean;
  loading: boolean;
}) {
  const isExpired = ghost.proof_status === "expired";
  const isExpiring = ghost.proof_status === "expiring";
  const recoveryPercent = ghost.recovery_window_total > 0
    ? Math.round((ghost.recovery_window_remaining / ghost.recovery_window_total) * 100)
    : 0;

  return (
    <div
      className={`px-3 py-3 rounded-lg border transition ${
        isExpired
          ? "bg-zinc-900/60 border-zinc-800 opacity-60"
          : "bg-evap-surface border-evap-border"
      }`}
    >
      <button onClick={onTap} className="w-full text-left">
        <div className="flex items-center justify-between mb-1.5">
          <div className="flex items-center gap-2">
            <span className="text-sm">{isExpired ? "💀" : "👻"}</span>
            <div>
              <p className={`text-xs font-semibold ${isExpired ? "text-zinc-500 italic" : "text-zinc-400 italic"}`}>
                {ghost.name || "Unknown Object"}
              </p>
              <p className="text-[10px] text-zinc-600 font-mono">
                {ghost.id.slice(0, 8)}...{ghost.id.slice(-6)}
              </p>
            </div>
          </div>

          {/* Proof badge */}
          <span className={`text-[10px] px-2 py-0.5 rounded-full ${
            isExpired
              ? "bg-red-500/10 text-red-400"
              : isExpiring
              ? "bg-evap-amber/10 text-evap-amber"
              : "bg-evap-green/10 text-evap-green"
          }`}>
            {isExpired ? "Unrecoverable" : isExpiring ? "Proof Expiring" : "Proof Valid"}
          </span>
        </div>

        {/* Evaporation info */}
        <div className="flex items-center justify-between text-[10px] text-zinc-500 mb-1.5">
          <span>Evaporated {ghost.epochs_since_evaporation} epochs ago</span>
          <span>Half-life: {ghost.half_life}e</span>
        </div>

        {/* Recovery window bar */}
        {!isExpired && (
          <div className="mb-1.5">
            <div className="flex justify-between text-[10px] mb-0.5">
              <span className="text-zinc-500">Recovery window</span>
              <span className={isExpiring ? "text-evap-amber" : "text-zinc-400"}>
                {ghost.recovery_window_remaining} epochs left
              </span>
            </div>
            <div className="w-full h-1.5 bg-evap-border rounded-full overflow-hidden">
              <div
                className="h-1.5 rounded-full transition-all duration-500"
                style={{
                  width: `${Math.max(recoveryPercent, 2)}%`,
                  backgroundColor: recoveryPercent > 50 ? "#22c55e" : recoveryPercent > 20 ? "#f59e0b" : "#ef4444",
                }}
              />
            </div>
          </div>
        )}
      </button>

      {/* Resurrect button */}
      {!isExpired && (
        <div className="flex items-center justify-between mt-2">
          <span className="text-[10px] text-zinc-500">
            Cost: <span className="text-evap-cyan font-medium">{formatBalance(ghost.recovery_cost)} EVAP</span>
          </span>
          <button
            onClick={(e) => { e.stopPropagation(); onResurrect(); }}
            disabled={loading || !canAfford}
            className="px-3 py-1.5 rounded-lg text-xs font-semibold bg-evap-green text-black hover:bg-evap-green/90 transition disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {isResurrecting ? "..." : "Resurrect"}
          </button>
        </div>
      )}

      {isExpired && (
        <div className="mt-1.5">
          <span className="text-[10px] text-red-400/70 italic">
            Merkle proof expired — this ghost is permanently lost
          </span>
        </div>
      )}
    </div>
  );
}

function GhostStat({ label, value, color }: { label: string; value: number; color: string }) {
  return (
    <div className="text-center">
      <p className={`text-sm font-semibold ${color}`}>{value}</p>
      <p className="text-[10px] text-zinc-500">{label}</p>
    </div>
  );
}

function ProofStatusBanner({ status }: { status: "valid" | "expiring" | "expired" }) {
  const config = {
    valid: { bg: "bg-evap-green/10", border: "border-evap-green/30", text: "text-evap-green", label: "Merkle Proof Valid", icon: "✓" },
    expiring: { bg: "bg-evap-amber/10", border: "border-evap-amber/30", text: "text-evap-amber", label: "Proof Expiring Soon", icon: "⚠" },
    expired: { bg: "bg-red-500/10", border: "border-red-500/30", text: "text-red-400", label: "Proof Expired (Unrecoverable)", icon: "✕" },
  };

  const c = config[status];

  return (
    <div className={`px-3 py-2 rounded-lg ${c.bg} border ${c.border}`}>
      <p className={`text-xs font-medium ${c.text} text-center`}>
        {c.icon} {c.label}
      </p>
    </div>
  );
}

function MetaRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-xs text-zinc-500">{label}</span>
      <span className={`text-xs text-zinc-300 ${mono ? "font-mono" : ""}`}>{value}</span>
    </div>
  );
}

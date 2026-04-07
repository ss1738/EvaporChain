import { useState, useEffect, useCallback } from "react";
import { api } from "@/utils/api";
import type { Pool, ChainStatus } from "@/utils/types";
import { useWalletConnect } from "@/hooks/useWalletConnect";
import { PoolCard } from "@/components/PoolCard";
import { PoolDetail } from "@/components/PoolDetail";
import { CreatePoolModal } from "@/components/CreatePoolModal";
import { StakeModal } from "@/components/StakeModal";
import { Dashboard } from "@/components/Dashboard";

type View = "pools" | "detail" | "dashboard";

export function App() {
  const wallet = useWalletConnect();
  const [pools, setPools] = useState<Pool[]>([]);
  const [status, setStatus] = useState<ChainStatus | null>(null);
  const [view, setView] = useState<View>("pools");
  const [selectedPoolId, setSelectedPoolId] = useState<string | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [stakeTarget, setStakeTarget] = useState<Pool | null>(null);
  const [loading, setLoading] = useState(true);

  const fetchPools = useCallback(async () => {
    try {
      const [poolData, statusData] = await Promise.all([
        api.getPools(),
        api.getStatus(),
      ]);
      setPools(poolData);
      setStatus(statusData);
    } catch {
      // retry on next poll
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchPools();
    const interval = setInterval(fetchPools, 8000);
    return () => clearInterval(interval);
  }, [fetchPools]);

  const handleSelectPool = (pool: Pool) => {
    setSelectedPoolId(pool.id);
    setView("detail");
  };

  const handleSelectPoolById = (poolId: string) => {
    setSelectedPoolId(poolId);
    setView("detail");
  };

  const totalEnergy = pools.reduce((s, p) => s + p.total_energy, 0);
  const totalObjects = pools.reduce((s, p) => s + p.protected_objects.length, 0);
  const totalContributors = pools.reduce((s, p) => s + p.contributor_count, 0);

  return (
    <div className="min-h-screen bg-evap-bg">
      {/* Navbar */}
      <nav className="bg-white border-b border-evap-border sticky top-0 z-40">
        <div className="max-w-6xl mx-auto px-4 py-3 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-evap-cyan to-evap-green flex items-center justify-center">
              <span className="text-sm font-bold text-white">EP</span>
            </div>
            <div>
              <h1 className="text-sm font-bold text-zinc-900">Energy Pool</h1>
              <p className="text-[10px] text-zinc-400">EvaporChain Staking</p>
            </div>
          </div>

          <div className="flex items-center gap-3">
            {/* Nav links */}
            <div className="hidden sm:flex items-center gap-1 mr-3">
              <NavBtn
                label="Pools"
                active={view === "pools"}
                onClick={() => setView("pools")}
              />
              <NavBtn
                label="Dashboard"
                active={view === "dashboard"}
                onClick={() => {
                  if (wallet.connected) setView("dashboard");
                }}
                disabled={!wallet.connected}
              />
            </div>

            {status && (
              <div className="hidden sm:flex items-center gap-3 text-[10px] text-zinc-400">
                <span className="flex items-center gap-1">
                  <span className="w-1.5 h-1.5 rounded-full bg-evap-green animate-pulse" />
                  Block {status.block_height.toLocaleString()}
                </span>
                <span>Epoch {status.epoch.toLocaleString()}</span>
              </div>
            )}

            {wallet.connected ? (
              <div className="flex items-center gap-2">
                <span className="text-[10px] text-zinc-500 font-mono hidden sm:inline">
                  {wallet.address?.slice(0, 6)}...{wallet.address?.slice(-4)}
                </span>
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
                className="px-4 py-2 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-green text-xs font-semibold text-white hover:opacity-90 transition disabled:opacity-50"
              >
                {wallet.connecting ? "Connecting..." : "Connect Wallet"}
              </button>
            )}
          </div>
        </div>
      </nav>

      {/* Main Content */}
      <div className="max-w-6xl mx-auto px-4 py-6">
        {view === "pools" && (
          <>
            {/* Hero stats */}
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-6">
              <StatCard
                label="Active Pools"
                value={pools.length}
                color="text-evap-cyan"
              />
              <StatCard
                label="Total Energy"
                value={totalEnergy}
                color="text-evap-green"
              />
              <StatCard
                label="Protected Objects"
                value={totalObjects}
                color="text-evap-purple"
              />
              <StatCard
                label="Contributors"
                value={totalContributors}
                color="text-evap-amber"
              />
            </div>

            {/* Actions bar */}
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-sm font-bold text-zinc-900">
                Energy Pools
              </h2>
              <button
                onClick={() => setShowCreate(true)}
                className="px-4 py-2 rounded-lg bg-gradient-to-r from-evap-purple to-evap-cyan text-xs font-semibold text-white hover:opacity-90 transition"
              >
                + Create Pool
              </button>
            </div>

            {/* Pool Grid */}
            {loading ? (
              <div className="flex items-center justify-center py-20">
                <div className="w-8 h-8 border-2 border-evap-cyan/30 border-t-evap-cyan rounded-full animate-spin" />
              </div>
            ) : pools.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-20 bg-white rounded-xl border border-evap-border">
                <span className="text-3xl mb-3">{`{ }`}</span>
                <p className="text-sm text-zinc-500">No energy pools yet</p>
                <p className="text-xs text-zinc-400 mt-1">
                  Create the first pool to start protecting objects
                </p>
              </div>
            ) : (
              <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
                {pools.map((pool) => (
                  <PoolCard
                    key={pool.id}
                    pool={pool}
                    onSelect={handleSelectPool}
                    onStake={setStakeTarget}
                  />
                ))}
              </div>
            )}

            {/* Wallet hint */}
            {!wallet.connected && (
              <div className="mt-6 px-4 py-3 rounded-lg bg-evap-cyan/5 border border-evap-cyan/20 text-center">
                <p className="text-xs text-evap-cyan">
                  Connect your EvaporChain Wallet to create pools, stake energy, and earn guardian points
                </p>
                {wallet.error && (
                  <p className="text-[10px] text-evap-red mt-1">{wallet.error}</p>
                )}
              </div>
            )}
          </>
        )}

        {view === "detail" && selectedPoolId && (
          <PoolDetail
            poolId={selectedPoolId}
            walletAddress={wallet.address}
            onBack={() => setView("pools")}
            onStake={setStakeTarget}
          />
        )}

        {view === "dashboard" && wallet.address && (
          <Dashboard
            address={wallet.address}
            onSelectPool={handleSelectPoolById}
          />
        )}

        {view === "dashboard" && !wallet.connected && (
          <div className="flex flex-col items-center justify-center py-20 bg-white rounded-xl border border-evap-border">
            <p className="text-sm text-zinc-500">
              Connect your wallet to view your dashboard
            </p>
          </div>
        )}
      </div>

      {/* Footer */}
      <footer className="border-t border-evap-border mt-12 py-6">
        <div className="max-w-6xl mx-auto px-4 flex items-center justify-between">
          <p className="text-[10px] text-zinc-400">
            Energy Pool — Powered by EvaporChain
          </p>
          <p className="text-[10px] text-zinc-400">
            Stake energy. Protect objects. Earn guardian points.
          </p>
        </div>
      </footer>

      {/* Modals */}
      {showCreate && (
        <CreatePoolModal
          onClose={() => setShowCreate(false)}
          onCreated={fetchPools}
        />
      )}
      {stakeTarget && (
        <StakeModal
          pool={stakeTarget}
          onClose={() => setStakeTarget(null)}
          onStaked={fetchPools}
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
  value: number;
  color: string;
}) {
  return (
    <div className="bg-white rounded-xl border border-evap-border px-4 py-3">
      <p className={`text-xl font-bold ${color}`}>{value.toLocaleString()}</p>
      <p className="text-[10px] text-zinc-400">{label}</p>
    </div>
  );
}

function NavBtn({
  label,
  active,
  onClick,
  disabled,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={`px-3 py-1.5 rounded-md text-xs font-medium transition ${
        active
          ? "bg-zinc-100 text-zinc-900"
          : disabled
            ? "text-zinc-300 cursor-not-allowed"
            : "text-zinc-500 hover:text-zinc-700"
      }`}
    >
      {label}
    </button>
  );
}

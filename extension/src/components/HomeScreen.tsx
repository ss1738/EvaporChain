import { useEffect } from "react";
import { useWallet } from "@/hooks/useWallet";
import { formatBalance, shortAddress } from "@/utils/format";
import { Header } from "./Header";
import { QuantumBadge } from "./QuantumBadge";

export function HomeScreen() {
  const {
    activeAccount, balance, chainStatus, ghosts, wcSessions,
    ledgerConnected,
    setView, claimFaucet, refreshBalance, refreshObjects, refreshGhosts,
    loading, notification, setNotification,
  } = useWallet();

  const recoverableGhosts = ghosts.filter(g => g.proof_status !== "expired").length;

  useEffect(() => {
    refreshBalance();
    refreshGhosts();
    const interval = setInterval(refreshBalance, 10_000);
    return () => clearInterval(interval);
  }, [refreshBalance, refreshGhosts]);

  useEffect(() => {
    if (notification) {
      const t = setTimeout(() => setNotification(null), 3000);
      return () => clearTimeout(t);
    }
  }, [notification, setNotification]);

  if (!activeAccount) return null;

  return (
    <div className="flex flex-col h-full">
      <Header />

      {/* Notification banner */}
      {notification && (
        <div className="mx-4 mt-2 px-3 py-2 rounded-lg bg-evap-green/10 border border-evap-green/30">
          <p className="text-xs text-evap-green text-center">{notification}</p>
        </div>
      )}

      {/* Hardware wallet indicator */}
      {ledgerConnected && (
        <div className="mx-4 mt-2 px-3 py-1.5 rounded-lg bg-blue-500/10 border border-blue-500/30 flex items-center gap-2">
          <div className="w-2 h-2 rounded-full bg-blue-500" />
          <span className="text-[10px] text-blue-400 font-medium">Hardware Wallet Connected</span>
          <button
            onClick={() => setView("ledger")}
            className="ml-auto text-[10px] text-blue-400 hover:text-blue-300 transition"
          >
            Manage
          </button>
        </div>
      )}

      {/* Balance card */}
      <div className="px-4 pt-6 pb-4">
        <p className="text-xs text-zinc-500 mb-1">Total Balance</p>
        <div className="flex items-baseline gap-2">
          <span className="text-3xl font-bold text-zinc-100">
            {formatBalance(balance)}
          </span>
          <span className="text-sm text-zinc-500">EVAP</span>
        </div>
        <p className="text-[10px] text-zinc-600 font-mono mt-1">
          {activeAccount.address}
        </p>
      </div>

      {/* Quick actions */}
      <div className="grid grid-cols-4 gap-2 px-4 pb-2">
        <QuickAction label="Send" icon="↑" onClick={() => setView("send")} />
        <QuickAction label="Receive" icon="↓" onClick={() => setView("receive")} />
        <QuickAction label="Swap" icon="⇄" onClick={() => setView("swap")} />
        <QuickAction label="Bridge" icon="🌉" onClick={() => setView("bridge")} />
      </div>
      <div className="grid grid-cols-5 gap-2 px-4 pb-2">
        <QuickAction label="Buy" icon="$" onClick={() => setView("buy")} />
        <QuickAction label="Objects" icon="◈" onClick={() => { setView("objects"); refreshObjects(); }} />
        <QuickAction label="NFTs" icon="🖼" onClick={() => setView("nfts")} />
        <QuickAction label="Energy" icon="⚡" onClick={() => setView("energy-dashboard")} />
        <QuickAction
          label="Faucet"
          icon="💧"
          onClick={claimFaucet}
          disabled={loading}
        />
      </div>
      <div className="grid grid-cols-3 gap-2 px-4 pb-4">
        <QuickAction label="Batch Refresh" icon="🔄" onClick={() => { setView("batch-refresh"); refreshObjects(); }} />
        <QuickAction
          label={recoverableGhosts > 0 ? `Ghosts (${recoverableGhosts})` : "Ghosts"}
          icon="👻"
          onClick={() => { setView("ghost-recovery"); refreshGhosts(); }}
          badge={recoverableGhosts > 0 ? recoverableGhosts : undefined}
        />
        <QuickAction label="Forecast" icon="📉" onClick={() => { setView("decay-forecast"); refreshObjects(); }} />
        <QuickAction
          label={wcSessions.length > 0 ? `WC (${wcSessions.length})` : "WalletConnect"}
          icon="🔗"
          onClick={() => setView("walletconnect")}
          badge={wcSessions.length > 0 ? wcSessions.length : undefined}
        />
        <QuickAction
          label={ledgerConnected ? "Ledger" : "Hardware"}
          icon="🔐"
          onClick={() => setView("ledger")}
          badge={ledgerConnected ? 1 : undefined}
        />
        <QuickAction label="Plugins" icon="+" onClick={() => setView("plugins")} />
        <QuickAction label="AI" icon=">" onClick={() => setView("ai-assistant")} />
      </div>

      {/* Chain status */}
      {chainStatus && (
        <div className="mx-4 mb-4 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <div className="flex items-center gap-1 mb-2">
            <div className="w-1.5 h-1.5 rounded-full bg-evap-green" />
            <span className="text-[10px] text-zinc-400">Testnet Connected</span>
          </div>
          <div className="grid grid-cols-3 gap-2">
            <Stat label="Block" value={chainStatus.block_height.toLocaleString()} />
            <Stat label="Objects" value={chainStatus.active_objects.toLocaleString()} />
            <Stat label="Ghosts" value={chainStatus.ghost_count.toLocaleString()} />
          </div>
        </div>
      )}

      {/* Post-quantum badge */}
      <div className="mt-auto">
        <QuantumBadge />
      </div>

      {/* Bottom nav */}
      <div className="flex border-t border-evap-border">
        <NavBtn label="Home" active onClick={() => setView("home")} />
        <NavBtn label="Objects" onClick={() => { setView("objects"); refreshObjects(); }} />
        <NavBtn label="Activity" onClick={() => setView("activity")} />
        <NavBtn label="Settings" onClick={() => setView("settings")} />
      </div>
    </div>
  );
}

function QuickAction({ label, icon, onClick, disabled, badge }: {
  label: string; icon: string; onClick: () => void; disabled?: boolean; badge?: number;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="relative flex flex-col items-center gap-1 py-3 rounded-lg bg-evap-surface border border-evap-border hover:border-evap-cyan/40 transition disabled:opacity-50"
    >
      <span className="text-lg">{icon}</span>
      <span className="text-[10px] text-zinc-400">{label}</span>
      {badge !== undefined && badge > 0 && (
        <span className="absolute -top-1 -right-1 w-4 h-4 rounded-full bg-evap-amber text-[9px] font-bold text-black flex items-center justify-center">
          {badge}
        </span>
      )}
    </button>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="text-center">
      <p className="text-xs font-semibold text-zinc-200">{value}</p>
      <p className="text-[10px] text-zinc-500">{label}</p>
    </div>
  );
}

function NavBtn({ label, active, onClick }: { label: string; active?: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className={`flex-1 py-3 text-[10px] font-medium transition ${
        active ? "text-evap-cyan" : "text-zinc-500 hover:text-zinc-300"
      }`}
    >
      {label}
    </button>
  );
}

import { useEffect } from "react";
import { useWallet } from "@/hooks/useWallet";
import { formatBalance, shortAddress } from "@/utils/format";
import { Header } from "./Header";

export function HomeScreen() {
  const {
    activeAccount, balance, chainStatus,
    setView, claimFaucet, refreshBalance, refreshObjects,
    loading, notification, setNotification,
  } = useWallet();

  useEffect(() => {
    refreshBalance();
    const interval = setInterval(refreshBalance, 10_000);
    return () => clearInterval(interval);
  }, [refreshBalance]);

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
      <div className="grid grid-cols-3 gap-2 px-4 pb-2">
        <QuickAction label="Send" icon="↑" onClick={() => setView("send")} />
        <QuickAction label="Receive" icon="↓" onClick={() => setView("receive")} />
        <QuickAction label="Swap" icon="⇄" onClick={() => setView("swap")} />
      </div>
      <div className="grid grid-cols-4 gap-2 px-4 pb-4">
        <QuickAction label="Buy" icon="$" onClick={() => setView("buy")} />
        <QuickAction label="Objects" icon="◈" onClick={() => { setView("objects"); refreshObjects(); }} />
        <QuickAction label="NFTs" icon="🖼" onClick={() => setView("nfts")} />
        <QuickAction
          label="Faucet"
          icon="💧"
          onClick={claimFaucet}
          disabled={loading}
        />
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
      <div className="mt-auto px-4 pb-4">
        <div className="px-3 py-2 rounded-lg bg-evap-purple/10 border border-evap-purple/20">
          <p className="text-[10px] text-evap-purple text-center">
            🛡️ Post-Quantum Secured · ML-DSA (FIPS 204)
          </p>
        </div>
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

function QuickAction({ label, icon, onClick, disabled }: {
  label: string; icon: string; onClick: () => void; disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="flex flex-col items-center gap-1 py-3 rounded-lg bg-evap-surface border border-evap-border hover:border-evap-cyan/40 transition disabled:opacity-50"
    >
      <span className="text-lg">{icon}</span>
      <span className="text-[10px] text-zinc-400">{label}</span>
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

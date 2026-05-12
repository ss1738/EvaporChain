import { useEffect, useState } from "react";
import type { LucideIcon } from "lucide-react";
import {
  ArrowUp, ArrowDown, ArrowLeftRight, Workflow,
  DollarSign, Box, Image as ImageIcon, Zap, PieChart, Droplet,
  RefreshCw, Ghost, TrendingDown, Link as LinkIcon, Usb,
  Puzzle, Sparkles, ChevronDown, ChevronUp,
} from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import { formatBalance } from "@/utils/format";
import { Header } from "./Header";
import { FeeControllerWidget } from "./FeeControllerWidget";
import { DemurrageBadge } from "./DemurrageBadge";
import { DsnBadge } from "./DsnBadge";

export function HomeScreen() {
  const {
    activeAccount, balance, chainStatus, ghosts, wcSessions,
    ledgerConnected,
    setView, claimFaucet, refreshBalance, refreshObjects, refreshGhosts,
    refreshChainStatus, refreshFeeStatus,
    loading, notification, setNotification,
  } = useWallet();

  const recoverableGhosts = ghosts.filter(g => g.proof_status !== "expired").length;
  const [moreOpen, setMoreOpen] = useState(false);

  useEffect(() => {
    refreshBalance();
    refreshGhosts();
    refreshChainStatus();
    refreshFeeStatus();
    const balInterval = setInterval(refreshBalance, 10_000);
    // Substrate-status polling cadence: chain status + fee controller
    // every 10s. Cheap reads, both endpoints are pure compute.
    const chainInterval = setInterval(() => {
      refreshChainStatus();
      refreshFeeStatus();
    }, 10_000);
    return () => {
      clearInterval(balInterval);
      clearInterval(chainInterval);
    };
  }, [refreshBalance, refreshGhosts, refreshChainStatus, refreshFeeStatus]);

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
      {/* TODO: enable when EvaporChain Ledger BOLOS app ships */}
      {import.meta.env.DEV && ledgerConnected && (
        <div className="mx-4 mt-2 px-3 py-1.5 rounded-lg bg-blue-500/10 border border-blue-500/30 flex items-center gap-2">
          <div className="w-2 h-2 rounded-full bg-blue-500" />
          <span className="text-xs text-blue-400 font-medium">Hardware Wallet Connected</span>
          <button
            onClick={() => setView("ledger")}
            className="ml-auto text-xs text-blue-400 hover:text-blue-300 transition"
          >
            Manage
          </button>
        </div>
      )}

      {/* Balance — single source of truth. Address lives in the
          Header pill; no need to repeat the full hex here. */}
      <div className="px-5 pt-8 pb-6">
        <div className="flex items-center justify-between mb-2">
          <p className="text-xs font-medium text-zinc-500 uppercase tracking-wide">Balance</p>
          <DemurrageBadge />
        </div>
        <div className="flex items-baseline gap-2">
          <span className="text-4xl font-semibold text-zinc-100 tabular-nums">
            {formatBalance(balance)}
          </span>
          <span className="text-base text-zinc-500">EVAP</span>
        </div>
      </div>

      {/* Primary actions — 4 only. Anything secondary is in More. */}
      <div className="grid grid-cols-4 gap-2 px-4 pb-3">
        <QuickAction label="Send" icon={ArrowUp} onClick={() => setView("send")} />
        <QuickAction label="Receive" icon={ArrowDown} onClick={() => setView("receive")} />
        <QuickAction label="Swap" icon={ArrowLeftRight} onClick={() => setView("swap")} />
        <QuickAction label="Buy" icon={DollarSign} onClick={() => setView("buy")} />
      </div>

      {/* More — collapsed by default. 14 secondary actions in a
          consistent 4-col grid so visual rhythm doesn't break. */}
      <div className="px-4 pb-3">
        <button
          onClick={() => setMoreOpen(o => !o)}
          className="w-full flex items-center justify-between px-3 py-2 rounded-lg bg-evap-surface border border-evap-border hover:border-evap-cyan/40 transition"
        >
          <span className="text-sm text-zinc-300">More</span>
          {moreOpen
            ? <ChevronUp className="w-4 h-4 text-zinc-500" strokeWidth={1.5} />
            : <ChevronDown className="w-4 h-4 text-zinc-500" strokeWidth={1.5} />
          }
        </button>
        {moreOpen && (
          <div className="grid grid-cols-4 gap-2 mt-2">
            <QuickAction label="Bridge" icon={Workflow} onClick={() => setView("bridge")} />
            <QuickAction label="Objects" icon={Box} onClick={() => { setView("objects"); refreshObjects(); }} />
            <QuickAction label="NFTs" icon={ImageIcon} onClick={() => setView("nfts")} />
            <QuickAction label="Energy" icon={Zap} onClick={() => setView("energy-dashboard")} />
            <QuickAction label="Portfolio" icon={PieChart} onClick={() => setView("portfolio")} />
            <QuickAction label="Faucet" icon={Droplet} onClick={claimFaucet} disabled={loading} />
            <QuickAction label="Refresh" icon={RefreshCw} onClick={() => { setView("batch-refresh"); refreshObjects(); }} />
            <QuickAction
              label="Ghosts"
              icon={Ghost}
              onClick={() => { setView("ghost-recovery"); refreshGhosts(); }}
              badge={recoverableGhosts > 0 ? recoverableGhosts : undefined}
            />
            <QuickAction label="Forecast" icon={TrendingDown} onClick={() => { setView("decay-forecast"); refreshObjects(); }} />
            <QuickAction
              label="WC"
              icon={LinkIcon}
              onClick={() => setView("walletconnect")}
              badge={wcSessions.length > 0 ? wcSessions.length : undefined}
            />
            {import.meta.env.DEV && (
              <QuickAction
                label="Hardware"
                icon={Usb}
                onClick={() => setView("ledger")}
                badge={ledgerConnected ? 1 : undefined}
              />
            )}
            <QuickAction label="Plugins" icon={Puzzle} onClick={() => setView("plugins")} />
            <QuickAction label="AI" icon={Sparkles} onClick={() => setView("ai-assistant")} />
          </div>
        )}
      </div>

      {/* Chain status — only render when we have actual data, not "—". */}
      {chainStatus && chainStatus.block_height > 0 && (
        <div className="mx-4 mb-3 px-4 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <div className="flex items-center gap-2 mb-2">
            <div className="w-1.5 h-1.5 rounded-full bg-evap-green" />
            <span className="text-xs text-zinc-400">Testnet</span>
          </div>
          <div className="grid grid-cols-3 gap-3">
            <Stat label="Block" value={chainStatus.block_height.toLocaleString()} />
            <Stat label="Objects" value={chainStatus.active_objects.toLocaleString()} />
            <Stat label="Ghosts" value={chainStatus.ghost_count.toLocaleString()} />
          </div>
        </div>
      )}

      {/* Advanced — fee controller + DSN privacy. Only render the
          fee widget when there's real data, not a "—" placeholder.
          The DsnBadge handles its own zero-state gracefully. */}
      {chainStatus && chainStatus.block_height > 0 && (
        <>
          <FeeControllerWidget />
          <DsnBadge />
        </>
      )}

      <div className="mt-auto" />

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

function QuickAction({ label, icon: Icon, onClick, disabled, badge }: {
  label: string; icon: LucideIcon; onClick: () => void; disabled?: boolean; badge?: number;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="relative flex flex-col items-center gap-2 py-3.5 rounded-lg bg-evap-surface border border-evap-border hover:border-evap-cyan/40 hover:bg-zinc-800/40 transition disabled:opacity-50"
    >
      <Icon className="w-[18px] h-[18px] text-zinc-200" strokeWidth={1.5} />
      <span className="text-xs text-zinc-400 font-medium">{label}</span>
      {badge !== undefined && badge > 0 && (
        <span className="absolute top-1.5 right-1.5 min-w-[16px] h-4 px-1 rounded-full bg-evap-amber text-xs font-semibold text-black flex items-center justify-center">
          {badge}
        </span>
      )}
    </button>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="text-sm font-semibold text-zinc-100 tabular-nums">{value}</p>
      <p className="text-xs text-zinc-500 mt-0.5">{label}</p>
    </div>
  );
}

function NavBtn({ label, active, onClick }: { label: string; active?: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className={`flex-1 py-3 text-xs font-medium transition ${
        active ? "text-evap-cyan" : "text-zinc-500 hover:text-zinc-300"
      }`}
    >
      {label}
    </button>
  );
}

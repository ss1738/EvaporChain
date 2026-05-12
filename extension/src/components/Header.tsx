import { Lock } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import { shortAddress } from "@/utils/format";

export function Header() {
  const {
    activeAccount, chainStatus, lock, pendingTxs, setView,
    shardsHealth, addressShard, preferences,
  } = useWallet();
  const network = preferences.network;
  const networkPill =
    network === "mainnet"
      ? "bg-evap-green/10 text-evap-green border-evap-green/30 hover:border-evap-green/60"
      : network === "testnet"
        ? "bg-amber-500/10 text-amber-500 border-amber-500/30 hover:border-amber-500/60"
        : "bg-zinc-700/40 text-zinc-300 border-zinc-600/40 hover:border-zinc-500/60";
  const inflight = pendingTxs.filter(t => t.status !== "finalised" && t.status !== "rejected").length;
  const showShardPill =
    shardsHealth?.active === true &&
    shardsHealth.total_shards > 1 &&
    addressShard != null;

  // Single deterministic 2-letter identicon from the address. Stable
  // across sessions; better-than-first-letter visual distinction.
  const initials = activeAccount?.address
    ? (activeAccount.address.slice(2, 3) + activeAccount.address.slice(-1)).toUpperCase()
    : "EC";

  return (
    <div className="flex items-center justify-between px-4 py-3 border-b border-evap-border">
      {/* Left: avatar + name + address */}
      <button
        onClick={() => setView("settings")}
        className="flex items-center gap-2.5 group"
        title="Account settings"
      >
        <div className="w-8 h-8 rounded-full bg-gradient-to-br from-evap-cyan/90 to-evap-purple/90 flex items-center justify-center text-xs font-semibold text-black ring-1 ring-evap-cyan/20 group-hover:ring-evap-cyan/40 transition">
          {initials}
        </div>
        <div className="text-left">
          <div className="text-sm font-medium text-zinc-100 leading-tight">
            {activeAccount?.name ?? "EvaporChain"}
          </div>
          <div className="text-xs text-zinc-500 font-mono leading-tight mt-0.5">
            {activeAccount ? shortAddress(activeAccount.address) : ""}
          </div>
        </div>
      </button>

      {/* Right: status pills + lock */}
      <div className="flex items-center gap-1.5">
        {showShardPill && (
          <button
            onClick={() => setView("shards")}
            className="text-xs px-2 py-1 rounded-md bg-evap-cyan/10 text-evap-cyan border border-evap-cyan/30 hover:border-evap-cyan/60 transition"
            title="Shard health"
          >
            S{addressShard}
          </button>
        )}
        <button
          onClick={() => setView("settings")}
          className={`text-xs px-2 py-1 rounded-md border capitalize transition ${networkPill}`}
          title="Network — tap to change"
        >
          {network}
        </button>
        {inflight > 0 && (
          <button
            onClick={() => setView("activity")}
            className="flex items-center gap-1.5 px-2 py-1 rounded-md border border-evap-cyan/40 bg-evap-cyan/10 text-xs text-evap-cyan hover:border-evap-cyan/70 transition"
            title="Pending transactions"
          >
            <span className="w-1.5 h-1.5 rounded-full bg-evap-cyan animate-pulse" />
            {inflight}
          </button>
        )}
        {chainStatus && chainStatus.block_height > 0 && !inflight && (
          <span
            className="hidden sm:flex items-center gap-1.5 text-xs text-zinc-500"
            title={`Block ${chainStatus.block_height}`}
          >
            <span className="w-1.5 h-1.5 rounded-full bg-evap-green animate-pulse" />
            #{chainStatus.block_height}
          </span>
        )}
        <button
          onClick={lock}
          className="p-1.5 rounded-md text-zinc-500 hover:text-zinc-200 hover:bg-evap-surface transition"
          title="Lock wallet"
          aria-label="Lock wallet"
        >
          <Lock className="w-4 h-4" strokeWidth={1.5} />
        </button>
      </div>
    </div>
  );
}

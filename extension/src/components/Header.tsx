import { useWallet } from "@/hooks/useWallet";
import { shortAddress } from "@/utils/format";

export function Header() {
  const {
    activeAccount, chainStatus, lock, pendingTxs, setView,
    shardsHealth, addressShard, preferences,
  } = useWallet();
  const network = preferences.network;
  // Mainnet = green pill, Testnet = amber pill, Custom = gray pill.
  const networkPill =
    network === "mainnet"
      ? "bg-evap-green/10 text-evap-green border-evap-green/30 hover:border-evap-green/60"
      : network === "testnet"
        ? "bg-amber-500/10 text-amber-500 border-amber-500/30 hover:border-amber-500/60"
        : "bg-zinc-500/10 text-zinc-300 border-zinc-500/30 hover:border-zinc-500/60";
  // Count txs that are still in flight (i.e. not yet finalised/rejected).
  const inflight = pendingTxs.filter(t => t.status !== "finalised" && t.status !== "rejected").length;
  // Show the shard pill only when sharding is active and the chain
  // has more than one shard — single-shard chains don't need to
  // distract the user with shard info.
  const showShardPill =
    shardsHealth?.active === true &&
    shardsHealth.total_shards > 1 &&
    addressShard != null;

  return (
    <div className="flex items-center justify-between px-4 py-3 border-b border-evap-border">
      <div className="flex items-center gap-2">
        <div className="w-7 h-7 rounded-full bg-gradient-to-br from-evap-cyan to-evap-purple flex items-center justify-center text-[10px] font-bold text-black">
          E
        </div>
        <div>
          <div className="flex items-center gap-1.5">
            <div className="text-xs font-semibold text-zinc-200">
              {activeAccount?.name ?? "EvaporChain"}
            </div>
            {showShardPill && (
              <button
                onClick={() => setView("shards")}
                className="text-[9px] px-1.5 py-0.5 rounded-full bg-evap-cyan/10 text-evap-cyan border border-evap-cyan/30 hover:border-evap-cyan/60 transition"
                title="View shard health"
              >
                Shard {addressShard}
              </button>
            )}
          </div>
          <div className="text-[10px] text-zinc-500 font-mono">
            {activeAccount ? shortAddress(activeAccount.address) : ""}
          </div>
        </div>
      </div>
      <div className="flex items-center gap-2">
        <button
          onClick={() => setView("settings")}
          className={`text-[9px] px-1.5 py-0.5 rounded-full border capitalize transition ${networkPill}`}
          title="Network — tap to change in settings"
        >
          {network}
        </button>
        {inflight > 0 && (
          <button
            onClick={() => setView("activity")}
            className="flex items-center gap-1 px-2 py-0.5 rounded-full border border-evap-cyan/40 bg-evap-cyan/10 text-[10px] text-evap-cyan hover:border-evap-cyan/70 transition"
            title="View pending transactions"
          >
            <span className="w-1.5 h-1.5 rounded-full bg-evap-cyan animate-pulse" />
            {inflight} pending
          </button>
        )}
        {chainStatus && (
          <div className="flex items-center gap-1">
            <div className="w-1.5 h-1.5 rounded-full bg-evap-green animate-pulse" />
            <span className="text-[10px] text-zinc-500">
              Block {chainStatus.block_height}
              {shardsHealth?.active && shardsHealth.total_shards > 0 && (
                <> · {shardsHealth.total_shards} shard{shardsHealth.total_shards === 1 ? "" : "s"}</>
              )}
            </span>
          </div>
        )}
        <button
          onClick={lock}
          className="text-[10px] text-zinc-500 hover:text-zinc-300 px-2 py-1 rounded hover:bg-evap-surface transition"
          title="Lock wallet"
        >
          🔒
        </button>
      </div>
    </div>
  );
}

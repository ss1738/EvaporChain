import { useWallet } from "@/hooks/useWallet";
import { shortAddress } from "@/utils/format";

export function Header() {
  const { activeAccount, chainStatus, lock, pendingTxs, setView } = useWallet();
  // Count txs that are still in flight (i.e. not yet finalised/rejected).
  const inflight = pendingTxs.filter(t => t.status !== "finalised" && t.status !== "rejected").length;

  return (
    <div className="flex items-center justify-between px-4 py-3 border-b border-evap-border">
      <div className="flex items-center gap-2">
        <div className="w-7 h-7 rounded-full bg-gradient-to-br from-evap-cyan to-evap-purple flex items-center justify-center text-[10px] font-bold text-black">
          E
        </div>
        <div>
          <div className="text-xs font-semibold text-zinc-200">
            {activeAccount?.name ?? "EvaporChain"}
          </div>
          <div className="text-[10px] text-zinc-500 font-mono">
            {activeAccount ? shortAddress(activeAccount.address) : ""}
          </div>
        </div>
      </div>
      <div className="flex items-center gap-2">
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
            <span className="text-[10px] text-zinc-500">Block {chainStatus.block_height}</span>
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

import { useEffect, useState } from "react";
import { useWallet, type PendingTx } from "@/hooks/useWallet";
import { Header } from "./Header";
import { shortAddress, timeAgo } from "@/utils/format";
import { api } from "@/utils/api";

interface ActivityItem {
  type: string;
  detail: string;
  hash?: string;
  timestamp?: string;
}

export function ActivityScreen() {
  const { activeAccount, setView, pendingTxs, clearTx } = useWallet();
  const [transactions, setTransactions] = useState<ActivityItem[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!activeAccount) return;
    let cancelled = false;

    const fetchActivity = async () => {
      setLoading(true);
      try {
        const txs = await api.getTransactions();
        if (!cancelled) {
          // Filter to transactions involving this account
          const addr = activeAccount.address.toLowerCase();
          const relevant = txs.filter(tx =>
            tx.detail.toLowerCase().includes(addr) ||
            tx.detail.toLowerCase().includes(addr.slice(2)) // without 0x
          );
          setTransactions(relevant.map(tx => ({
            ...tx,
            timestamp: new Date().toISOString(), // API doesn't return timestamps per tx yet
          })));
        }
      } catch {
        if (!cancelled) setTransactions([]);
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    fetchActivity();
    const interval = setInterval(fetchActivity, 15_000);
    return () => { cancelled = true; clearInterval(interval); };
  }, [activeAccount]);

  const txIcon = (type: string) => {
    switch (type.toLowerCase()) {
      case "transfer": return "↗";
      case "receive": return "↙";
      case "refresh": return "⟳";
      case "createobject": return "◈";
      case "faucet": return "💧";
      case "deploycontract": return "📄";
      case "callcontract": return "⚡";
      case "validatorstake": return "🔒";
      case "validatorexit": return "🔓";
      default: return "•";
    }
  };

  const txColor = (type: string) => {
    switch (type.toLowerCase()) {
      case "transfer": return "text-evap-red";
      case "receive": return "text-evap-green";
      case "refresh": return "text-evap-cyan";
      case "createobject": return "text-evap-purple";
      case "faucet": return "text-evap-cyan";
      default: return "text-zinc-400";
    }
  };

  return (
    <div className="flex flex-col h-full">
      <Header />
      <div className="px-4 pt-4 pb-2 flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold text-zinc-100">Activity</h2>
          <p className="text-xs text-zinc-500">
            {transactions.length} transaction{transactions.length !== 1 ? "s" : ""}
          </p>
        </div>
        <button
          onClick={() => setView("home")}
          className="text-xs text-zinc-500 hover:text-zinc-300"
        >
          ← Back
        </button>
      </div>

      <div className="flex-1 overflow-y-auto px-4 pb-4">
        {pendingTxs.length > 0 && (
          <div className="mb-3">
            <p className="text-[10px] text-zinc-500 uppercase tracking-wider mb-1.5 px-1">
              Live transactions
            </p>
            <div className="space-y-1">
              {pendingTxs.map((tx) => (
                <PendingTxRow key={tx.hash} tx={tx} onClear={() => clearTx(tx.hash)} />
              ))}
            </div>
          </div>
        )}
        {loading ? (
          <div className="flex items-center justify-center py-16">
            <div className="w-6 h-6 border-2 border-evap-cyan/30 border-t-evap-cyan rounded-full animate-spin" />
          </div>
        ) : transactions.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-12">
            <span className="text-3xl mb-3">📋</span>
            <p className="text-sm text-zinc-500">No transactions yet</p>
            <p className="text-xs text-zinc-600 mt-1">Send EVAP or claim from faucet to get started</p>
          </div>
        ) : (
          <div className="space-y-1">
            {transactions.map((tx, i) => (
              <div
                key={tx.hash ?? i}
                className="flex items-center gap-3 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border hover:border-evap-border/80 transition"
              >
                {/* Icon */}
                <div className={`w-8 h-8 rounded-full bg-evap-bg flex items-center justify-center text-base ${txColor(tx.type)}`}>
                  {txIcon(tx.type)}
                </div>

                {/* Details */}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center justify-between">
                    <p className="text-xs font-medium text-zinc-200 capitalize">{tx.type}</p>
                    {tx.timestamp && (
                      <span className="text-[10px] text-zinc-600">{timeAgo(tx.timestamp)}</span>
                    )}
                  </div>
                  <p className="text-[10px] text-zinc-500 truncate mt-0.5">{tx.detail}</p>
                  {tx.hash && (
                    <p className="text-[10px] text-zinc-600 font-mono mt-0.5">
                      {shortAddress(tx.hash)}
                    </p>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Bottom nav */}
      <div className="flex border-t border-evap-border">
        <NavBtn label="Home" onClick={() => setView("home")} />
        <NavBtn label="Objects" onClick={() => setView("objects")} />
        <NavBtn label="Activity" active onClick={() => {}} />
        <NavBtn label="Settings" onClick={() => setView("settings")} />
      </div>
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

function PendingTxRow({ tx, onClear }: { tx: PendingTx; onClear: () => void }) {
  const badge = (() => {
    switch (tx.status) {
      case "pending":   return { label: "Pending",   cls: "bg-zinc-700/40 text-zinc-300 border-zinc-600/40" };
      case "mempool":   return { label: "Mempool",   cls: "bg-evap-amber/10 text-evap-amber border-evap-amber/30" };
      case "included":  return { label: "Included",  cls: "bg-evap-cyan/10 text-evap-cyan border-evap-cyan/30" };
      case "finalised": return { label: "Finalised", cls: "bg-evap-green/10 text-evap-green border-evap-green/30" };
      case "rejected":  return { label: "Rejected",  cls: "bg-evap-red/10 text-evap-red border-evap-red/30" };
    }
  })();
  const showSpinner = tx.status === "pending" || tx.status === "mempool" || tx.status === "included";
  return (
    <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-evap-surface border border-evap-border">
      <div className="w-6 h-6 shrink-0 flex items-center justify-center">
        {showSpinner ? (
          <div className="w-3 h-3 border-2 border-evap-cyan/30 border-t-evap-cyan rounded-full animate-spin" />
        ) : tx.status === "finalised" ? (
          <span className="text-evap-green text-sm">✓</span>
        ) : (
          <span className="text-evap-red text-sm">✗</span>
        )}
      </div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center justify-between gap-2">
          <span className="text-[11px] font-medium text-zinc-200 truncate">{tx.summary}</span>
          <span className={`text-[9px] px-1.5 py-0.5 rounded-full border shrink-0 ${badge.cls}`}>{badge.label}</span>
        </div>
        <div className="flex items-center justify-between mt-0.5 gap-2">
          <span className="text-[9px] text-zinc-600 font-mono truncate">{shortAddress(tx.hash)}</span>
          <span className="text-[9px] text-zinc-600 shrink-0">
            {tx.blockHeight != null ? `Block ${tx.blockHeight}` : ""}
          </span>
        </div>
        {tx.error && (
          <p className="text-[9px] text-evap-red mt-0.5 truncate">{tx.error}</p>
        )}
      </div>
      {(tx.status === "rejected" || tx.status === "finalised") && (
        <button
          onClick={onClear}
          className="text-[9px] text-zinc-500 hover:text-zinc-300 px-1 shrink-0"
          title="Dismiss"
        >
          ×
        </button>
      )}
    </div>
  );
}

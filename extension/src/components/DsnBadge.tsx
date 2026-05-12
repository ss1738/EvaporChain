import { useEffect, useState } from "react";
import { RotateCw } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";

/**
 * DSN — Decay-Stamped Nullifiers privacy badge.
 *
 * Compact card showing the size of the current DSN window's anonymity
 * set: the more nullifiers folded into the accumulator, the larger the
 * privacy set a shielded transfer hides within. The aggregate root is
 * the blake3-folded summary of every nullifier in the window — useful
 * for verifying that two clients see the same window state.
 *
 * Endpoint:
 *   GET /api/dsn/status — api.rs L5205 — returns
 *     { status, total_count, aggregate_root_hex } (serde_json::Value).
 *
 * Field names match api.rs §post_dsn_fold_nullifier / §get_dsn_status
 * (L5188-L5211): total_count, aggregate_root_hex.
 *
 * Read-only. No signing. Mounted from HomeScreen below FeeControllerWidget.
 * Refresh is driven by useWallet.init() and pollTxStatuses() finalisation
 * — this component just consumes the cached store value.
 */
export function DsnBadge() {
  const { dsnStatus, refreshDsnStatus, setView } = useWallet();
  const [hasMounted, setHasMounted] = useState(false);

  useEffect(() => {
    if (!hasMounted) {
      setHasMounted(true);
      refreshDsnStatus();
    }
  }, [hasMounted, refreshDsnStatus]);

  const total = dsnStatus?.total_count ?? 0;
  const isLargeSet = total > 100;
  const root = dsnStatus?.aggregate_root_hex ?? "";
  const shortRoot = root ? `${root.slice(0, 8)}…${root.slice(-6)}` : "—";

  return (
    <div className="mx-4 mt-2 px-3 py-2.5 rounded-lg bg-evap-surface border border-evap-border">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <span
            className={`text-[10px] font-semibold px-2 py-0.5 rounded-full border shrink-0 ${
              isLargeSet
                ? "bg-evap-purple/15 text-evap-purple border-evap-purple/40"
                : "bg-zinc-700/40 text-zinc-400 border-zinc-600/40"
            }`}
          >
            DSN
          </span>
          <p className="text-xs font-medium text-zinc-200 truncate">
            Privacy set: <span className="text-zinc-100 font-bold">{total.toLocaleString()}</span>{" "}
            <span className="text-zinc-500">nullifier{total === 1 ? "" : "s"}</span>
          </p>
        </div>
        <button
          onClick={() => setView("dsn-details")}
          className="text-[10px] text-evap-purple hover:text-evap-purple/80 transition shrink-0"
        >
          Details<ChevronRight className="inline w-3.5 h-3.5 -mt-0.5" strokeWidth={1.5} />
        </button>
      </div>
      <p className="text-xs text-zinc-500 mt-1 leading-snug">
        Your shielded transfers are anonymous within this set.
      </p>
      <div className="flex items-center justify-between mt-1.5">
        <p className="text-[10px] text-zinc-600 uppercase tracking-wider">Root</p>
        <p
          className="text-[10px] text-zinc-400 font-mono"
          title={root || undefined}
        >
          {shortRoot}
        </p>
      </div>
    </div>
  );
}

/**
 * Full-screen DSN details view, reachable from DsnBadge → "Details<ChevronRight className="inline w-3.5 h-3.5 -mt-0.5" strokeWidth={1.5} />".
 * Surfaces the full aggregate root and lets the user manually re-pull
 * the status so they can confirm that two wallet instances agree on the
 * same window. Window epoch is not directly exposed by the endpoint —
 * the response returns total_count + aggregate_root_hex only — so we
 * surface the chain epoch as the closest contextual signal.
 */
export function DsnDetailsScreen() {
  const { dsnStatus, refreshDsnStatus, chainStatus, setView } = useWallet();

  useEffect(() => {
    refreshDsnStatus();
  }, [refreshDsnStatus]);

  const total = dsnStatus?.total_count ?? 0;
  const root = dsnStatus?.aggregate_root_hex ?? "—";
  const epoch = chainStatus?.epoch ?? 0;

  return (
    <div className="flex flex-col h-full bg-evap-bg">
      <div className="flex items-center gap-3 px-4 py-3 border-b border-evap-border">
        <button
          onClick={() => setView("home")}
          className="text-zinc-400 hover:text-zinc-200 transition text-sm"
        >
          ←
        </button>
        <div className="flex-1">
          <h1 className="text-sm font-semibold text-zinc-200">Privacy Window</h1>
          <p className="text-xs text-zinc-500">
            Decay-Stamped Nullifiers — current window state.
          </p>
        </div>
        <button
          onClick={() => refreshDsnStatus()}
          className="text-xs px-2 py-1 rounded text-evap-purple border border-evap-purple/40 hover:border-evap-purple/70 transition"
        ><RotateCw className="w-3.5 h-3.5" strokeWidth={1.5} /></button>
      </div>

      <div className="flex-1 overflow-y-auto p-4 space-y-3">
        <div className="px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <p className="text-xs text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
            Anonymity Set
          </p>
          <div className="flex items-baseline gap-2">
            <span className="text-2xl font-bold text-zinc-100">
              {total.toLocaleString()}
            </span>
            <span className="text-xs text-zinc-500">nullifiers</span>
          </div>
          <p className="text-xs text-zinc-500 mt-1.5 leading-snug">
            Every shielded transfer in this window is indistinguishable
            from any other within this set.
          </p>
        </div>

        <div className="px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <p className="text-xs text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
            Aggregate Root (blake3)
          </p>
          <p className="text-xs text-zinc-300 font-mono break-all leading-relaxed">
            {root}
          </p>
          <p className="text-xs text-zinc-500 mt-2 leading-snug">
            Two wallets seeing the same window will compute the same
            root. Use this to verify state agreement out-of-band.
          </p>
        </div>

        <div className="px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <div className="flex items-center justify-between">
            <p className="text-xs text-zinc-500 uppercase tracking-wider">
              Chain epoch
            </p>
            <p className="text-xs text-zinc-300 font-mono">{epoch}</p>
          </div>
          <p className="text-xs text-zinc-500 mt-1.5 leading-snug">
            The window slides forward via /api/dsn/advance_window —
            advancing drops the oldest accumulator slot.
          </p>
        </div>
      </div>
    </div>
  );
}

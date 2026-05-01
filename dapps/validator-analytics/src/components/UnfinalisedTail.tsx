import type { FinalityGap } from "@/lib/api";

/** Heights committed locally but not yet finalised. Worst gap drives the
 *  `EvapFinalityStalled` alert (alertmanager fires above ~30s). */
export function UnfinalisedTail({ gaps }: { gaps: FinalityGap }) {
  const sorted = [...gaps.unfinalised].sort((a, b) => a.height - b.height);
  const stalled = gaps.worst_gap_seconds > 30;
  return (
    <div className="rounded-xl border border-evap-border bg-white p-4">
      <div className="flex items-center justify-between">
        <p className="text-xs font-semibold text-zinc-900">Unfinalised Tail</p>
        <span
          className={`rounded-md px-2 py-0.5 text-[10px] font-semibold ${
            stalled ? "bg-red-50 text-evap-red" : sorted.length > 0 ? "bg-amber-50 text-evap-amber" : "bg-green-50 text-evap-green"
          }`}
        >
          {stalled ? "stalled" : sorted.length > 0 ? "lagging" : "live"}
        </span>
      </div>
      <p className="mt-2 text-[10px] text-zinc-500">
        Worst gap: <span className="font-mono text-zinc-800">{gaps.worst_gap_seconds.toFixed(2)}s</span>{" "}
        · Heights pending: <span className="font-mono text-zinc-800">{sorted.length}</span>
      </p>
      {sorted.length > 0 && (
        <div className="mt-3 max-h-40 overflow-y-auto">
          <table className="w-full text-[11px]">
            <thead>
              <tr className="text-left text-[9px] uppercase text-zinc-400">
                <th className="py-1">Height</th>
                <th className="py-1 text-right">Age</th>
              </tr>
            </thead>
            <tbody>
              {sorted.map((u) => (
                <tr key={u.height} className="border-t border-evap-border/60">
                  <td className="py-1 font-mono">{u.height}</td>
                  <td className="py-1 text-right font-mono">
                    <span className={u.age_seconds > 30 ? "text-evap-red" : u.age_seconds > 5 ? "text-evap-amber" : "text-zinc-700"}>
                      {u.age_seconds.toFixed(1)}s
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

import { fmtEvap } from "@/lib/api";

/**
 * Cumulative slash view.
 *
 * The chain exposes `total_slashed` per validator on /api/validators but no
 * event-by-event slash log endpoint. POST /api/validators/sanov_slash records
 * a slash but there's no GET counterpart. So this surfaces the cumulative
 * figure with a "no event stream" notice, which is honest about the gap.
 */
export function SlashTimeline({ totalSlashed }: { totalSlashed: number }) {
  return (
    <div className="rounded-xl border border-evap-border bg-white p-4">
      <p className="text-xs font-semibold text-zinc-900">Slash History</p>
      {totalSlashed > 0 ? (
        <div className="mt-2 flex items-baseline gap-2">
          <p className="text-2xl font-bold text-evap-red">{fmtEvap(totalSlashed)}</p>
          <p className="text-[10px] text-zinc-500">EVAP cumulatively slashed</p>
        </div>
      ) : (
        <p className="mt-2 text-xs text-evap-green">No slashes recorded.</p>
      )}
      <p className="mt-3 text-[10px] text-zinc-400">
        Per-event slash timeline (severity, reason, height) is not exposed by the node REST API
        as of this build. Only the cumulative `total_slashed` field is available.
      </p>
    </div>
  );
}

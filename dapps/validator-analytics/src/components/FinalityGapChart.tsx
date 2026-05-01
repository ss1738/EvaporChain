import type { FinalityGap } from "@/lib/api";

const BUCKETS: Array<{ ub: number; label: string }> = [
  { ub: 0.5, label: "≤0.5s" },
  { ub: 1, label: "≤1s" },
  { ub: 2, label: "≤2s" },
  { ub: 5, label: "≤5s" },
  { ub: 10, label: "≤10s" },
  { ub: 30, label: "≤30s" },
  { ub: 60, label: "≤60s" },
  { ub: Infinity, label: ">60s" },
];

/** Histogram of recent commit→finalise gaps. Bucket bounds match the
 *  Prometheus exposition in api.rs:11898 (0.5/1/2/5/10/30/60/300/1800). */
export function FinalityGapChart({ gaps }: { gaps: FinalityGap }) {
  const counts = BUCKETS.map(() => 0);
  for (const g of gaps.recent_gaps) {
    let placed = false;
    for (let i = 0; i < BUCKETS.length; i++) {
      if (g.gap_seconds <= BUCKETS[i].ub) {
        counts[i]++;
        placed = true;
        break;
      }
    }
    if (!placed) counts[counts.length - 1]++;
  }
  const max = Math.max(1, ...counts);
  return (
    <div className="rounded-xl border border-evap-border bg-white p-4">
      <p className="text-xs font-semibold text-zinc-900">Recent Finality Gaps ({gaps.recent_gaps.length} samples)</p>
      <div className="mt-3 grid grid-cols-8 items-end gap-2 h-28">
        {BUCKETS.map((b, i) => {
          const c = counts[i];
          const h = (c / max) * 100;
          const tone = b.ub <= 2 ? "bg-evap-green/80" : b.ub <= 30 ? "bg-evap-amber/80" : "bg-evap-red/80";
          return (
            <div key={b.label} className="flex flex-col items-center gap-1">
              <div className="flex h-full w-full items-end">
                <div
                  className={`w-full rounded-t-sm ${tone}`}
                  style={{ height: `${Math.max(h, c > 0 ? 4 : 0)}%` }}
                  title={`${b.label}: ${c}`}
                />
              </div>
              <span className="text-[9px] text-zinc-500">{b.label}</span>
              <span className="text-[10px] font-mono text-zinc-700">{c}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

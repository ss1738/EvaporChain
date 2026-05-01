"use client";

interface Props {
  endorsed: number;
  required: number;
  /** Compact mode strips the labels for use in cards. */
  compact?: boolean;
}

/**
 * Stake-quorum progress bar. Hits 100% (and turns emerald) when the chain
 * would accept the amendment. Anything above stays emerald — over-quorum is
 * still valid, just wasteful.
 */
export function EndorsementProgress({ endorsed, required, compact }: Props) {
  const pct = required > 0 ? Math.min(100, (endorsed / required) * 100) : 0;
  const met = endorsed >= required && required > 0;
  const tone = met
    ? "bg-evap-emerald"
    : pct > 60
      ? "bg-evap-violet"
      : "bg-evap-cyan";

  return (
    <div className="w-full">
      {!compact && (
        <div className="mb-1 flex items-center justify-between text-[10px] font-medium text-zinc-500">
          <span>Endorsed stake</span>
          <span className="font-mono tabular-nums">
            {endorsed.toLocaleString()} / {required.toLocaleString()}
          </span>
        </div>
      )}
      <div className="h-2 w-full overflow-hidden rounded-full bg-zinc-100">
        <div
          className={`h-full ${tone} transition-all duration-300`}
          style={{ width: `${Math.max(2, pct)}%` }}
        />
      </div>
      {!compact && (
        <p
          className={`mt-1 text-[10px] font-medium ${met ? "text-evap-emerald" : "text-zinc-400"}`}
        >
          {met
            ? "Quorum met — ready to broadcast"
            : `${(100 - pct).toFixed(1)}% remaining`}
        </p>
      )}
    </div>
  );
}

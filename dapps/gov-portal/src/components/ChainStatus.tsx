"use client";

import { useEffect, useState } from "react";
import { Activity } from "lucide-react";
import { govApi } from "@/lib/api";

interface Status {
  forkChoiceMode?: string;
  attractorCount?: number;
  blockHeight?: number;
  epoch?: number;
}

/**
 * Tiny banner showing the live chain state context for governance:
 * current fork-choice mode, attractor count, current height/epoch. Pulled
 * from /api/governance/fork_choice_mode + /api/chain/status (both proxied).
 */
export function ChainStatus() {
  const [status, setStatus] = useState<Status>({});
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const [mode, chain] = await Promise.all([
          govApi.getForkChoiceMode().catch(() => null),
          govApi.chainStatus().catch(() => null),
        ]);
        if (cancelled) return;
        setStatus({
          forkChoiceMode: mode?.forkChoiceMode,
          attractorCount: mode?.attractors?.length,
          blockHeight: chain?.blockHeight,
          epoch: chain?.epoch,
        });
        setError(null);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : "load failed");
      }
    };
    load();
    const t = setInterval(load, 12_000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, []);

  return (
    <div className="rounded-xl border border-evap-border bg-white px-4 py-3">
      <div className="flex items-center gap-3">
        <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-violet-50">
          <Activity className="h-4 w-4 text-evap-violet" />
        </div>
        <div className="flex-1 grid grid-cols-2 gap-3 text-[10px] sm:grid-cols-4">
          <Field
            label="Fork-choice"
            value={status.forkChoiceMode ?? "—"}
            tone="text-evap-violet"
          />
          <Field
            label="Attractors"
            value={status.attractorCount ?? 0}
            tone="text-evap-cyan"
          />
          <Field label="Height" value={status.blockHeight ?? "—"} tone="text-zinc-700" />
          <Field label="Epoch" value={status.epoch ?? "—"} tone="text-zinc-700" />
        </div>
      </div>
      {error && (
        <p className="mt-2 text-[10px] text-evap-red">
          Chain unreachable — set EVAPORCHAIN_RPC: {error}
        </p>
      )}
    </div>
  );
}

function Field({
  label,
  value,
  tone,
}: {
  label: string;
  value: string | number;
  tone: string;
}) {
  return (
    <div>
      <p className={`text-sm font-bold ${tone}`}>{value}</p>
      <p className="text-[10px] uppercase tracking-wider text-zinc-400">{label}</p>
    </div>
  );
}

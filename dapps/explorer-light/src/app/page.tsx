"use client";

import { useEffect, useState } from "react";
import { Radio } from "lucide-react";
import { explorerApi, type CompactHeader, type LightClientStatus } from "@/lib/api";
import { HeaderRow, HeaderRowSkeleton } from "@/components/HeaderRow";
import { SearchBar } from "@/components/SearchBar";

// Live-streamed compact-header feed. Polls /api/light/headers every 5s.
// We don't open a WebSocket here on purpose — the wire format is stable HTTP
// and headers are tiny (~120 bytes each), so polling keeps the dApp portable
// behind any proxy and keeps the trust surface minimal.
export default function HomePage() {
  const [headers, setHeaders] = useState<CompactHeader[]>([]);
  const [tip, setTip] = useState<number | null>(null);
  const [trusted, setTrusted] = useState<LightClientStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const [hs, status, lc] = await Promise.all([
          explorerApi.latestHeaders(20),
          explorerApi.chainStatus(),
          explorerApi.lightClientStatus().catch(() => null),
        ]);
        if (cancelled) return;
        setHeaders(hs);
        setTip(status.blockHeight ?? hs[0]?.number ?? null);
        setTrusted(lc);
        setError(null);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    tick();
    const id = setInterval(tick, 5_000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  return (
    <div className="space-y-6">
      <SearchBar />

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <Stat label="Chain tip" value={tip != null ? `#${tip}` : "—"} tone="text-evap-cyan" />
        <Stat
          label="Trusted header watermark"
          value={trusted ? `#${trusted.latest_trusted_height}` : "—"}
          tone="text-evap-purple"
          subtitle={trusted ? `${trusted.trusted_headers_stored} headers cached` : undefined}
        />
        <Stat
          label="Verification model"
          value="client-side"
          tone="text-evap-green"
          subtitle="merkle + structural Verkle in-browser"
        />
      </div>

      <div className="overflow-hidden rounded-xl border border-evap-border bg-white">
        <div className="flex items-center justify-between border-b border-evap-border bg-zinc-50 px-4 py-2">
          <p className="text-xs font-semibold text-zinc-700">Latest compact headers</p>
          <span className="inline-flex items-center gap-1 text-[10px] text-zinc-500">
            <Radio className="h-3 w-3 animate-pulse text-evap-green" />
            live · polling 5s
          </span>
        </div>
        <div className="grid grid-cols-12 gap-2 border-b border-evap-border bg-zinc-50 px-4 py-1.5 font-mono text-[10px] uppercase tracking-wider text-zinc-400">
          <span className="col-span-1">#</span>
          <span className="col-span-1">epoch</span>
          <span className="col-span-1">txs</span>
          <span className="col-span-3">parent</span>
          <span className="col-span-3">state root</span>
          <span className="col-span-2">proof</span>
          <span className="col-span-1 text-right">age</span>
        </div>
        {loading && headers.length === 0 ? (
          Array.from({ length: 8 }).map((_, i) => <HeaderRowSkeleton key={i} />)
        ) : error ? (
          <div className="px-4 py-12 text-center text-sm text-evap-red">{error}</div>
        ) : headers.length === 0 ? (
          <div className="px-4 py-12 text-center text-sm text-zinc-500">No blocks yet.</div>
        ) : (
          headers.map((h) => <HeaderRow key={h.number} header={h} />)
        )}
      </div>

      <div className="rounded-xl border border-evap-border bg-white p-4 text-[11px] leading-relaxed text-zinc-500">
        <p className="font-semibold text-zinc-700">Why &ldquo;light&rdquo;?</p>
        <p>
          Each row above is a <em>compact header</em> (~120 bytes) — number,
          epoch, parent hash, state root, tx Merkle root, timestamp, and
          a flag for whether a folded-IVC Nova proof is attached. The dApp
          never downloads block bodies unless you click into one. When you do,
          all inclusion proofs are recomputed in your browser. The indexer is
          never trusted.
        </p>
      </div>
    </div>
  );
}

function Stat({
  label,
  value,
  tone,
  subtitle,
}: {
  label: string;
  value: string;
  tone: string;
  subtitle?: string;
}) {
  return (
    <div className="rounded-xl border border-evap-border bg-white px-4 py-3">
      <p className={`font-mono text-lg font-bold ${tone}`}>{value}</p>
      <p className="text-[10px] text-zinc-400">{label}</p>
      {subtitle && <p className="mt-0.5 text-[10px] text-zinc-400">{subtitle}</p>}
    </div>
  );
}

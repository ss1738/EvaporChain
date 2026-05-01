import Link from "next/link";
import { CheckCircle2, Circle } from "lucide-react";
import type { CompactHeader } from "@/lib/api";
import { shortHex } from "@/lib/api";

// Compact-header row in the homepage live feed. Etherscan-density,
// monospace where it matters (hashes/heights). The "Nova" pill toggles
// whether the block carries an attached folded-IVC proof — the only
// signal a light client cares about beyond the header itself.
export function HeaderRow({ header }: { header: CompactHeader }) {
  const ts = new Date(header.timestamp * 1000);
  const ago = relativeAgo(ts);
  return (
    <Link
      href={`/block/${header.number}`}
      className="grid grid-cols-12 items-center gap-2 border-b border-evap-border px-4 py-2 font-mono text-[11px] hover:bg-zinc-50"
    >
      <span className="col-span-1 font-semibold text-evap-cyan">#{header.number}</span>
      <span className="col-span-1 text-zinc-500">e{header.epoch}</span>
      <span className="col-span-1 text-zinc-700">{header.tx_count} tx</span>
      <span className="col-span-3 truncate text-zinc-500" title={header.parent_hash}>
        ← {shortHex(header.parent_hash)}
      </span>
      <span className="col-span-3 truncate text-zinc-700" title={header.state_root}>
        sr {shortHex(header.state_root)}
      </span>
      <span className="col-span-2 flex items-center gap-1 text-zinc-500">
        {header.has_nova_proof ? (
          <CheckCircle2 className="h-3 w-3 text-evap-green" />
        ) : (
          <Circle className="h-3 w-3 text-zinc-300" />
        )}
        Nova
      </span>
      <span className="col-span-1 text-right text-[10px] text-zinc-400">{ago}</span>
    </Link>
  );
}

export function HeaderRowSkeleton() {
  return (
    <div className="grid grid-cols-12 gap-2 border-b border-evap-border px-4 py-2">
      <div className="col-span-12 h-3 animate-pulse rounded bg-zinc-100" />
    </div>
  );
}

function relativeAgo(d: Date): string {
  const sec = Math.max(0, Math.floor((Date.now() - d.getTime()) / 1000));
  if (sec < 60) return `${sec}s`;
  if (sec < 3600) return `${Math.floor(sec / 60)}m`;
  if (sec < 86400) return `${Math.floor(sec / 3600)}h`;
  return `${Math.floor(sec / 86400)}d`;
}

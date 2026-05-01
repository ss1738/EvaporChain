import { Hash } from "lucide-react";

// Small shard badge for the address page header. The chain currently uses a
// "first byte mod num_shards" assignment; if a richer locator endpoint is
// exposed we'll switch to that.
export function ShardChip({ shard, total }: { shard: number; total: number }) {
  if (total <= 0) {
    return (
      <span className="inline-flex items-center gap-1 rounded-full border border-evap-border bg-zinc-50 px-2 py-0.5 text-[10px] text-zinc-500">
        <Hash className="h-3 w-3" /> sharding off
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1 rounded-full border border-cyan-200 bg-cyan-50 px-2 py-0.5 font-mono text-[10px] text-evap-cyan">
      <Hash className="h-3 w-3" /> shard {shard}/{total}
    </span>
  );
}

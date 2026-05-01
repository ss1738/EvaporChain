import Link from "next/link";
import type { TxRecord } from "@/lib/api";
import { shortHex } from "@/lib/api";
import { StateBadge } from "./StateBadge";

// Compact tx row used inside block-detail and the homepage feed. Keeps the
// hash mono-spaced and ~12-col friendly so users can scan a thousand rows.
export function TxRow({ tx, includeBlock = false }: { tx: TxRecord; includeBlock?: boolean }) {
  return (
    <Link
      href={`/tx/${tx.hash}`}
      className="grid grid-cols-12 items-center gap-2 border-b border-evap-border px-4 py-2 font-mono text-[11px] hover:bg-zinc-50"
    >
      <span className="col-span-3 truncate text-zinc-700" title={tx.hash}>
        {shortHex(tx.hash)}
      </span>
      <span className="col-span-2 text-zinc-500">{tx.type}</span>
      {includeBlock ? (
        <span className="col-span-1 text-zinc-500">#{tx.block_number}</span>
      ) : null}
      <span className={`${includeBlock ? "col-span-2" : "col-span-3"} truncate text-zinc-500`} title={tx.from}>
        {shortHex(tx.from)}
      </span>
      <span className={`${includeBlock ? "col-span-2" : "col-span-2"} truncate text-zinc-500`} title={tx.to}>
        → {tx.to ? shortHex(tx.to) : "—"}
      </span>
      <span className="col-span-1 text-right text-zinc-700">
        {tx.amount != null ? tx.amount.toLocaleString() : ""}
      </span>
      <span className={`${includeBlock ? "col-span-1" : "col-span-1"} flex justify-end`}>
        <StateBadge state={tx.status === "success" ? "finalised" : tx.status} />
      </span>
    </Link>
  );
}

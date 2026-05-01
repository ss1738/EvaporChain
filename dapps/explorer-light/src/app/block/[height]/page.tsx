"use client";

import { useEffect, useState, use } from "react";
import Link from "next/link";
import { ArrowLeft, Download, ShieldCheck } from "lucide-react";
import {
  explorerApi,
  shortHex,
  type CompactHeader,
  type BlockDetail,
} from "@/lib/api";
import { TxRow } from "@/components/TxRow";

// Block detail. We deliberately split the fetch in two:
//   1. compact header   — always pulled (cheap, the light path)
//   2. full block body  — only on user click (the heavy path)
// This is the headline UX of a light explorer: the user *opts in* to
// downloading every transaction in the block.
export default function BlockPage({ params }: { params: Promise<{ height: string }> }) {
  const { height: heightStr } = use(params);
  const height = Number(heightStr);

  const [header, setHeader] = useState<CompactHeader | null>(null);
  const [headerError, setHeaderError] = useState<string | null>(null);
  const [block, setBlock] = useState<BlockDetail | null>(null);
  const [loadingBlock, setLoadingBlock] = useState(false);
  const [blockError, setBlockError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    explorerApi
      .headerAt(height)
      .then((h) => {
        if (cancelled) return;
        setHeader(h);
        if (!h) setHeaderError("compact header not found at this height");
      })
      .catch((e) => {
        if (!cancelled) setHeaderError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [height]);

  const loadFullBlock = async () => {
    setLoadingBlock(true);
    setBlockError(null);
    try {
      const b = await explorerApi.fullBlock(height);
      setBlock(b);
    } catch (e) {
      setBlockError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoadingBlock(false);
    }
  };

  if (Number.isNaN(height)) {
    return <p className="text-sm text-evap-red">Invalid block height.</p>;
  }

  return (
    <div className="space-y-6">
      <Link
        href="/"
        className="inline-flex items-center gap-1 text-xs text-zinc-500 hover:text-zinc-900"
      >
        <ArrowLeft className="h-3 w-3" /> latest headers
      </Link>

      <div>
        <h1 className="font-mono text-2xl font-bold text-zinc-900">Block #{height}</h1>
        <p className="text-xs text-zinc-500">
          Compact header pulled from <code className="font-mono">/api/light/headers</code>;
          full block body is gated behind an explicit click.
        </p>
      </div>

      <div className="rounded-xl border border-evap-border bg-white p-5">
        <p className="text-xs font-semibold text-zinc-700">Compact header</p>
        {headerError && (
          <p className="mt-2 text-sm text-evap-red">{headerError}</p>
        )}
        {header && (
          <dl className="mt-3 grid grid-cols-1 gap-x-6 gap-y-2 sm:grid-cols-2">
            <Field k="Number" v={`#${header.number}`} mono />
            <Field k="Epoch" v={`${header.epoch}`} mono />
            <Field k="Tx count" v={`${header.tx_count}`} mono />
            <Field k="Timestamp" v={new Date(header.timestamp * 1000).toISOString()} mono />
            <Field k="Parent hash" v={header.parent_hash} mono full />
            <Field k="State root" v={header.state_root} mono full />
            <Field k="Tx Merkle root" v={header.tx_merkle_root} mono full />
            <Field
              k="Nova proof"
              v={header.has_nova_proof ? "attached" : "absent"}
              mono
            />
          </dl>
        )}
        {!header && !headerError && (
          <div className="mt-3 h-20 animate-pulse rounded bg-zinc-100" />
        )}
      </div>

      <div className="rounded-xl border border-evap-border bg-white p-5">
        <div className="flex items-center justify-between gap-3">
          <div>
            <p className="text-xs font-semibold text-zinc-700">Block body (heavy)</p>
            <p className="text-[11px] text-zinc-500">
              Click to fetch <code className="font-mono">/api/block/{height}</code>{" "}
              with the full transaction list. Each tx can then be Merkle-verified
              against the compact header above.
            </p>
          </div>
          <button
            onClick={loadFullBlock}
            disabled={loadingBlock || !header}
            className="inline-flex items-center gap-1 rounded-md border border-evap-cyan/40 bg-evap-cyan/5 px-3 py-1.5 text-xs font-semibold text-evap-cyan hover:bg-evap-cyan/10 disabled:opacity-50"
          >
            <Download className="h-3 w-3" />
            {block ? "Refresh full block" : "Load full block"}
          </button>
        </div>

        {blockError && <p className="mt-3 text-sm text-evap-red">{blockError}</p>}

        {block && (
          <>
            <div className="mt-4 grid grid-cols-2 gap-3 sm:grid-cols-4">
              <Mini label="evaporations" value={block.evaporations} />
              <Mini label="entered grace" value={block.entered_grace} />
              <Mini label="active objs" value={block.active_objects} />
              <Mini label="ghosts" value={block.ghost_count} />
              <Mini label="gas used" value={block.gas_used.toLocaleString()} />
              <Mini label="base fee" value={block.base_fee.toLocaleString()} />
              <Mini label="total fees" value={block.total_fees.toLocaleString()} />
              <Mini
                label="state root"
                value={shortHex(block.state_root)}
                mono
                ok={block.state_root === header?.state_root}
              />
            </div>

            {header &&
              block.state_root.toLowerCase() !== header.state_root.toLowerCase() && (
                <div className="mt-3 rounded-md border border-red-200 bg-red-50 p-3 text-[11px] text-evap-red">
                  WARNING: full-block state_root differs from the compact-header
                  state_root. Treat this server as untrusted.
                </div>
              )}
            {header &&
              block.state_root.toLowerCase() === header.state_root.toLowerCase() && (
                <div className="mt-3 inline-flex items-center gap-1 rounded-md border border-emerald-200 bg-emerald-50 px-2 py-1 text-[10px] text-evap-green">
                  <ShieldCheck className="h-3 w-3" /> full-block state_root matches
                  trusted compact header
                </div>
              )}

            <div className="mt-5 overflow-hidden rounded-lg border border-evap-border">
              <div className="grid grid-cols-12 gap-2 border-b border-evap-border bg-zinc-50 px-4 py-1.5 font-mono text-[10px] uppercase tracking-wider text-zinc-400">
                <span className="col-span-3">hash</span>
                <span className="col-span-2">type</span>
                <span className="col-span-3">from</span>
                <span className="col-span-2">to</span>
                <span className="col-span-1 text-right">amount</span>
                <span className="col-span-1 text-right">state</span>
              </div>
              {block.transactions.length === 0 ? (
                <p className="px-4 py-6 text-center text-xs text-zinc-500">no transactions in this block</p>
              ) : (
                block.transactions.map((tx) => <TxRow key={tx.hash} tx={tx} />)
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function Field({ k, v, mono, full }: { k: string; v: string; mono?: boolean; full?: boolean }) {
  return (
    <div className={full ? "sm:col-span-2" : undefined}>
      <dt className="text-[10px] uppercase tracking-wider text-zinc-400">{k}</dt>
      <dd className={`text-xs text-zinc-700 ${mono ? "break-all font-mono" : ""}`}>{v}</dd>
    </div>
  );
}

function Mini({
  label,
  value,
  mono,
  ok,
}: {
  label: string;
  value: number | string;
  mono?: boolean;
  ok?: boolean;
}) {
  return (
    <div className="rounded-md border border-evap-border bg-zinc-50 px-3 py-2">
      <p className={`text-xs ${mono ? "font-mono" : "font-semibold"} ${ok ? "text-evap-green" : "text-zinc-700"}`}>
        {value}
      </p>
      <p className="text-[10px] text-zinc-400">{label}</p>
    </div>
  );
}

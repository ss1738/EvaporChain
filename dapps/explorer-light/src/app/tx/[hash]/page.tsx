"use client";

import { useEffect, useState, use } from "react";
import Link from "next/link";
import { ArrowLeft } from "lucide-react";
import { explorerApi, type CompactHeader, type TxStatus, type TxRecord } from "@/lib/api";
import { StateBadge } from "@/components/StateBadge";
import { VerifyInclusionButton } from "@/components/VerifyInclusionButton";

// Tx detail. Polls /api/tx/:hash so the user can watch the state machine
// progress (pending → mempool → included → finalised) in real time.
//
// We also pull the compact header for the inclusion block (if known) so the
// "Verify inclusion" button can anchor against the trusted tx_merkle_root.
export default function TxPage({ params }: { params: Promise<{ hash: string }> }) {
  const { hash: rawHash } = use(params);
  const hash = rawHash.replace(/^0x/i, "").toLowerCase();

  const [status, setStatus] = useState<TxStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [header, setHeader] = useState<CompactHeader | null>(null);
  const [tx, setTx] = useState<TxRecord | null>(null);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const s = await explorerApi.tx(hash);
        if (cancelled) return;
        setStatus(s);
        setError(null);

        // Once we know the inclusion block, pull the compact header so the
        // verify button can anchor proofs to the trusted tx_merkle_root.
        if (s.block_height != null && (!header || header.number !== s.block_height)) {
          const h = await explorerApi.headerAt(s.block_height);
          if (!cancelled) setHeader(h);
        }

        // Look up the tx body inside the block payload — the lightest way to
        // get tx_index without a dedicated endpoint.
        if (
          s.block_height != null &&
          (s.state === "included" || s.state === "finalised" || s.state === "rejected") &&
          (!tx || tx.hash.toLowerCase() !== hash)
        ) {
          try {
            const b = await explorerApi.fullBlock(s.block_height);
            const found = b.transactions.find((t) => t.hash.toLowerCase() === hash);
            if (!cancelled && found) setTx(found);
          } catch {
            // best-effort; tx body fetch is non-fatal
          }
        }
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }
    };
    tick();
    const id = setInterval(tick, 4_000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [hash, header, tx]);

  return (
    <div className="space-y-6">
      <Link
        href="/"
        className="inline-flex items-center gap-1 text-xs text-zinc-500 hover:text-zinc-900"
      >
        <ArrowLeft className="h-3 w-3" /> latest headers
      </Link>

      <div>
        <h1 className="break-all font-mono text-base text-zinc-900">0x{hash}</h1>
        <p className="mt-1 text-xs text-zinc-500">
          Live tx status from <code className="font-mono">/api/tx/:hash</code>;
          polled every 4 seconds.
        </p>
      </div>

      {error && (
        <div className="rounded-lg border border-red-200 bg-red-50 p-3 text-xs text-evap-red">
          {error}
        </div>
      )}

      <div className="rounded-xl border border-evap-border bg-white p-5">
        <div className="flex items-center justify-between">
          <p className="text-xs font-semibold text-zinc-700">Status</p>
          {status && <StateBadge state={status.state} />}
        </div>
        {status ? (
          <>
            {/*
              Rejected-tx error surfacing. The error string can come from
              either /api/tx/:hash (TxStatus.error) or the per-block
              /api/transactions feed (TxRecord.error) — they're sourced
              from the same TxOutcome.error on the executor and should
              agree, but we tolerate one being absent. Multi-line stack
              traces are rendered behind a <details> so the row stays
              compact by default.
            */}
            {status.state === "rejected" && (status.error || tx?.error) && (
              <RejectedError text={status.error ?? tx?.error ?? ""} />
            )}
            <dl className="mt-3 grid grid-cols-1 gap-x-6 gap-y-2 sm:grid-cols-2">
              {status.block_height != null && (
                <Field k="Block" v={`#${status.block_height}`} />
              )}
              {status.epoch != null && <Field k="Epoch" v={`${status.epoch}`} />}
              {status.confirmations != null && (
                <Field k="Confirmations" v={`${status.confirmations}`} />
              )}
              {status.gas_used != null && <Field k="Gas used" v={status.gas_used.toLocaleString()} />}
              {status.tx_index != null && <Field k="Tx index" v={`${status.tx_index}`} />}
              {status.mempool_position != null && (
                <Field
                  k="Mempool position"
                  v={`${status.mempool_position} of ${status.mempool_size ?? "?"}`}
                />
              )}
              {/*
                Inline summary line for cases where the explorer reads
                /api/transactions instead of /api/tx/:hash (e.g. the
                state machine has long since finalised); the Rejected
                callout above is the primary surface.
              */}
              {status.state !== "rejected" && status.error && (
                <Field k="Error" v={status.error} />
              )}
            </dl>
          </>
        ) : (
          <div className="mt-3 h-16 animate-pulse rounded bg-zinc-100" />
        )}
      </div>

      {tx && (
        <div className="rounded-xl border border-evap-border bg-white p-5">
          <p className="text-xs font-semibold text-zinc-700">Transaction body</p>
          <dl className="mt-3 grid grid-cols-1 gap-x-6 gap-y-2 sm:grid-cols-2">
            <Field k="Type" v={tx.type} />
            <Field k="From" v={tx.from} mono full />
            <Field k="To" v={tx.to || "—"} mono full />
            {tx.amount != null && <Field k="Amount" v={tx.amount.toLocaleString()} mono />}
            {tx.energy != null && <Field k="Energy" v={tx.energy.toLocaleString()} mono />}
            {tx.half_life != null && <Field k="Half-life" v={`${tx.half_life}`} mono />}
            {tx.method && <Field k="Method" v={tx.method} />}
            <Field k="Gas" v={`${tx.gas}`} mono />
            <Field k="Status" v={tx.status} />
          </dl>
        </div>
      )}

      {status?.block_height != null ? (
        <VerifyInclusionButton
          blockNumber={status.block_height}
          txIndex={status.tx_index ?? -1}
          headerTxMerkleRoot={header?.tx_merkle_root}
        />
      ) : (
        <div className="rounded-xl border border-dashed border-evap-border bg-white p-4 text-[11px] text-zinc-500">
          Inclusion proof becomes available once the tx is included in a block.
        </div>
      )}
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

/**
 * Red-tinted callout that surfaces the executor rejection reason for
 * a failed tx. Single-line strings render inline; multi-line content
 * (stack-style detail) is collapsed behind a "View error" disclosure
 * so the row stays compact while still letting users diagnose deep
 * failures.
 */
function RejectedError({ text }: { text: string }) {
  const isMultiLine = text.includes("\n");
  const firstLine = isMultiLine ? text.split("\n", 1)[0] : text;
  return (
    <div className="mt-3 rounded-lg border border-red-200 bg-red-50 p-3">
      <p className="text-[10px] font-semibold uppercase tracking-wider text-evap-red">
        Execution error
      </p>
      <p className="mt-1 break-all font-mono text-xs text-evap-red">{firstLine}</p>
      {isMultiLine && (
        <details className="mt-2">
          <summary className="cursor-pointer text-[11px] text-evap-red hover:underline">
            View error
          </summary>
          <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded bg-white/60 p-2 font-mono text-[11px] text-evap-red">
            {text}
          </pre>
        </details>
      )}
    </div>
  );
}

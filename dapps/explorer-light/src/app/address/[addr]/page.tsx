"use client";

import { useEffect, useState, use } from "react";
import Link from "next/link";
import { ArrowLeft, Wallet } from "lucide-react";
import {
  explorerApi,
  shortHex,
  shardOf,
  type AddressDetail,
  type TxRecord,
  type ShardStatus,
} from "@/lib/api";
import { TxRow } from "@/components/TxRow";
import { ShardChip } from "@/components/ShardChip";
import { EnergyBar } from "@/components/EnergyBar";
import { VerifyBalanceButton } from "@/components/VerifyBalanceButton";

// Address overview. Pulls /api/address/:addr for balance / nonce / objects,
// /api/transactions?address=:addr for the tx feed, /api/shards for shard
// assignment, and exposes "Verify balance" which routes to the Verkle
// state-proof structural verifier.
export default function AddressPage({ params }: { params: Promise<{ addr: string }> }) {
  const { addr: rawAddr } = use(params);
  const addr = decodeURIComponent(rawAddr).replace(/^0x/i, "").toLowerCase();

  const [detail, setDetail] = useState<AddressDetail | null>(null);
  const [txs, setTxs] = useState<TxRecord[]>([]);
  const [shard, setShard] = useState<ShardStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const [d, feed, sh] = await Promise.all([
          explorerApi.address(addr),
          explorerApi.transactions(25, 0).catch(() => null),
          explorerApi.shardStatus().catch(() => null),
        ]);
        if (cancelled) return;
        setDetail(d);
        setShard(sh);
        if (feed) {
          const lower = addr.toLowerCase();
          setTxs(
            feed.transactions
              .filter(
                (t) =>
                  t.from.toLowerCase().includes(lower) ||
                  t.to.toLowerCase().includes(lower),
              )
              .slice(0, 15),
          );
        }
        setError(null);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    tick();
    const id = setInterval(tick, 8_000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [addr]);

  const numShards = shard?.active ? (shard.num_shards ?? 0) : 0;
  const myShard = numShards > 0 ? shardOf(addr, numShards) : 0;

  return (
    <div className="space-y-6">
      <Link
        href="/"
        className="inline-flex items-center gap-1 text-xs text-zinc-500 hover:text-zinc-900"
      >
        <ArrowLeft className="h-3 w-3" /> latest headers
      </Link>

      <div className="flex flex-wrap items-center gap-3">
        <div className="rounded-lg border border-evap-border bg-white p-3">
          <Wallet className="h-5 w-5 text-evap-cyan" />
        </div>
        <div className="min-w-0 flex-1">
          <h1 className="break-all font-mono text-base text-zinc-900">0x{addr}</h1>
          <p className="text-xs text-zinc-500">
            {detail ? shortHex(detail.address) : "loading…"}
          </p>
        </div>
        {numShards > 0 && <ShardChip shard={myShard} total={numShards} />}
      </div>

      {error && (
        <div className="rounded-lg border border-red-200 bg-red-50 p-3 text-xs text-evap-red">
          {error}
        </div>
      )}

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <Stat
          label="Balance"
          value={detail ? detail.balance.toLocaleString() : "—"}
          tone="text-evap-cyan"
          subtitle="EVAP"
        />
        <Stat label="Nonce" value={detail ? `${detail.nonce}` : "—"} tone="text-evap-purple" />
        <Stat
          label="Last touched"
          value={detail ? `epoch ${detail.last_touched_epoch}` : "—"}
          tone="text-zinc-700"
          subtitle="anchors demurrage accrual"
        />
      </div>

      <VerifyBalanceButton address={addr} />

      <div className="rounded-xl border border-evap-border bg-white p-5">
        <p className="text-xs font-semibold text-zinc-700">Owned objects</p>
        {detail && detail.objects.length > 0 ? (
          <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {detail.objects.slice(0, 12).map((o) => (
              <div key={o.id} className="rounded-lg border border-evap-border p-3">
                <p className="font-mono text-[11px] font-semibold text-zinc-700">{o.name}</p>
                <p className="font-mono text-[10px] text-zinc-400">{shortHex(o.id)}</p>
                <div className="mt-2">
                  <EnergyBar energy={o.current_energy} maxEnergy={o.max_energy} state={o.state} />
                </div>
                <p className="mt-1 text-[10px] text-zinc-400">half-life {o.half_life}e · {o.decay_percentage}% decayed</p>
              </div>
            ))}
          </div>
        ) : (
          <p className="mt-2 text-xs text-zinc-500">{loading ? "loading…" : "no objects"}</p>
        )}
      </div>

      <div className="overflow-hidden rounded-xl border border-evap-border bg-white">
        <div className="border-b border-evap-border bg-zinc-50 px-4 py-2">
          <p className="text-xs font-semibold text-zinc-700">Recent transactions touching this address</p>
        </div>
        <div className="grid grid-cols-12 gap-2 border-b border-evap-border bg-zinc-50 px-4 py-1.5 font-mono text-[10px] uppercase tracking-wider text-zinc-400">
          <span className="col-span-3">hash</span>
          <span className="col-span-2">type</span>
          <span className="col-span-1">block</span>
          <span className="col-span-2">from</span>
          <span className="col-span-2">to</span>
          <span className="col-span-1 text-right">amount</span>
          <span className="col-span-1 text-right">state</span>
        </div>
        {txs.length === 0 ? (
          <p className="px-4 py-6 text-center text-[11px] text-zinc-500">
            {loading ? "loading…" : "no recent transactions"}
          </p>
        ) : (
          txs.map((t) => <TxRow key={t.hash} tx={t} includeBlock />)
        )}
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

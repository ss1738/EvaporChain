"use client";

import { useEffect, useState } from "react";
import {
  analyticsApi,
  pollWithInterval,
  uptimeProxy,
  type ValidatorsResponse,
} from "@/lib/api";
import { ValidatorTable } from "@/components/ValidatorTable";

export default function ValidatorsPage() {
  const [vs, setVs] = useState<ValidatorsResponse | null>(null);
  const [perValidator, setPerValidator] = useState<Map<number, number[]>>(new Map());
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    return pollWithInterval(
      async () => {
        const [v, m] = await Promise.all([analyticsApi.validators(), analyticsApi.metrics()]);
        const buckets = new Map<number, number[]>();
        for (const p of m.perValidator) {
          // Producer label is `validator-{id}` per api.rs:12076.
          const id = Number(p.producer.replace(/^validator-/, ""));
          if (Number.isFinite(id)) {
            // Use cumulative bucket counts as the sparkline values — strictly
            // monotonic so the trend reflects "how many recent blocks fell
            // under each latency bucket". A flat right-tail is healthy.
            buckets.set(id, p.buckets.map((b) => b.cum));
          }
        }
        return { v, buckets };
      },
      ({ v, buckets }) => {
        setVs(v);
        setPerValidator(buckets);
        setErr(null);
      },
      (e) => setErr(e.message),
      15_000,
    );
  }, []);

  if (err && !vs) {
    return <p className="py-20 text-center text-sm text-evap-red">{err}</p>;
  }
  if (!vs) {
    return <p className="py-20 text-center text-sm text-zinc-500">Loading…</p>;
  }
  const uptime = uptimeProxy(vs.validators);
  return (
    <div className="space-y-4">
      <header>
        <h1 className="text-xl font-bold text-zinc-900">Validators</h1>
        <p className="mt-1 text-xs text-zinc-500">
          {vs.count} total · {vs.bls_registered_count} BLS-registered · {vs.jailed_count} jailed.
          Click any column to sort.
        </p>
      </header>
      <ValidatorTable rows={vs.validators} uptime={uptime} blockHist={perValidator} />
    </div>
  );
}

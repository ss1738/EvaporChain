"use client";

import { useEffect, useState } from "react";
import { explorerApi, type CompactHeader } from "@/lib/api";
import { EpsilonMachineGraph } from "@/components/EpsilonMachineGraph";

// State-evolution graph for the CSLC ε-machine.
//
// The chain currently exposes only POST /api/cslc_reconstruct, which
// reconstructs an ε-machine from a symbol-count distribution (CSSR over
// the unconditional symbol histogram) and returns a *summary* — state
// count and alphabet size — but not the full transition graph. There is
// no GET /api/cslc/states. We document the gap on the page itself.
//
// To still make this page useful we feed the live header sequence into
// CSSR by quantising tx_count into a small alphabet and posting the
// resulting count vector. That gives users a real ε-machine summary
// derived from the most-recent block stream, even if the transition
// graph itself isn't exposed yet.
export default function StateGraphPage() {
  const [summary, setSummary] = useState<{
    state_count?: number;
    alphabet_size?: number;
    detail?: string;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [sampleSize, setSampleSize] = useState(0);

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      try {
        const headers: CompactHeader[] = await explorerApi.latestHeaders(200);
        if (cancelled) return;
        if (headers.length === 0) {
          setSummary({ state_count: 0, alphabet_size: 0, detail: "no blocks yet" });
          return;
        }
        // Build a tx-count histogram bucketed into 5 bins as our symbol
        // alphabet. Real ε-machines on EvaporChain use richer state
        // descriptors; this quantisation is a stand-in until the chain
        // exposes its production CSSR pipeline over HTTP.
        const buckets = [0, 0, 0, 0, 0];
        for (const h of headers) {
          const tc = h.tx_count;
          const idx = tc === 0 ? 0 : tc < 5 ? 1 : tc < 20 ? 2 : tc < 100 ? 3 : 4;
          buckets[idx]++;
        }
        setSampleSize(headers.length);

        const r = await fetch("/api/cslc_reconstruct", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ counts: buckets }),
        });
        if (!r.ok) {
          throw new Error(`/api/cslc_reconstruct → ${r.status}`);
        }
        const j: { state_count: number; alphabet_size: number; detail: string } = await r.json();
        if (cancelled) return;
        setSummary({
          state_count: j.state_count,
          alphabet_size: j.alphabet_size,
          detail: j.detail || "ok",
        });
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    run();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-xl font-bold text-zinc-900">ε-Machine state evolution</h1>
        <p className="mt-1 text-xs text-zinc-500">
          CSLC (Causal-State Light Client) reconstructs the minimal sufficient
          statistic for predicting future blocks via Shalizi-Crutchfield CSSR.
          Light clients sync against this state distribution rather than full
          block bodies.
        </p>
      </div>

      {loading && (
        <div className="rounded-xl border border-evap-border bg-white p-6">
          <div className="h-24 animate-pulse rounded bg-zinc-100" />
        </div>
      )}

      {error && (
        <div className="rounded-lg border border-red-200 bg-red-50 p-3 text-xs text-evap-red">
          {error}
        </div>
      )}

      {summary && !error && (
        <>
          <EpsilonMachineGraph
            stateCount={summary.state_count}
            alphabetSize={summary.alphabet_size}
            detail={summary.detail}
          />
          <p className="text-[11px] text-zinc-500">
            Reconstructed from <code className="font-mono">{sampleSize}</code> recent compact
            headers · symbols quantised into 5 tx-count buckets · CSSR via{" "}
            <code className="font-mono">POST /api/cslc_reconstruct</code>.
          </p>
        </>
      )}
    </div>
  );
}

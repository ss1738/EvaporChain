"use client";

import { useEffect, useMemo, useState } from "react";
import { singhApi, type SwapQuote } from "@/lib/api";

// Standard token-pair swap. Price impact is computed against the active
// liquidity sum — Grace/Ghost positions are excluded from the AMM curve so
// users see slippage that reflects only live liquidity.
export default function SwapPage() {
  const [fromToken, setFromToken] = useState("EVAP");
  const [toToken, setToToken] = useState("USDC");
  const [amountIn, setAmountIn] = useState(100);
  const [quote, setQuote] = useState<SwapQuote | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    if (amountIn <= 0) {
      setQuote(null);
      return;
    }
    setLoading(true);
    setError(null);
    singhApi
      .quote(fromToken, toToken, amountIn)
      .then((q) => {
        if (!cancelled) setQuote(q);
      })
      .catch((err) => {
        if (!cancelled) setError(err instanceof Error ? err.message : "Quote failed");
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [fromToken, toToken, amountIn]);

  const decayShare = useMemo(() => {
    if (!quote) return 0;
    const total = quote.activeLiquidity + quote.decayedLiquidity;
    return total > 0 ? Math.round((quote.decayedLiquidity / total) * 100) : 0;
  }, [quote]);

  return (
    <div className="mx-auto max-w-md space-y-4">
      <div>
        <h1 className="text-xl font-bold text-zinc-900">Swap</h1>
        <p className="mt-1 text-xs text-zinc-500">
          Price impact is computed against{" "}
          <span className="font-semibold text-evap-cyan">active</span> liquidity only.
          Decayed positions don&rsquo;t earn fees and don&rsquo;t move the curve.
        </p>
      </div>

      <div className="rounded-2xl border border-evap-border bg-white p-4">
        <div className="grid grid-cols-3 gap-2">
          <input
            value={fromToken}
            onChange={(e) => setFromToken(e.target.value.toUpperCase())}
            className="rounded-lg border border-evap-border bg-white px-3 py-2 text-sm focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
          />
          <span className="self-center text-center text-xs text-zinc-400">→</span>
          <input
            value={toToken}
            onChange={(e) => setToToken(e.target.value.toUpperCase())}
            className="rounded-lg border border-evap-border bg-white px-3 py-2 text-sm focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
          />
        </div>

        <div className="mt-4">
          <label className="block text-xs font-medium text-zinc-700">
            Amount in ({fromToken})
          </label>
          <input
            type="number"
            min={0}
            value={amountIn}
            onChange={(e) => setAmountIn(Math.max(0, Number(e.target.value)))}
            className="mt-1 w-full rounded-lg border border-evap-border bg-white px-3 py-2 text-base focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
          />
        </div>

        {loading && (
          <p className="mt-3 text-xs text-zinc-400">Pricing across active positions…</p>
        )}
        {error && <p className="mt-3 text-xs text-evap-red">{error}</p>}

        {quote && (
          <div className="mt-4 rounded-lg border border-evap-cyan/30 bg-cyan-50 p-3 text-xs text-evap-cyan">
            <div className="flex items-center justify-between">
              <span className="font-semibold">You receive</span>
              <span className="font-mono text-base font-bold">
                {quote.amountOut.toFixed(4)} {toToken}
              </span>
            </div>
            <div className="mt-2 grid grid-cols-3 gap-2 text-[10px]">
              <Box label="Price impact" value={`${(quote.priceImpactBps / 100).toFixed(2)} bps`} />
              <Box label="Fee" value={`${quote.feeBps / 100}%`} />
              <Box label="Active LIQ" value={quote.activeLiquidity.toLocaleString()} />
            </div>
            {decayShare > 0 && (
              <p className="mt-2 text-[10px] text-evap-amber">
                ⚠ {decayShare}% of pool liquidity is decayed and excluded from this quote.
              </p>
            )}
          </div>
        )}

        <button
          disabled={!quote || loading}
          className="mt-4 w-full rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple px-4 py-2.5 text-sm font-semibold text-white hover:opacity-90 disabled:opacity-50"
          onClick={() => {
            // Real broadcast goes through the wallet provider's signed swap
            // tx. UI proves the substrate primitives work — the chain
            // mutation requires a connected wallet.
            alert(
              "Connect EvaporChain Wallet to broadcast this swap. The quote above already excludes decayed positions.",
            );
          }}
        >
          Swap {amountIn} {fromToken} → {toToken}
        </button>
      </div>
    </div>
  );
}

function Box({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md bg-white/60 px-2 py-1">
      <p className="font-semibold">{value}</p>
      <p className="opacity-70">{label}</p>
    </div>
  );
}

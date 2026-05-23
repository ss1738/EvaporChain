import { useEffect, useState } from "react";
import { ArrowLeft } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import { api, type TokenInfo, type PriceData } from "@/utils/api";
import { Header } from "./Header";
import { formatBalance } from "@/utils/format";

function Bar({ pct, color }: { pct: number; color: string }) {
  return (
    <div className="flex-1 h-1.5 rounded-full bg-zinc-800 overflow-hidden">
      <div
        className={`h-full rounded-full ${color} transition-all duration-700`}
        style={{ width: `${Math.min(pct, 100)}%` }}
      />
    </div>
  );
}

export function PortfolioScreen() {
  const { activeAccount, balance, tokens, objects, setView } = useWallet();
  const [prices, setPrices] = useState<PriceData[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!activeAccount) return;
    api.getPrices()
      .catch(() => [])
      .then((p) => { setPrices(p); setLoading(false); });
  }, [activeAccount]);

  const evapPrice = prices.find((p) => p.symbol === "EVAP")?.price_usd ?? 0;
  const evapChange = prices.find((p) => p.symbol === "EVAP")?.change_24h_pct ?? 0;
  const evapUsd = balance * evapPrice;

  const tokenRows: Array<{ token: TokenInfo; price: PriceData | undefined; usd: number }> = tokens.map((t) => {
    const price = prices.find((p) => p.symbol === t.symbol);
    return { token: t, price, usd: t.balance * (price?.price_usd ?? 0) };
  });

  const totalUsd = evapUsd + tokenRows.reduce((s, r) => s + r.usd, 0);

  const activeObjects = objects.filter((o) => o.state === "Active").length;
  const graceObjects = objects.filter((o) => o.state === "Grace").length;
  const ghostObjects = objects.filter((o) => o.state === "Ghost").length;
  const totalEnergy = objects.reduce((s, o) => s + (o.current_energy ?? 0), 0);

  return (
    <div className="flex flex-col h-full">
      <Header />
      <div className="px-4 pt-4 pb-2">
        <button onClick={() => setView("home")} className="text-xs text-zinc-500 hover:text-zinc-300 mb-3"><><ArrowLeft className="inline w-3.5 h-3.5 mr-1 -mt-0.5" strokeWidth={1.5} />Back</></button>
        <h2 className="text-lg font-semibold text-zinc-100">Portfolio Analytics</h2>
      </div>

      <div className="flex-1 overflow-y-auto px-4 space-y-3 pb-4">
        {/* Total value */}
        <div className="px-4 py-4 rounded-xl bg-evap-surface border border-evap-border">
          <p className="text-xs text-zinc-500 uppercase tracking-wider mb-1">Total Portfolio Value</p>
          <p className="text-2xl font-bold text-zinc-100">
            {evapPrice > 0
              ? `$${totalUsd.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`
              : "—"}
          </p>
          <div className="flex items-center gap-2 mt-1">
            <span className={`text-xs font-medium ${evapChange >= 0 ? "text-evap-green" : "text-evap-red"}`}>
              {evapChange >= 0 ? "+" : ""}{evapChange.toFixed(2)}% EVAP (24h)
            </span>
          </div>
        </div>

        {/* EVAP row */}
        <div className="px-4 py-3 rounded-xl bg-evap-surface border border-evap-border">
          <div className="flex items-center justify-between mb-1">
            <div className="flex items-center gap-2">
              <div className="w-7 h-7 rounded-full bg-evap-cyan/20 flex items-center justify-center">
                <span className="text-xs font-bold text-evap-cyan">E</span>
              </div>
              <div>
                <p className="text-xs font-medium text-zinc-200">EVAP</p>
                <p className="text-xs text-zinc-500">EvaporChain</p>
              </div>
            </div>
            <div className="text-right">
              <p className="text-xs font-semibold text-zinc-200">{formatBalance(balance)} EVAP</p>
              {evapPrice > 0 && (
                <p className="text-xs text-zinc-500">
                  ${evapUsd.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
                </p>
              )}
            </div>
          </div>
          {totalUsd > 0 && (
            <div className="flex items-center gap-2 mt-2">
              <Bar pct={evapUsd / totalUsd * 100} color="bg-evap-cyan" />
              <span className="text-xs text-zinc-500 w-10 text-right">
                {(evapUsd / totalUsd * 100).toFixed(1)}%
              </span>
            </div>
          )}
        </div>

        {/* Other tokens */}
        {tokenRows.filter((r) => r.token.balance > 0).map(({ token, price, usd }) => (
          <div key={token.symbol} className="px-4 py-3 rounded-xl bg-evap-surface border border-evap-border">
            <div className="flex items-center justify-between mb-1">
              <div className="flex items-center gap-2">
                <div className="w-7 h-7 rounded-full bg-zinc-800 flex items-center justify-center">
                  <span className="text-xs font-bold text-zinc-400">{token.symbol[0]}</span>
                </div>
                <div>
                  <p className="text-xs font-medium text-zinc-200">{token.symbol}</p>
                  <p className="text-xs text-zinc-500">{token.name}</p>
                </div>
              </div>
              <div className="text-right">
                <p className="text-xs font-semibold text-zinc-200">{token.balance.toLocaleString()} {token.symbol}</p>
                {price && (
                  <p className="text-xs text-zinc-500">
                    ${usd.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
                  </p>
                )}
              </div>
            </div>
            {price && (
              <p className={`text-xs text-right ${price.change_24h_pct >= 0 ? "text-evap-green" : "text-evap-red"}`}>
                {price.change_24h_pct >= 0 ? "+" : ""}{price.change_24h_pct.toFixed(2)}% 24h
              </p>
            )}
            {totalUsd > 0 && (
              <div className="flex items-center gap-2 mt-1">
                <Bar pct={usd / totalUsd * 100} color="bg-zinc-600" />
                <span className="text-xs text-zinc-500 w-10 text-right">
                  {(usd / totalUsd * 100).toFixed(1)}%
                </span>
              </div>
            )}
          </div>
        ))}

        {/* Objects analytics */}
        <div className="px-4 py-4 rounded-xl bg-evap-surface border border-evap-border">
          <p className="text-xs text-zinc-400 font-semibold uppercase tracking-wider mb-3">On-Chain Objects</p>
          <div className="grid grid-cols-3 gap-2 mb-3">
            <div className="text-center">
              <p className="text-sm font-bold text-evap-green">{activeObjects}</p>
              <p className="text-xs text-zinc-500">Active</p>
            </div>
            <div className="text-center">
              <p className="text-sm font-bold text-yellow-500">{graceObjects}</p>
              <p className="text-xs text-zinc-500">Grace</p>
            </div>
            <div className="text-center">
              <p className="text-sm font-bold text-zinc-500">{ghostObjects}</p>
              <p className="text-xs text-zinc-500">Evaporated</p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-xs text-zinc-500">Total energy</span>
            <div className="flex-1 h-1.5 rounded-full bg-zinc-800 overflow-hidden">
              <div
                className="h-full rounded-full bg-evap-cyan"
                style={{ width: `${Math.min((activeObjects / Math.max(activeObjects + graceObjects + ghostObjects, 1)) * 100, 100)}%` }}
              />
            </div>
            <span className="text-xs text-zinc-400 font-mono">{totalEnergy.toLocaleString()}</span>
          </div>
        </div>

        {loading && (
          <p className="text-center text-xs text-zinc-600 py-2">Loading price data…</p>
        )}
      </div>
    </div>
  );
}

import { useState, useEffect, useCallback, useRef } from "react";
import { useWallet } from "@/hooks/useWallet";
import { formatBalance } from "@/utils/format";
import { api, type TokenInfo, type SwapQuote } from "@/utils/api";
import { Header } from "./Header";

const SLIPPAGE_OPTIONS = [0.5, 1, 3] as const;
const QUOTE_REFRESH_MS = 10_000;

/** Native EVAP token placeholder used when the API token list doesn't include it */
const EVAP_TOKEN: TokenInfo = {
  symbol: "EVAP",
  name: "EvaporChain",
  address: "native",
  decimals: 18,
  balance: 0,
};

type SwapStep = "form" | "confirming" | "success" | "error";

export function SwapScreen() {
  const {
    balance, setView, loading, error, setError,
    tokens, refreshTokens, swapTokens,
    notification, setNotification,
  } = useWallet();

  // ── Token list ──
  const [allTokens, setAllTokens] = useState<TokenInfo[]>([]);
  const [fromToken, setFromToken] = useState<TokenInfo | null>(null);
  const [toToken, setToToken] = useState<TokenInfo | null>(null);

  // ── Input ──
  const [amount, setAmount] = useState("");
  const [slippage, setSlippage] = useState<number>(0.5);
  const [customSlippage, setCustomSlippage] = useState("");
  const [showSlippage, setShowSlippage] = useState(false);

  // ── Quote ──
  const [quote, setQuote] = useState<SwapQuote | null>(null);
  const [quoteLoading, setQuoteLoading] = useState(false);
  const quoteTimer = useRef<ReturnType<typeof setInterval> | null>(null);

  // ── UI state ──
  const [step, setStep] = useState<SwapStep>("form");
  const [swapResult, setSwapResult] = useState<{ amountIn: number; amountOut: number; symbol: string } | null>(null);

  // ── Selectors ──
  const [showFromPicker, setShowFromPicker] = useState(false);
  const [showToPicker, setShowToPicker] = useState(false);

  // Load tokens on mount
  useEffect(() => {
    refreshTokens();
  }, [refreshTokens]);

  // Build token list from store, always including native EVAP
  useEffect(() => {
    const evap: TokenInfo = { ...EVAP_TOKEN, balance };
    const hasEvap = tokens.some(t => t.symbol === "EVAP");
    const list = hasEvap
      ? tokens.map(t => t.symbol === "EVAP" ? { ...t, balance } : t)
      : [evap, ...tokens];
    setAllTokens(list);

    // Default selection
    if (!fromToken) setFromToken(list[0] ?? null);
    if (!toToken && list.length > 1) setToToken(list[1] ?? null);
  }, [tokens, balance]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Fetch quote ──
  const fetchQuote = useCallback(async () => {
    if (!fromToken || !toToken || !amount || parseFloat(amount) <= 0) {
      setQuote(null);
      return;
    }
    setQuoteLoading(true);
    try {
      const q = await api.getSwapQuote(fromToken.symbol, toToken.symbol, parseFloat(amount));
      setQuote(q);
    } catch {
      setQuote(null);
    } finally {
      setQuoteLoading(false);
    }
  }, [fromToken, toToken, amount]);

  // Fetch quote when inputs change, and auto-refresh every 10s
  useEffect(() => {
    fetchQuote();

    if (quoteTimer.current) clearInterval(quoteTimer.current);
    if (fromToken && toToken && amount && parseFloat(amount) > 0) {
      quoteTimer.current = setInterval(fetchQuote, QUOTE_REFRESH_MS);
    }
    return () => {
      if (quoteTimer.current) clearInterval(quoteTimer.current);
    };
  }, [fetchQuote]);

  // Clear notification after 3s
  useEffect(() => {
    if (notification) {
      const t = setTimeout(() => setNotification(null), 3000);
      return () => clearTimeout(t);
    }
  }, [notification, setNotification]);

  // ── Handlers ──
  const handleFlip = () => {
    setFromToken(toToken);
    setToToken(fromToken);
    setAmount("");
    setQuote(null);
  };

  const handleSelectFrom = (token: TokenInfo) => {
    if (token.symbol === toToken?.symbol) {
      setToToken(fromToken);
    }
    setFromToken(token);
    setShowFromPicker(false);
    setQuote(null);
  };

  const handleSelectTo = (token: TokenInfo) => {
    if (token.symbol === fromToken?.symbol) {
      setFromToken(toToken);
    }
    setToToken(token);
    setShowToPicker(false);
    setQuote(null);
  };

  const handleSlippageChange = (val: number) => {
    setSlippage(val);
    setCustomSlippage("");
  };

  const handleCustomSlippage = (val: string) => {
    setCustomSlippage(val);
    const num = parseFloat(val);
    if (!isNaN(num) && num > 0 && num <= 50) {
      setSlippage(num);
    }
  };

  const handleMax = () => {
    if (fromToken) {
      const maxBalance = fromToken.symbol === "EVAP" ? balance : fromToken.balance;
      setAmount(String(maxBalance));
    }
  };

  const handleSwap = async () => {
    if (!fromToken || !toToken || !amount) return;
    setStep("confirming");
    const result = await swapTokens(fromToken.symbol, toToken.symbol, parseFloat(amount), slippage);
    if (result.success) {
      setSwapResult({ amountIn: result.amount_in, amountOut: result.amount_out, symbol: toToken.symbol });
      setStep("success");
    } else {
      setStep("error");
    }
  };

  const fromBalance = fromToken?.symbol === "EVAP" ? balance : (fromToken?.balance ?? 0);
  const priceImpactHigh = quote ? quote.price_impact > 2 : false;
  const canSwap = fromToken && toToken && amount && parseFloat(amount) > 0 && parseFloat(amount) <= fromBalance && quote && !loading;

  // ── Success state ──
  if (step === "success" && swapResult) {
    return (
      <div className="flex flex-col h-full">
        <Header />
        <div className="flex flex-col items-center justify-center flex-1 px-8">
          <div className="w-16 h-16 rounded-full bg-evap-green/20 flex items-center justify-center mb-4">
            <span className="text-3xl">✓</span>
          </div>
          <p className="text-sm font-semibold text-zinc-200">Swap Successful</p>
          <p className="text-xs text-zinc-500 mt-1">
            {swapResult.amountIn} {fromToken?.symbol} swapped for {swapResult.amountOut} {swapResult.symbol}
          </p>
          <button
            onClick={() => setView("home")}
            className="mt-6 px-6 py-2 rounded-lg bg-evap-surface border border-evap-border text-xs text-zinc-300 hover:border-evap-cyan/40 transition"
          >
            Back to Home
          </button>
        </div>
        <Footer />
      </div>
    );
  }

  // ── Error state ──
  if (step === "error") {
    return (
      <div className="flex flex-col h-full">
        <Header />
        <div className="flex flex-col items-center justify-center flex-1 px-8">
          <div className="w-16 h-16 rounded-full bg-evap-red/20 flex items-center justify-center mb-4">
            <span className="text-3xl">✕</span>
          </div>
          <p className="text-sm font-semibold text-zinc-200">Swap Failed</p>
          <p className="text-xs text-zinc-500 mt-1 text-center">{error ?? "Something went wrong"}</p>
          <button
            onClick={() => { setStep("form"); setError(null); }}
            className="mt-6 px-6 py-2 rounded-lg bg-evap-surface border border-evap-border text-xs text-zinc-300 hover:border-evap-cyan/40 transition"
          >
            Try Again
          </button>
        </div>
        <Footer />
      </div>
    );
  }

  // ── Token picker overlay ──
  if (showFromPicker || showToPicker) {
    const onSelect = showFromPicker ? handleSelectFrom : handleSelectTo;
    const excludeSymbol = showFromPicker ? toToken?.symbol : fromToken?.symbol;
    return (
      <div className="flex flex-col h-full">
        <Header />
        <div className="px-4 pt-4">
          <button
            onClick={() => { setShowFromPicker(false); setShowToPicker(false); }}
            className="text-xs text-zinc-500 hover:text-zinc-300 mb-3"
          >
            ← Back
          </button>
          <h2 className="text-lg font-semibold text-zinc-100 mb-3">Select Token</h2>
        </div>
        <div className="px-4 space-y-2 flex-1 overflow-y-auto">
          {allTokens.map(token => (
            <button
              key={token.symbol}
              onClick={() => onSelect(token)}
              disabled={token.symbol === excludeSymbol}
              className="w-full flex items-center gap-3 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border hover:border-evap-cyan/40 transition disabled:opacity-30 disabled:cursor-not-allowed"
            >
              <div className="w-8 h-8 rounded-full bg-gradient-to-br from-evap-cyan to-evap-purple flex items-center justify-center text-[10px] font-bold text-black shrink-0">
                {token.symbol.slice(0, 2)}
              </div>
              <div className="flex-1 text-left">
                <p className="text-xs font-semibold text-zinc-200">{token.name}</p>
                <p className="text-[10px] text-zinc-500">{token.symbol}</p>
              </div>
              <div className="text-right">
                <p className="text-xs text-zinc-300">{formatBalance(token.symbol === "EVAP" ? balance : token.balance)}</p>
              </div>
            </button>
          ))}
          {allTokens.length === 0 && (
            <p className="text-xs text-zinc-500 text-center py-8">No tokens available</p>
          )}
        </div>
        <Footer />
      </div>
    );
  }

  // ── Main form ──
  return (
    <div className="flex flex-col h-full">
      <Header />

      <div className="px-4 pt-4">
        <button
          onClick={() => setView("home")}
          className="text-xs text-zinc-500 hover:text-zinc-300 mb-3"
        >
          ← Back
        </button>
        <h2 className="text-lg font-semibold text-zinc-100 mb-1">Swap Tokens</h2>
        <p className="text-xs text-zinc-500 mb-4">Trade tokens on EvaporChain DEX</p>
      </div>

      <div className="px-4 space-y-1 flex-1 overflow-y-auto">
        {/* From token */}
        <div className="rounded-lg bg-evap-surface border border-evap-border p-3">
          <div className="flex items-center justify-between mb-2">
            <span className="text-[10px] text-zinc-500">From</span>
            <span className="text-[10px] text-zinc-500">
              Balance: {formatBalance(fromBalance)}
            </span>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={() => setShowFromPicker(true)}
              className="flex items-center gap-2 px-2 py-1.5 rounded-lg bg-evap-border/50 hover:bg-evap-border transition shrink-0"
            >
              <div className="w-5 h-5 rounded-full bg-gradient-to-br from-evap-cyan to-evap-purple flex items-center justify-center text-[8px] font-bold text-black">
                {fromToken?.symbol.slice(0, 2) ?? "?"}
              </div>
              <span className="text-xs font-semibold text-zinc-200">{fromToken?.symbol ?? "Select"}</span>
              <span className="text-[10px] text-zinc-500">▼</span>
            </button>
            <input
              type="number"
              placeholder="0.00"
              min="0"
              value={amount}
              onChange={e => setAmount(e.target.value)}
              className="flex-1 text-right bg-transparent text-lg font-semibold text-zinc-100 placeholder-zinc-600 focus:outline-none min-w-0"
            />
          </div>
          <div className="flex justify-end mt-1">
            <button
              onClick={handleMax}
              className="text-[10px] text-evap-cyan hover:underline"
            >
              MAX
            </button>
          </div>
        </div>

        {/* Flip button */}
        <div className="flex justify-center -my-1 relative z-10">
          <button
            onClick={handleFlip}
            className="w-8 h-8 rounded-full bg-evap-surface border border-evap-border flex items-center justify-center hover:border-evap-cyan/40 transition text-sm text-zinc-400 hover:text-evap-cyan"
          >
            ↕
          </button>
        </div>

        {/* To token */}
        <div className="rounded-lg bg-evap-surface border border-evap-border p-3">
          <div className="flex items-center justify-between mb-2">
            <span className="text-[10px] text-zinc-500">To</span>
            <span className="text-[10px] text-zinc-500">
              Balance: {formatBalance(toToken?.symbol === "EVAP" ? balance : (toToken?.balance ?? 0))}
            </span>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={() => setShowToPicker(true)}
              className="flex items-center gap-2 px-2 py-1.5 rounded-lg bg-evap-border/50 hover:bg-evap-border transition shrink-0"
            >
              <div className="w-5 h-5 rounded-full bg-gradient-to-br from-evap-purple to-evap-cyan flex items-center justify-center text-[8px] font-bold text-black">
                {toToken?.symbol.slice(0, 2) ?? "?"}
              </div>
              <span className="text-xs font-semibold text-zinc-200">{toToken?.symbol ?? "Select"}</span>
              <span className="text-[10px] text-zinc-500">▼</span>
            </button>
            <div className="flex-1 text-right text-lg font-semibold text-zinc-400 min-w-0">
              {quoteLoading ? (
                <span className="text-xs text-zinc-500">Fetching...</span>
              ) : quote ? (
                formatBalance(quote.amount_out)
              ) : (
                "0.00"
              )}
            </div>
          </div>
        </div>

        {/* Quote details */}
        {quote && (
          <div className="rounded-lg bg-evap-surface border border-evap-border p-3 space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-[10px] text-zinc-500">Rate</span>
              <span className="text-[10px] text-zinc-300">
                1 {fromToken?.symbol} = {quote.rate.toFixed(6)} {toToken?.symbol}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-[10px] text-zinc-500">Price Impact</span>
              <span className={`text-[10px] ${priceImpactHigh ? "text-evap-red font-semibold" : "text-zinc-300"}`}>
                {quote.price_impact.toFixed(2)}%
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-[10px] text-zinc-500">Energy Cost</span>
              <span className="text-[10px] text-evap-amber">
                {quote.energy_cost} energy
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-[10px] text-zinc-500">Fee</span>
              <span className="text-[10px] text-zinc-300">
                {quote.estimated_fee} {fromToken?.symbol}
              </span>
            </div>
          </div>
        )}

        {/* Price impact warning */}
        {priceImpactHigh && quote && (
          <div className="px-3 py-2 rounded-lg bg-evap-red/10 border border-evap-red/30">
            <p className="text-[10px] text-evap-red text-center">
              High price impact ({quote.price_impact.toFixed(2)}%). You may receive significantly less than expected.
            </p>
          </div>
        )}

        {/* Energy cost callout */}
        {quote && (
          <div className="px-3 py-2 rounded-lg bg-evap-amber/10 border border-evap-amber/20">
            <p className="text-[10px] text-evap-amber text-center">
              This swap costs {quote.energy_cost} energy
            </p>
          </div>
        )}

        {/* Slippage setting */}
        <div className="rounded-lg bg-evap-surface border border-evap-border p-3">
          <button
            onClick={() => setShowSlippage(!showSlippage)}
            className="w-full flex items-center justify-between"
          >
            <span className="text-[10px] text-zinc-500">Slippage Tolerance</span>
            <span className="text-[10px] text-zinc-300">{slippage}% ▼</span>
          </button>
          {showSlippage && (
            <div className="flex items-center gap-2 mt-2">
              {SLIPPAGE_OPTIONS.map(opt => (
                <button
                  key={opt}
                  onClick={() => handleSlippageChange(opt)}
                  className={`flex-1 py-1.5 rounded text-[10px] font-medium transition ${
                    slippage === opt && !customSlippage
                      ? "bg-evap-cyan/20 text-evap-cyan border border-evap-cyan/40"
                      : "bg-evap-border/50 text-zinc-400 hover:text-zinc-300"
                  }`}
                >
                  {opt}%
                </button>
              ))}
              <input
                type="number"
                placeholder="Custom"
                value={customSlippage}
                onChange={e => handleCustomSlippage(e.target.value)}
                className="flex-1 px-2 py-1.5 rounded bg-evap-border/50 text-[10px] text-zinc-200 placeholder-zinc-600 focus:outline-none focus:ring-1 focus:ring-evap-cyan/40 text-center min-w-0"
              />
            </div>
          )}
        </div>

        {/* Amount exceeds balance warning */}
        {amount && parseFloat(amount) > fromBalance && (
          <p className="text-[10px] text-evap-red text-center">Insufficient {fromToken?.symbol} balance</p>
        )}

        {/* Error */}
        {error && step === "form" && (
          <p className="text-xs text-evap-red text-center">{error}</p>
        )}

        {/* Swap button */}
        <button
          onClick={handleSwap}
          disabled={!canSwap}
          className="w-full py-3 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-sm font-semibold text-black hover:opacity-90 transition disabled:opacity-50"
        >
          {loading || step === "confirming"
            ? "Swapping..."
            : !fromToken || !toToken
              ? "Select Tokens"
              : !amount || parseFloat(amount) <= 0
                ? "Enter Amount"
                : parseFloat(amount) > fromBalance
                  ? "Insufficient Balance"
                  : priceImpactHigh
                    ? "Swap Anyway"
                    : "Swap"}
        </button>
      </div>

      <Footer />
    </div>
  );
}

function Footer() {
  return (
    <div className="px-4 py-3 mt-auto">
      <p className="text-[9px] text-zinc-600 text-center">
        Powered by EvaporChain DEX
      </p>
    </div>
  );
}

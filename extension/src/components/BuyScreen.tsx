import { useState, useEffect, useCallback, useRef } from "react";
import { ChevronDown } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import { Header } from "./Header";
import { FiatProviderWidget, type FiatProvider } from "./FiatProviderWidget";

// ── Constants ──

type FiatCurrency = "USD" | "EUR" | "GBP";
type PaymentMethod = "card" | "apple_pay" | "google_pay";

const FIAT_CURRENCIES: FiatCurrency[] = ["USD", "EUR", "GBP"];

const CURRENCY_SYMBOLS: Record<FiatCurrency, string> = {
  USD: "$",
  EUR: "\u20AC",
  GBP: "\u00A3",
};

const PAYMENT_METHODS: { id: PaymentMethod; label: string; icon: string }[] = [
  { id: "card", label: "Credit Card", icon: "Card" },
  { id: "apple_pay", label: "Apple Pay", icon: "Apple" },
  { id: "google_pay", label: "Google Pay", icon: "G Pay" },
];

const PROVIDERS: { id: FiatProvider; label: string }[] = [
  { id: "moonpay", label: "MoonPay" },
  { id: "transak", label: "Transak" },
];

/** Simulated EVAP/USD rate. In production this comes from the provider API. */
const BASE_EVAP_RATE = 0.045; // 1 EVAP = $0.045

const RATE_REFRESH_MS = 30_000;

const NETWORK_FEE_PERCENT = 0.5;
const PROVIDER_FEE_PERCENT: Record<FiatProvider, number> = {
  moonpay: 3.5,
  transak: 2.9,
};

type BuyStep = "form" | "widget" | "success" | "error";

// ── Component ──

export function BuyScreen() {
  const { activeAccount, setView } = useWallet();

  // Form state
  const [fiatAmount, setFiatAmount] = useState("");
  const [currency, setCurrency] = useState<FiatCurrency>("USD");
  const [paymentMethod, setPaymentMethod] = useState<PaymentMethod>("card");
  const [provider, setProvider] = useState<FiatProvider>("moonpay");
  const [showCurrencyPicker, setShowCurrencyPicker] = useState(false);

  // Rate
  const [evapRate, setEvapRate] = useState(BASE_EVAP_RATE);
  const [rateLoading, setRateLoading] = useState(false);
  const [lastRateRefresh, setLastRateRefresh] = useState<Date>(new Date());
  const rateTimer = useRef<ReturnType<typeof setInterval> | null>(null);

  // UI
  const [step, setStep] = useState<BuyStep>("form");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // ── Rate refresh ──
  const refreshRate = useCallback(() => {
    setRateLoading(true);
    // Simulated rate fetch — in production this calls the provider's quote API
    // Adding slight jitter to simulate live pricing
    setTimeout(() => {
      const jitter = (Math.random() - 0.5) * 0.002;
      setEvapRate(BASE_EVAP_RATE + jitter);
      setLastRateRefresh(new Date());
      setRateLoading(false);
    }, 300);
  }, []);

  useEffect(() => {
    refreshRate();
    rateTimer.current = setInterval(refreshRate, RATE_REFRESH_MS);
    return () => {
      if (rateTimer.current) clearInterval(rateTimer.current);
    };
  }, [refreshRate]);

  // ── Derived values ──
  const parsedAmount = parseFloat(fiatAmount) || 0;
  const evapAmount = parsedAmount > 0 ? parsedAmount / evapRate : 0;
  const networkFee = parsedAmount * (NETWORK_FEE_PERCENT / 100);
  const providerFee = parsedAmount * (PROVIDER_FEE_PERCENT[provider] / 100);
  const totalCost = parsedAmount + networkFee + providerFee;
  const canBuy = parsedAmount >= 10 && !!activeAccount;

  // ── Handlers ──
  const handleBuy = () => {
    if (!canBuy) return;
    setStep("widget");
  };

  const handleWidgetSuccess = () => {
    setStep("success");
  };

  const handleWidgetError = (msg: string) => {
    setErrorMessage(msg);
    setStep("error");
  };

  const handleWidgetClose = () => {
    setStep("form");
  };

  if (!activeAccount) return null;

  // ── Provider widget ──
  if (step === "widget") {
    return (
      <FiatProviderWidget
        provider={provider}
        walletAddress={activeAccount.address}
        fiatAmount={parsedAmount}
        fiatCurrency={currency}
        onClose={handleWidgetClose}
        onSuccess={handleWidgetSuccess}
        onError={handleWidgetError}
      />
    );
  }

  // ── Success state ──
  if (step === "success") {
    return (
      <div className="flex flex-col h-full">
        <Header />
        <div className="flex flex-col items-center justify-center flex-1 px-8">
          <div className="w-16 h-16 rounded-full bg-emerald-500/20 flex items-center justify-center mb-4">
            <span className="text-3xl text-emerald-500">&#10003;</span>
          </div>
          <p className="text-sm font-semibold text-zinc-200">
            Purchase Initiated!
          </p>
          <p className="text-xs text-zinc-500 mt-1 text-center">
            EVAP will arrive in ~5 minutes.
          </p>
          <p className="text-xs text-zinc-500 mt-1 text-center">
            You will receive approximately{" "}
            <span className="text-zinc-300 font-medium">
              {formatEvap(evapAmount)} EVAP
            </span>
          </p>
          <button
            onClick={() => setView("home")}
            className="mt-6 px-6 py-2 rounded-lg bg-evap-surface border border-evap-border text-xs text-zinc-300 hover:border-emerald-500/40 transition"
          >
            Back to Home
          </button>
        </div>
        <Footer provider={provider} />
      </div>
    );
  }

  // ── Error state ──
  if (step === "error") {
    return (
      <div className="flex flex-col h-full">
        <Header />
        <div className="flex flex-col items-center justify-center flex-1 px-8">
          <div className="w-16 h-16 rounded-full bg-red-500/20 flex items-center justify-center mb-4">
            <span className="text-3xl text-red-400">&#10007;</span>
          </div>
          <p className="text-sm font-semibold text-zinc-200">Purchase Failed</p>
          <p className="text-xs text-zinc-500 mt-1 text-center">
            {errorMessage ?? "Something went wrong. Please try again."}
          </p>
          <button
            onClick={() => {
              setStep("form");
              setErrorMessage(null);
            }}
            className="mt-6 px-6 py-2 rounded-lg bg-evap-surface border border-evap-border text-xs text-zinc-300 hover:border-emerald-500/40 transition"
          >
            Try Again
          </button>
        </div>
        <Footer provider={provider} />
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
          &larr; Back
        </button>
        <h2 className="text-lg font-semibold text-zinc-100 mb-1">Buy EVAP</h2>
        <p className="text-xs text-zinc-500 mb-4">
          Purchase EVAP tokens with fiat currency
        </p>
      </div>

      <div className="px-4 space-y-3 flex-1 overflow-y-auto pb-4">
        {/* Amount input */}
        <div className="rounded-lg bg-evap-surface border border-evap-border p-3">
          <div className="flex items-center justify-between mb-2">
            <span className="text-xs text-zinc-500">You pay</span>
            <button
              onClick={() => setShowCurrencyPicker(!showCurrencyPicker)}
              className="inline-flex items-center gap-1 text-xs text-zinc-400 hover:text-zinc-300 transition"
            >
              {currency}
              <ChevronDown className="w-3 h-3" strokeWidth={1.5} />
            </button>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-lg text-zinc-500 font-medium">
              {CURRENCY_SYMBOLS[currency]}
            </span>
            <input
              type="number"
              placeholder="0.00"
              min="10"
              step="1"
              value={fiatAmount}
              onChange={(e) => setFiatAmount(e.target.value)}
              className="flex-1 bg-transparent text-lg font-semibold text-zinc-100 placeholder-zinc-600 focus:outline-none min-w-0"
            />
          </div>
          {parsedAmount > 0 && parsedAmount < 10 && (
            <p className="text-xs text-red-400 mt-1">
              Minimum purchase is {CURRENCY_SYMBOLS[currency]}10
            </p>
          )}

          {/* Currency picker dropdown */}
          {showCurrencyPicker && (
            <div className="mt-2 flex gap-2">
              {FIAT_CURRENCIES.map((c) => (
                <button
                  key={c}
                  onClick={() => {
                    setCurrency(c);
                    setShowCurrencyPicker(false);
                  }}
                  className={`flex-1 py-1.5 rounded text-xs font-medium transition ${
                    currency === c
                      ? "bg-emerald-500/20 text-emerald-400 border border-emerald-500/40"
                      : "bg-evap-border/50 text-zinc-400 hover:text-zinc-300"
                  }`}
                >
                  {CURRENCY_SYMBOLS[c]} {c}
                </button>
              ))}
            </div>
          )}
        </div>

        {/* EVAP amount preview */}
        <div className="rounded-lg bg-evap-surface border border-evap-border p-3">
          <div className="flex items-center justify-between mb-2">
            <span className="text-xs text-zinc-500">You receive</span>
            <span className="text-xs text-zinc-500">EVAP</span>
          </div>
          <div className="flex items-center gap-2">
            <div className="w-6 h-6 rounded-full bg-gradient-to-br from-evap-cyan to-evap-purple flex items-center justify-center text-[8px] font-bold text-black shrink-0">
              EV
            </div>
            <span className="text-lg font-semibold text-zinc-300">
              {rateLoading ? (
                <span className="text-xs text-zinc-500">Updating...</span>
              ) : parsedAmount > 0 ? (
                formatEvap(evapAmount)
              ) : (
                "0.00"
              )}
            </span>
          </div>
          <p className="text-xs text-zinc-600 mt-1">
            1 EVAP = {CURRENCY_SYMBOLS[currency]}
            {evapRate.toFixed(4)} {currency}
            {rateLoading && " (refreshing...)"}
          </p>
        </div>

        {/* Payment method selector */}
        <div className="rounded-lg bg-evap-surface border border-evap-border p-3">
          <span className="text-xs text-zinc-500 mb-2 block">
            Payment method
          </span>
          <div className="flex gap-2">
            {PAYMENT_METHODS.map((pm) => (
              <button
                key={pm.id}
                onClick={() => setPaymentMethod(pm.id)}
                className={`flex-1 flex flex-col items-center gap-1 py-2 rounded-lg border transition text-xs font-medium ${
                  paymentMethod === pm.id
                    ? "border-emerald-500/50 bg-emerald-500/10 text-emerald-400"
                    : "border-evap-border bg-evap-surface text-zinc-400 hover:border-zinc-500"
                }`}
              >
                <span className="text-xs">{pm.icon}</span>
                <span>{pm.label}</span>
              </button>
            ))}
          </div>
        </div>

        {/* Provider toggle */}
        <div className="rounded-lg bg-evap-surface border border-evap-border p-3">
          <span className="text-xs text-zinc-500 mb-2 block">
            Provider
          </span>
          <div className="flex rounded-lg bg-evap-border/50 p-0.5">
            {PROVIDERS.map((p) => (
              <button
                key={p.id}
                onClick={() => setProvider(p.id)}
                className={`flex-1 py-2 rounded-md text-xs font-medium transition ${
                  provider === p.id
                    ? "bg-evap-surface text-emerald-400 shadow-sm"
                    : "text-zinc-500 hover:text-zinc-300"
                }`}
              >
                {p.label}
              </button>
            ))}
          </div>
        </div>

        {/* Fee breakdown */}
        {parsedAmount > 0 && (
          <div className="rounded-lg bg-evap-surface border border-evap-border p-3 space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-xs text-zinc-500">Subtotal</span>
              <span className="text-xs text-zinc-300">
                {CURRENCY_SYMBOLS[currency]}
                {parsedAmount.toFixed(2)}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-xs text-zinc-500">Network fee</span>
              <span className="text-xs text-zinc-300">
                {CURRENCY_SYMBOLS[currency]}
                {networkFee.toFixed(2)}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span className="text-xs text-zinc-500">
                {PROVIDERS.find((p) => p.id === provider)?.label} fee (
                {PROVIDER_FEE_PERCENT[provider]}%)
              </span>
              <span className="text-xs text-zinc-300">
                {CURRENCY_SYMBOLS[currency]}
                {providerFee.toFixed(2)}
              </span>
            </div>
            <div className="border-t border-evap-border pt-2 flex items-center justify-between">
              <span className="text-xs text-zinc-400 font-medium">
                Total
              </span>
              <span className="text-xs text-zinc-200 font-semibold">
                {CURRENCY_SYMBOLS[currency]}
                {totalCost.toFixed(2)}
              </span>
            </div>
          </div>
        )}

        {/* KYC notice */}
        <div className="px-3 py-2 rounded-lg bg-amber-500/5 border border-amber-500/20">
          <p className="text-xs text-amber-400/80 text-center">
            Identity verification handled by{" "}
            {PROVIDERS.find((p) => p.id === provider)?.label}
          </p>
        </div>

        {/* Buy button */}
        <button
          onClick={handleBuy}
          disabled={!canBuy}
          className="w-full py-3 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-sm font-semibold text-white transition disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {parsedAmount <= 0
            ? "Enter Amount"
            : parsedAmount < 10
              ? `Minimum ${CURRENCY_SYMBOLS[currency]}10`
              : `Buy with ${PROVIDERS.find((p) => p.id === provider)?.label}`}
        </button>
      </div>

      <Footer provider={provider} />
    </div>
  );
}

// ── Helpers ──

function formatEvap(amount: number): string {
  if (amount >= 1_000_000) return `${(amount / 1_000_000).toFixed(2)}M`;
  if (amount >= 1_000) return `${(amount / 1_000).toFixed(2)}K`;
  return amount.toFixed(2);
}

function Footer({ provider }: { provider: FiatProvider }) {
  const label = provider === "moonpay" ? "MoonPay" : "Transak";
  return (
    <div className="px-4 py-3 mt-auto">
      <p className="text-[9px] text-zinc-600 text-center">
        Powered by {label}
      </p>
    </div>
  );
}

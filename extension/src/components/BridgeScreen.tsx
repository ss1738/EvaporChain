import { useState, useEffect, useCallback } from "react";
import { Check, X } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import {
  bridgeManager,
  SUPPORTED_CHAINS,
  SUPPORTED_ASSETS,
  type Chain,
  type Asset,
  type BridgeQuote,
  type BridgeTransfer,
  type BridgeStep,
} from "@/utils/bridge";
import { HlwaCard } from "./HlwaCard";

type BridgeTab = "transfer" | "history";

export function BridgeScreen() {
  const { setView, activeAccount, balance, setNotification, setError, bridgeTransfers } = useWallet();

  const [tab, setTab] = useState<BridgeTab>("transfer");
  const [fromChain, setFromChain] = useState<string>("evaporchain");
  const [toChain, setToChain] = useState<string>("ethereum");
  const [selectedAsset, setSelectedAsset] = useState<string>("EVAP");
  const [amount, setAmount] = useState<string>("");
  const [quote, setQuote] = useState<BridgeQuote | null>(null);
  const [loading, setLoading] = useState(false);
  const [quoteLoading, setQuoteLoading] = useState(false);
  const [history, setHistory] = useState<BridgeTransfer[]>([]);
  const [activeBridges, setActiveBridges] = useState<BridgeTransfer[]>([]);

  const chains = SUPPORTED_CHAINS;
  const assets = SUPPORTED_ASSETS;

  const destinationChains = chains.filter((c) => c.id !== fromChain);

  // Auto-correct toChain if it matches fromChain
  useEffect(() => {
    if (toChain === fromChain) {
      const alt = destinationChains[0];
      if (alt) setToChain(alt.id);
    }
  }, [fromChain, toChain, destinationChains]);

  // Fetch quote when parameters change
  useEffect(() => {
    const numAmount = parseFloat(amount);
    if (!amount || isNaN(numAmount) || numAmount <= 0) {
      setQuote(null);
      return;
    }

    const timeout = setTimeout(async () => {
      setQuoteLoading(true);
      try {
        const q = await bridgeManager.getQuote(fromChain, toChain, selectedAsset, numAmount);
        setQuote(q);
      } catch {
        setQuote(null);
      }
      setQuoteLoading(false);
    }, 300);

    return () => clearTimeout(timeout);
  }, [fromChain, toChain, selectedAsset, amount]);

  // Load history
  useEffect(() => {
    if (tab === "history" && activeAccount) {
      bridgeManager.getBridgeHistory(activeAccount.address).then(setHistory).catch(() => {});
    }
  }, [tab, activeAccount]);

  const handleMax = () => {
    if (fromChain === "evaporchain") {
      setAmount(Math.max(0, balance - 0.01).toString());
    }
  };

  const handleBridge = async () => {
    const numAmount = parseFloat(amount);
    if (!numAmount || !activeAccount) return;

    setLoading(true);
    try {
      const result = await bridgeManager.initiateBridge({
        fromChain,
        toChain,
        asset: selectedAsset,
        amount: numAmount,
        sourceAddress: activeAccount.address,
        destinationAddress: activeAccount.address,
      });

      if (result.success) {
        setNotification(`Bridge initiated! TX: ${result.bridgeTxId.slice(0, 12)}...`);
        setAmount("");
        setQuote(null);

        // Track the active bridge
        const status = await bridgeManager.getBridgeStatus(result.bridgeTxId);
        setActiveBridges((prev) => [status, ...prev]);
      } else {
        setError(result.message ?? "Bridge failed");
      }
    } catch (err: any) {
      setError(err.message);
    }
    setLoading(false);
  };

  const getChainColor = (chainId: string): string => {
    const chain = chains.find((c) => c.id === chainId);
    return chain?.color ?? "#71717a";
  };

  const getChainName = (chainId: string): string => {
    const chain = chains.find((c) => c.id === chainId);
    return chain?.name ?? chainId;
  };

  return (
    <div className="flex flex-col h-full bg-white">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-zinc-200">
        <button
          onClick={() => setView("home")}
          className="text-zinc-500 hover:text-zinc-700 transition text-sm"
        >
          &larr;
        </button>
        <h1 className="text-sm font-semibold text-zinc-800">Bridge</h1>
      </div>

      {/* Warning banner */}
      <div className="mx-4 mt-3 px-3 py-2 rounded-lg bg-amber-50 border border-amber-200">
        <p className="text-xs text-amber-700">
          Bridge transfers are irreversible. Verify the destination chain and address.
        </p>
      </div>

      {/* Tabs */}
      <div className="flex mx-4 mt-3 rounded-lg bg-zinc-100 p-0.5">
        <TabButton
          active={tab === "transfer"}
          label="Transfer"
          onClick={() => setTab("transfer")}
        />
        <TabButton
          active={tab === "history"}
          label="History"
          onClick={() => setTab("history")}
        />
      </div>

      <div className="flex-1 overflow-y-auto px-4 py-3 space-y-4">
        {tab === "transfer" && (
          <>
            {/* HLWA wrapped-asset live state */}
            <HlwaCard />

            {/* Source chain */}
            <div>
              <label className="text-xs font-medium text-zinc-500 uppercase tracking-wide">
                From
              </label>
              <div className="flex gap-1.5 mt-1.5">
                {chains.map((chain) => (
                  <ChainPill
                    key={chain.id}
                    chain={chain}
                    selected={fromChain === chain.id}
                    onClick={() => setFromChain(chain.id)}
                  />
                ))}
              </div>
            </div>

            {/* Swap direction indicator */}
            <div className="flex justify-center">
              <div className="w-8 h-8 rounded-full bg-zinc-100 flex items-center justify-center">
                <span className="text-zinc-400 text-sm">&#8595;</span>
              </div>
            </div>

            {/* Destination chain */}
            <div>
              <label className="text-xs font-medium text-zinc-500 uppercase tracking-wide">
                To
              </label>
              <div className="flex gap-1.5 mt-1.5">
                {destinationChains.map((chain) => (
                  <ChainPill
                    key={chain.id}
                    chain={chain}
                    selected={toChain === chain.id}
                    onClick={() => setToChain(chain.id)}
                  />
                ))}
              </div>
            </div>

            {/* Asset selector */}
            <div>
              <label className="text-xs font-medium text-zinc-500 uppercase tracking-wide">
                Asset
              </label>
              <div className="flex gap-1.5 mt-1.5">
                {assets.map((asset) => (
                  <button
                    key={asset.symbol}
                    onClick={() => setSelectedAsset(asset.symbol)}
                    className={`px-3 py-1.5 rounded-full text-xs font-medium transition ${
                      selectedAsset === asset.symbol
                        ? "bg-cyan-600 text-white"
                        : "bg-zinc-100 text-zinc-600 hover:bg-zinc-200"
                    }`}
                  >
                    {asset.symbol}
                  </button>
                ))}
              </div>
            </div>

            {/* Amount input */}
            <div>
              <div className="flex items-center justify-between mb-1">
                <label className="text-xs font-medium text-zinc-500 uppercase tracking-wide">
                  Amount
                </label>
                {fromChain === "evaporchain" && (
                  <span className="text-xs text-zinc-400">
                    Balance: {balance.toFixed(2)} EVAP
                  </span>
                )}
              </div>
              <div className="flex items-center gap-2 border border-zinc-200 rounded-lg px-3 py-2 focus-within:border-cyan-400 transition">
                <input
                  type="number"
                  value={amount}
                  onChange={(e) => setAmount(e.target.value)}
                  placeholder="0.00"
                  className="flex-1 text-sm text-zinc-800 bg-transparent outline-none [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
                />
                <span className="text-xs text-zinc-400">{selectedAsset}</span>
                {fromChain === "evaporchain" && (
                  <button
                    onClick={handleMax}
                    className="text-xs font-semibold text-cyan-600 hover:text-cyan-700 transition"
                  >
                    MAX
                  </button>
                )}
              </div>
            </div>

            {/* Quote preview */}
            {quoteLoading && (
              <div className="px-3 py-3 rounded-lg bg-zinc-50 border border-zinc-200 text-center">
                <p className="text-xs text-zinc-400">Fetching bridge quote...</p>
              </div>
            )}

            {quote && !quoteLoading && (
              <div className="px-3 py-3 rounded-lg bg-zinc-50 border border-zinc-200 space-y-2">
                <h4 className="text-xs font-semibold text-zinc-500 uppercase tracking-wide">
                  Route Preview
                </h4>
                <QuoteRow label="Estimated time" value={`~${quote.estimatedTimeMinutes} minutes`} />
                <QuoteRow label="Bridge fee" value={`${quote.feePercent}% (${quote.fee.toFixed(4)} ${selectedAsset})`} />
                <QuoteRow
                  label="You receive"
                  value={`${quote.destinationAmount.toFixed(4)} ${selectedAsset}`}
                  highlight
                />
                <QuoteRow label="Provider" value={quote.provider} />
              </div>
            )}

            {/* Bridge button */}
            <button
              onClick={handleBridge}
              disabled={loading || !quote || !amount}
              className="w-full py-2.5 rounded-lg bg-cyan-600 text-white text-sm font-medium hover:bg-cyan-700 transition disabled:opacity-40 disabled:cursor-not-allowed"
            >
              {loading ? "Bridging..." : "Bridge"}
            </button>

            {/* Active bridge transfers */}
            {activeBridges.length > 0 && (
              <div>
                <h3 className="text-xs font-semibold text-zinc-700 mb-2">Active Transfers</h3>
                <div className="space-y-3">
                  {activeBridges.map((transfer) => (
                    <ActiveBridgeCard
                      key={transfer.bridgeTxId}
                      transfer={transfer}
                      getChainName={getChainName}
                      getChainColor={getChainColor}
                    />
                  ))}
                </div>
              </div>
            )}
          </>
        )}

        {tab === "history" && (
          <div>
            {history.length === 0 && bridgeTransfers.length === 0 ? (
              <div className="text-center py-12">
                <p className="text-zinc-400 text-sm">No bridge history yet.</p>
                <p className="text-zinc-300 text-xs mt-1">
                  Your completed transfers will appear here.
                </p>
              </div>
            ) : (
              <div className="space-y-2">
                {[...bridgeTransfers, ...history].map((transfer) => (
                  <HistoryCard
                    key={transfer.bridgeTxId}
                    transfer={transfer}
                    getChainName={getChainName}
                    getChainColor={getChainColor}
                  />
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Post-quantum footer */}
      <div className="px-4 py-2 border-t border-zinc-200">
        <p className="text-xs text-zinc-400 text-center">
          Cross-chain bridge secured by ML-DSA post-quantum signatures
        </p>
      </div>
    </div>
  );
}

// ── Sub-components ──

function TabButton({
  active,
  label,
  onClick,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex-1 py-1.5 rounded-md text-xs font-medium transition ${
        active ? "bg-white text-zinc-800 shadow-sm" : "text-zinc-500 hover:text-zinc-700"
      }`}
    >
      {label}
    </button>
  );
}

function ChainPill({
  chain,
  selected,
  onClick,
}: {
  chain: Chain;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`px-3 py-1.5 rounded-full text-xs font-medium transition border ${
        selected
          ? "text-white border-transparent"
          : "bg-white text-zinc-600 border-zinc-200 hover:border-zinc-300"
      }`}
      style={selected ? { backgroundColor: chain.color, borderColor: chain.color } : undefined}
    >
      {chain.name}
    </button>
  );
}

function QuoteRow({
  label,
  value,
  highlight,
}: {
  label: string;
  value: string;
  highlight?: boolean;
}) {
  return (
    <div className="flex justify-between items-center">
      <span className="text-xs text-zinc-500">{label}</span>
      <span
        className={`text-xs font-mono ${
          highlight ? "text-cyan-700 font-semibold" : "text-zinc-700"
        }`}
      >
        {value}
      </span>
    </div>
  );
}

function ActiveBridgeCard({
  transfer,
  getChainName,
  getChainColor,
}: {
  transfer: BridgeTransfer;
  getChainName: (id: string) => string;
  getChainColor: (id: string) => string;
}) {
  return (
    <div className="px-3 py-3 rounded-lg border border-zinc-200 bg-white space-y-3">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          <span
            className="w-2 h-2 rounded-full"
            style={{ backgroundColor: getChainColor(transfer.fromChain) }}
          />
          <span className="text-xs text-zinc-600">{getChainName(transfer.fromChain)}</span>
          <span className="text-xs text-zinc-300 mx-1">&rarr;</span>
          <span
            className="w-2 h-2 rounded-full"
            style={{ backgroundColor: getChainColor(transfer.toChain) }}
          />
          <span className="text-xs text-zinc-600">{getChainName(transfer.toChain)}</span>
        </div>
        <span className="text-xs font-semibold text-zinc-800">
          {transfer.amount} {transfer.asset}
        </span>
      </div>

      {/* Step indicators */}
      <div className="space-y-1.5">
        {transfer.steps.map((step, idx) => (
          <StepRow key={idx} step={step} />
        ))}
      </div>

      {/* Explorer links */}
      <div className="flex gap-3">
        {transfer.sourceTxHash && (
          <a
            href="#"
            className="text-xs text-cyan-600 hover:underline"
            onClick={(e) => e.preventDefault()}
          >
            View on {getChainName(transfer.fromChain)}
          </a>
        )}
        {transfer.destinationTxHash && (
          <a
            href="#"
            className="text-xs text-cyan-600 hover:underline"
            onClick={(e) => e.preventDefault()}
          >
            View on {getChainName(transfer.toChain)}
          </a>
        )}
      </div>
    </div>
  );
}

function StepRow({ step }: { step: BridgeStep }) {
  return (
    <div className="flex items-center gap-2">
      {/* Status icon */}
      {step.status === "completed" && (
        <div className="w-4 h-4 rounded-full bg-emerald-100 flex items-center justify-center shrink-0">
          <Check className="w-2.5 h-2.5 text-emerald-600" strokeWidth={3} />
        </div>
      )}
      {step.status === "in_progress" && (
        <div className="w-4 h-4 rounded-full border border-cyan-400 border-t-transparent animate-spin shrink-0" />
      )}
      {step.status === "pending" && (
        <div className="w-4 h-4 rounded-full bg-zinc-100 shrink-0" />
      )}
      {step.status === "failed" && (
        <div className="w-4 h-4 rounded-full bg-red-100 flex items-center justify-center shrink-0">
          <X className="w-2.5 h-2.5 text-red-600" strokeWidth={3} />
        </div>
      )}

      <span
        className={`text-xs ${
          step.status === "completed"
            ? "text-zinc-700"
            : step.status === "in_progress"
            ? "text-cyan-700 font-medium"
            : "text-zinc-400"
        }`}
      >
        {step.label}
      </span>

      {step.timestamp && (
        <span className="text-[9px] text-zinc-300 ml-auto">
          {new Date(step.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
        </span>
      )}
    </div>
  );
}

function HistoryCard({
  transfer,
  getChainName,
  getChainColor,
}: {
  transfer: BridgeTransfer;
  getChainName: (id: string) => string;
  getChainColor: (id: string) => string;
}) {
  return (
    <div className="px-3 py-2.5 rounded-lg border border-zinc-200 bg-white">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          <span
            className="w-2 h-2 rounded-full"
            style={{ backgroundColor: getChainColor(transfer.fromChain) }}
          />
          <span className="text-xs text-zinc-500">{getChainName(transfer.fromChain)}</span>
          <span className="text-xs text-zinc-300 mx-0.5">&rarr;</span>
          <span
            className="w-2 h-2 rounded-full"
            style={{ backgroundColor: getChainColor(transfer.toChain) }}
          />
          <span className="text-xs text-zinc-500">{getChainName(transfer.toChain)}</span>
        </div>
        <div className="text-right">
          <p className="text-xs font-semibold text-zinc-800">
            {transfer.amount} {transfer.asset}
          </p>
        </div>
      </div>
      <div className="flex items-center justify-between mt-1">
        <span className="text-[9px] text-zinc-400">
          {new Date(transfer.initiatedAt).toLocaleDateString()}
        </span>
        <span
          className={`text-[9px] font-medium ${
            transfer.status === "completed"
              ? "text-emerald-600"
              : transfer.status === "failed"
              ? "text-red-500"
              : "text-amber-500"
          }`}
        >
          {transfer.status.charAt(0).toUpperCase() + transfer.status.slice(1)}
        </span>
      </div>
    </div>
  );
}

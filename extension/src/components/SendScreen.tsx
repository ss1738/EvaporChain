import { useState } from "react";
import { useWallet } from "@/hooks/useWallet";
import { formatBalance } from "@/utils/format";
import { Header } from "./Header";
import { TxSimulation } from "./TxSimulation";
import { LadVmPreview } from "./LadVmPreview";

type SendStep = "form" | "preview" | "sent";

export function SendScreen() {
  const { sendTransfer, balance, setView, loading, error, chainStatus } = useWallet();
  const [to, setTo] = useState("");
  const [amount, setAmount] = useState("");
  const [step, setStep] = useState<SendStep>("form");

  const parsedAmount = parseInt(amount, 10);
  const isFormValid = to.length > 0 && !isNaN(parsedAmount) && parsedAmount > 0;

  const handlePreview = (e: React.FormEvent) => {
    e.preventDefault();
    if (!isFormValid) return;
    setStep("preview");
  };

  const handleConfirm = async () => {
    if (!to || !amount) return;
    const result = await sendTransfer(to, parsedAmount);
    if (result.success) {
      setStep("sent");
      setTimeout(() => setView("home"), 2000);
    }
  };

  const handleCancelPreview = () => {
    setStep("form");
  };

  // Success screen
  if (step === "sent") {
    return (
      <div className="flex flex-col h-full">
        <Header />
        <div className="flex flex-col items-center justify-center flex-1 px-8">
          <div className="w-16 h-16 rounded-full bg-evap-green/20 flex items-center justify-center mb-4">
            <span className="text-3xl">&#10003;</span>
          </div>
          <p className="text-sm font-semibold text-zinc-200">Transaction Sent</p>
          <p className="text-xs text-zinc-500 mt-1">{amount} EVAP sent</p>
        </div>
      </div>
    );
  }

  // Preview / simulation step
  if (step === "preview") {
    const activeAddress = useWallet.getState().activeAccount?.address ?? "";
    const currentEpoch = chainStatus?.epoch ?? 0;
    // Default epoch duration: 30 seconds (configurable per chain)
    const epochDurationMs = 30_000;

    return (
      <div className="flex flex-col h-full">
        <Header />
        <div className="px-4 pt-4">
          <button
            onClick={handleCancelPreview}
            className="text-xs text-zinc-500 hover:text-zinc-300 mb-3"
          >
            &larr; Back to form
          </button>
          <h2 className="text-lg font-semibold text-zinc-100 mb-1">Preview Transaction</h2>
          <p className="text-xs text-zinc-500 mb-4">
            Review the simulation before sending
          </p>
        </div>

        <div className="px-4 flex-1 overflow-y-auto pb-4">
          {error && (
            <div className="mb-3 rounded-md bg-evap-red/10 border border-evap-red/30 px-3 py-2">
              <p className="text-[10px] text-evap-red">{error}</p>
            </div>
          )}

          <TxSimulation
            from={activeAddress}
            to={to}
            amount={parsedAmount}
            currentBalance={balance}
            currentEpoch={currentEpoch}
            epochDurationMs={epochDurationMs}
            onConfirm={handleConfirm}
            onCancel={handleCancelPreview}
          />

          {/* LAD-VM substructural-resource preview. Self-gates behind
              import.meta.env.DEV — the chain doesn't yet expose an
              is_lad_typed flag on objects, so until then this is a
              manual probe tool. See LadVmPreview.tsx file header. */}
          <LadVmPreview />

          {loading && (
            <div className="mt-3 flex items-center justify-center gap-2">
              <div className="w-4 h-4 border-2 border-evap-cyan border-t-transparent rounded-full animate-spin" />
              <span className="text-[11px] text-zinc-400">Sending transaction...</span>
            </div>
          )}
        </div>
      </div>
    );
  }

  // Form step
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
        <h2 className="text-lg font-semibold text-zinc-100 mb-1">Send EVAP</h2>
        <p className="text-xs text-zinc-500 mb-4">
          Available: {formatBalance(balance)} EVAP
        </p>
      </div>

      <form onSubmit={handlePreview} className="px-4 space-y-3 flex-1">
        <div>
          <label className="text-[10px] text-zinc-500 mb-1 block">Recipient Address</label>
          <input
            type="text"
            placeholder="0x..."
            value={to}
            onChange={e => setTo(e.target.value)}
            className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition font-mono"
            autoFocus
          />
        </div>

        <div>
          <label className="text-[10px] text-zinc-500 mb-1 block">Amount</label>
          <div className="relative">
            <input
              type="number"
              placeholder="0"
              min="1"
              max={balance}
              value={amount}
              onChange={e => setAmount(e.target.value)}
              className="w-full px-4 py-3 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-evap-cyan transition"
            />
            <button
              type="button"
              onClick={() => setAmount(String(balance))}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-[10px] text-evap-cyan hover:underline"
            >
              MAX
            </button>
          </div>
        </div>

        {error && <p className="text-xs text-evap-red">{error}</p>}

        <button
          type="submit"
          disabled={!isFormValid}
          className="w-full py-3 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-sm font-semibold text-black hover:opacity-90 transition disabled:opacity-50"
        >
          Preview Transaction
        </button>
      </form>
    </div>
  );
}

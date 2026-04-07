import { useState } from "react";
import { useWallet } from "@/hooks/useWallet";
import { formatBalance } from "@/utils/format";
import { Header } from "./Header";

export function SendScreen() {
  const { sendTransfer, balance, setView, loading, error } = useWallet();
  const [to, setTo] = useState("");
  const [amount, setAmount] = useState("");
  const [sent, setSent] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!to || !amount) return;

    const result = await sendTransfer(to, parseInt(amount, 10));
    if (result.success) {
      setSent(true);
      setTimeout(() => setView("home"), 2000);
    }
  };

  if (sent) {
    return (
      <div className="flex flex-col h-full">
        <Header />
        <div className="flex flex-col items-center justify-center flex-1 px-8">
          <div className="w-16 h-16 rounded-full bg-evap-green/20 flex items-center justify-center mb-4">
            <span className="text-3xl">✓</span>
          </div>
          <p className="text-sm font-semibold text-zinc-200">Transaction Sent</p>
          <p className="text-xs text-zinc-500 mt-1">{amount} EVAP sent</p>
        </div>
      </div>
    );
  }

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
        <h2 className="text-lg font-semibold text-zinc-100 mb-1">Send EVAP</h2>
        <p className="text-xs text-zinc-500 mb-4">
          Available: {formatBalance(balance)} EVAP
        </p>
      </div>

      <form onSubmit={handleSubmit} className="px-4 space-y-3 flex-1">
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
          disabled={loading || !to || !amount}
          className="w-full py-3 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-sm font-semibold text-black hover:opacity-90 transition disabled:opacity-50"
        >
          {loading ? "Sending..." : "Send"}
        </button>
      </form>
    </div>
  );
}

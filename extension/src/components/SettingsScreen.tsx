import { useState } from "react";
import { useWallet } from "@/hooks/useWallet";
import { Header } from "./Header";

export function SettingsScreen() {
  const { accounts, activeAccount, switchAccount, nodeUrl, setNodeUrl, setView, lock } = useWallet();
  const [customUrl, setCustomUrl] = useState(nodeUrl);

  return (
    <div className="flex flex-col h-full">
      <Header />
      <div className="px-4 pt-4 pb-2">
        <button
          onClick={() => setView("home")}
          className="text-xs text-zinc-500 hover:text-zinc-300 mb-3"
        >
          ← Back
        </button>
        <h2 className="text-lg font-semibold text-zinc-100">Settings</h2>
      </div>

      <div className="flex-1 overflow-y-auto px-4 space-y-4 pb-4">
        {/* Accounts */}
        <div>
          <h3 className="text-xs font-semibold text-zinc-400 mb-2">Accounts</h3>
          <div className="space-y-1">
            {accounts.map(acc => (
              <button
                key={acc.name}
                onClick={() => switchAccount(acc.name)}
                className={`w-full px-3 py-2 rounded-lg text-left transition ${
                  acc.name === activeAccount?.name
                    ? "bg-evap-cyan/10 border border-evap-cyan/30"
                    : "bg-evap-surface border border-evap-border hover:border-evap-cyan/20"
                }`}
              >
                <p className="text-xs font-medium text-zinc-200">{acc.name}</p>
                <p className="text-[10px] text-zinc-500 font-mono">{acc.address.slice(0, 16)}...</p>
              </button>
            ))}
          </div>
        </div>

        {/* Network */}
        <div>
          <h3 className="text-xs font-semibold text-zinc-400 mb-2">Network</h3>
          <div className="space-y-2">
            <input
              type="text"
              value={customUrl}
              onChange={e => setCustomUrl(e.target.value)}
              placeholder="https://testnet.evaporchain.com"
              className="w-full px-3 py-2 rounded-lg bg-evap-surface border border-evap-border text-xs text-zinc-200 font-mono focus:outline-none focus:border-evap-cyan transition"
            />
            <button
              onClick={() => setNodeUrl(customUrl)}
              className="w-full py-2 rounded-lg bg-evap-surface border border-evap-border text-xs text-zinc-300 hover:border-evap-cyan/40 transition"
            >
              Save Network
            </button>
          </div>
        </div>

        {/* About */}
        <div>
          <h3 className="text-xs font-semibold text-zinc-400 mb-2">About</h3>
          <div className="px-3 py-3 rounded-lg bg-evap-surface border border-evap-border space-y-1">
            <p className="text-xs text-zinc-300">EvaporChain Wallet v0.1.0</p>
            <p className="text-[10px] text-zinc-500">Post-quantum (ML-DSA) · Self-custodial</p>
            <p className="text-[10px] text-zinc-500">Keys never leave your browser</p>
          </div>
        </div>

        {/* Lock */}
        <button
          onClick={lock}
          className="w-full py-3 rounded-lg bg-evap-red/10 border border-evap-red/30 text-sm text-evap-red hover:bg-evap-red/20 transition"
        >
          Lock Wallet
        </button>
      </div>
    </div>
  );
}

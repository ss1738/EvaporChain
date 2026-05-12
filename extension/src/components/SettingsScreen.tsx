import { useEffect, useState } from "react";
import { ArrowLeft, ChevronRight } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import { Header } from "./Header";
import { formatBalance, shortAddress } from "@/utils/format";
import { MAINNET_URL, TESTNET_URL, type NetworkKind } from "@/utils/preferences";

function staleLabel(lastFetched: number): string {
  const seconds = Math.max(0, Math.floor((Date.now() - lastFetched) / 1000));
  if (seconds < 60) return `${seconds}s ago`;
  const mins = Math.floor(seconds / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  return `${hours}h ago`;
}

export function SettingsScreen() {
  const {
    accounts, activeAccount, switchAccount,
    accountBalances, refreshAllBalances,
    preferences, updatePreferences,
    setNetwork,
    setView, lock,
  } = useWallet();

  const [customUrl, setCustomUrl] = useState(preferences.customNodeUrl);

  // Refresh the multi-account batch view on mount so stale indicators
  // start counting from a fresh fetch when the user opens settings.
  useEffect(() => {
    refreshAllBalances();
  }, [refreshAllBalances]);

  const handleNetworkChange = (network: NetworkKind) => {
    setNetwork(network, customUrl);
  };

  const handleSaveCustomUrl = () => {
    setNetwork("custom", customUrl);
  };

  return (
    <div className="flex flex-col h-full">
      <Header />
      <div className="px-4 pt-4 pb-2">
        <button
          onClick={() => setView("home")}
          className="text-xs text-zinc-500 hover:text-zinc-300 mb-3"
        >
          <><ArrowLeft className="inline w-3.5 h-3.5 mr-1 -mt-0.5" strokeWidth={1.5} />Back</>
        </button>
        <h2 className="text-lg font-semibold text-zinc-100">Settings</h2>
      </div>

      <div className="flex-1 overflow-y-auto px-4 space-y-4 pb-4">
        {/* Accounts */}
        <div>
          <div className="flex items-center justify-between mb-2">
            <h3 className="text-xs font-semibold text-zinc-400">Accounts</h3>
            <button
              onClick={() => refreshAllBalances()}
              className="text-xs px-2 py-0.5 rounded text-evap-cyan border border-evap-cyan/30 hover:border-evap-cyan/60 transition"
              title="Refresh all balances"
            >
              ↻ All
            </button>
          </div>
          <div className="space-y-1">
            {accounts.map(acc => {
              const isActive = acc.name === activeAccount?.name;
              const entry = accountBalances[acc.address];
              return (
                <button
                  key={acc.name}
                  onClick={() => switchAccount(acc.name)}
                  className={`w-full px-3 py-2 rounded-lg text-left transition ${
                    isActive
                      ? "bg-evap-cyan/10 border border-evap-cyan/30"
                      : "bg-evap-surface border border-evap-border hover:border-evap-cyan/20"
                  }`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <div className="min-w-0">
                      <p className="text-xs font-medium text-zinc-200 truncate">{acc.name}</p>
                      <p className="text-xs text-zinc-500 font-mono truncate">
                        {shortAddress(acc.address)}
                      </p>
                    </div>
                    <div className="text-right shrink-0">
                      <p className="text-xs font-semibold text-zinc-200 tabular-nums">
                        {entry ? `${formatBalance(entry.balance)} EVAP` : "—"}
                      </p>
                      {entry && !isActive && (
                        <p className="text-[10px] text-zinc-500">
                          stale {staleLabel(entry.lastFetched)}
                        </p>
                      )}
                    </div>
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        {/* Network */}
        <div>
          <h3 className="text-xs font-semibold text-zinc-400 mb-2">Network</h3>
          <div className="space-y-2">
            <div className="grid grid-cols-3 gap-1">
              {(["mainnet", "testnet", "custom"] as NetworkKind[]).map((n) => {
                const selected = preferences.network === n;
                const palette =
                  n === "mainnet"
                    ? "border-evap-green/40 text-evap-green"
                    : n === "testnet"
                      ? "border-amber-500/40 text-amber-500"
                      : "border-zinc-500/40 text-zinc-300";
                return (
                  <button
                    key={n}
                    onClick={() => handleNetworkChange(n)}
                    className={`px-2 py-2 rounded-lg text-xs font-semibold capitalize transition border ${
                      selected
                        ? `bg-evap-surface ${palette}`
                        : "bg-evap-surface border-evap-border text-zinc-400 hover:border-evap-cyan/40"
                    }`}
                  >
                    {n}
                  </button>
                );
              })}
            </div>
            <p className="text-[10px] text-zinc-500 font-mono break-all">
              {preferences.network === "mainnet" && MAINNET_URL}
              {preferences.network === "testnet" && TESTNET_URL}
              {preferences.network === "custom" && (preferences.customNodeUrl || "(custom URL)")}
            </p>
            {preferences.network === "custom" && (
              <>
                <input
                  type="text"
                  value={customUrl}
                  onChange={e => setCustomUrl(e.target.value)}
                  placeholder="https://my-node.example.com"
                  className="w-full px-3 py-2 rounded-lg bg-evap-surface border border-evap-border text-xs text-zinc-200 font-mono focus:outline-none focus:border-evap-cyan transition"
                />
                <button
                  onClick={handleSaveCustomUrl}
                  className="w-full py-2 rounded-lg bg-evap-surface border border-evap-border text-xs text-zinc-300 hover:border-evap-cyan/40 transition"
                >
                  Save Custom URL
                </button>
              </>
            )}
          </div>
        </div>

        {/* Display preferences */}
        <div>
          <h3 className="text-xs font-semibold text-zinc-400 mb-2">Display</h3>
          <div className="space-y-2">
            <div className="flex items-center justify-between px-3 py-2 rounded-lg bg-evap-surface border border-evap-border">
              <span className="text-xs text-zinc-300">Currency</span>
              <select
                value={preferences.currency}
                onChange={e => updatePreferences({ currency: e.target.value as "USD" | "GBP" | "EUR" })}
                className="bg-transparent text-xs text-zinc-300 focus:outline-none"
              >
                <option value="USD">USD</option>
                <option value="GBP">GBP</option>
                <option value="EUR">EUR</option>
              </select>
            </div>
            <div className="flex items-center justify-between px-3 py-2 rounded-lg bg-evap-surface border border-evap-border">
              <span className="text-xs text-zinc-300">Hide small balances</span>
              <button
                onClick={() => updatePreferences({ hideSmallBalances: !preferences.hideSmallBalances })}
                className={`w-9 h-5 rounded-full transition-colors ${
                  preferences.hideSmallBalances ? "bg-evap-cyan" : "bg-zinc-700"
                }`}
              >
                <span className={`block w-4 h-4 mx-auto rounded-full bg-white transition-transform ${
                  preferences.hideSmallBalances ? "translate-x-2" : "-translate-x-2"
                }`} />
              </button>
            </div>
          </div>
        </div>

        {/* Security preferences */}
        <div>
          <h3 className="text-xs font-semibold text-zinc-400 mb-2">Security</h3>
          <div className="space-y-2">
            <div className="flex items-center justify-between px-3 py-2 rounded-lg bg-evap-surface border border-evap-border">
              <span className="text-xs text-zinc-300">Auto-lock (minutes)</span>
              <select
                value={preferences.autoLockMinutes}
                onChange={e => updatePreferences({ autoLockMinutes: Number(e.target.value) })}
                className="bg-transparent text-xs text-zinc-300 focus:outline-none"
              >
                <option value={5}>5</option>
                <option value={15}>15</option>
                <option value={30}>30</option>
                <option value={60}>60</option>
                <option value={0}>Never</option>
              </select>
            </div>
            <div className="flex items-center justify-between px-3 py-2 rounded-lg bg-evap-surface border border-evap-border">
              <span className="text-xs text-zinc-300">Notifications</span>
              <button
                onClick={() => updatePreferences({ notificationsEnabled: !preferences.notificationsEnabled })}
                className={`w-9 h-5 rounded-full transition-colors ${
                  preferences.notificationsEnabled ? "bg-evap-cyan" : "bg-zinc-700"
                }`}
              >
                <span className={`block w-4 h-4 mx-auto rounded-full bg-white transition-transform ${
                  preferences.notificationsEnabled ? "translate-x-2" : "-translate-x-2"
                }`} />
              </button>
            </div>
            <div className="flex items-center justify-between px-3 py-2 rounded-lg bg-evap-surface border border-evap-border">
              <span className="text-xs text-zinc-300">Default slippage</span>
              <select
                value={preferences.defaultSlippage}
                onChange={e => updatePreferences({ defaultSlippage: Number(e.target.value) })}
                className="bg-transparent text-xs text-zinc-300 focus:outline-none"
              >
                <option value={0.1}>0.1%</option>
                <option value={0.5}>0.5%</option>
                <option value={1.0}>1.0%</option>
                <option value={2.0}>2.0%</option>
              </select>
            </div>
            <div className="flex items-center justify-between px-3 py-2 rounded-lg bg-evap-surface border border-evap-border">
              <span className="text-xs text-zinc-300">Lock when popup loses focus</span>
              <button
                onClick={() => updatePreferences({ lockOnBlur: !preferences.lockOnBlur })}
                className={`w-9 h-5 rounded-full transition-colors ${
                  preferences.lockOnBlur ? "bg-evap-cyan" : "bg-zinc-700"
                }`}
              >
                <span className={`block w-4 h-4 mx-auto rounded-full bg-white transition-transform ${
                  preferences.lockOnBlur ? "translate-x-2" : "-translate-x-2"
                }`} />
              </button>
            </div>
            <div className="flex items-center justify-between px-3 py-2 rounded-lg bg-evap-surface border border-evap-border">
              <span className="text-xs text-zinc-300">Lock when popup closes</span>
              <button
                onClick={() => updatePreferences({ lockOnTabClose: !preferences.lockOnTabClose })}
                className={`w-9 h-5 rounded-full transition-colors ${
                  preferences.lockOnTabClose ? "bg-evap-cyan" : "bg-zinc-700"
                }`}
              >
                <span className={`block w-4 h-4 mx-auto rounded-full bg-white transition-transform ${
                  preferences.lockOnTabClose ? "translate-x-2" : "-translate-x-2"
                }`} />
              </button>
            </div>
          </div>
        </div>

        {/* About */}
        <div>
          <h3 className="text-xs font-semibold text-zinc-400 mb-2">About</h3>
          <div className="px-3 py-3 rounded-lg bg-evap-surface border border-evap-border space-y-1">
            <p className="text-xs text-zinc-300">EvaporChain Wallet v0.1.0</p>
            <p className="text-xs text-zinc-500">Post-quantum (ML-DSA) · Self-custodial</p>
            <p className="text-xs text-zinc-500">Keys never leave your browser</p>
          </div>
        </div>

        {/* Settings rows — consistent list-item pattern. */}
        <SettingsRow label="Backup & Restore" onClick={() => setView("backup")} />
        <SettingsRow label="Contacts"          onClick={() => setView("contacts")} />
        <SettingsRow label="Chain governance"  onClick={() => setView("governance")} />
        <SettingsRow label="Verify node honesty (DA sampling)" onClick={() => setView("da-verify")} />

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

function SettingsRow({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className="w-full flex items-center justify-between px-3 py-2.5 rounded-lg bg-evap-surface border border-evap-border text-sm text-zinc-300 hover:border-evap-cyan/40 hover:bg-zinc-800/40 transition text-left"
    >
      <span>{label}</span>
      <ChevronRight className="w-4 h-4 text-zinc-500" strokeWidth={1.5} />
    </button>
  );
}

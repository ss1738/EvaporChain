import { useState, useEffect } from "react";
import { useWalletConnect } from "@/hooks/useWalletConnect";
import { getStats } from "@/utils/api";
import type { MessageStats } from "@/utils/types";
import Inbox from "@/components/Inbox";
import ComposeMessage from "@/components/ComposeMessage";
import SentMessages from "@/components/SentMessages";
import GhostArchive from "@/components/GhostArchive";
import MessageView from "@/components/MessageView";

type Tab = "inbox" | "compose" | "sent" | "ghosts";

const TABS: { key: Tab; label: string; icon: string }[] = [
  { key: "inbox", label: "Inbox", icon: "\u2709" },
  { key: "compose", label: "Compose", icon: "\uD83D\uDCAC" },
  { key: "sent", label: "Sent", icon: "\u2197" },
  { key: "ghosts", label: "Ghosts", icon: "\uD83D\uDC7B" },
];

export default function App() {
  const { address, connected, loading, connect, disconnect } = useWalletConnect();
  const [activeTab, setActiveTab] = useState<Tab>("inbox");
  const [selectedMessage, setSelectedMessage] = useState<string | null>(null);
  const [stats, setStats] = useState<MessageStats | null>(null);

  useEffect(() => {
    if (!address) return;
    const fetchStats = async () => {
      try {
        const data = await getStats(address);
        setStats(data);
      } catch {
        /* ignore */
      }
    };
    fetchStats();
    const interval = setInterval(fetchStats, 10000);
    return () => clearInterval(interval);
  }, [address]);

  const handleSelectMessage = (id: string) => {
    setSelectedMessage(id);
  };

  const handleBack = () => {
    setSelectedMessage(null);
  };

  if (loading) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <div className="text-sm text-zinc-400">Connecting to EvaporChain...</div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-evap-bg">
      {/* Header */}
      <header className="border-b border-evap-border bg-evap-surface">
        <div className="mx-auto flex max-w-4xl items-center justify-between px-4 py-3">
          <div className="flex items-center gap-2">
            <span className="text-2xl">{"\u2709"}</span>
            <div>
              <h1 className="text-lg font-bold text-zinc-800">Mortal Messages</h1>
              <p className="text-[10px] text-zinc-400">Self-destructing messages on EvaporChain</p>
            </div>
          </div>

          {connected && address ? (
            <div className="flex items-center gap-3">
              <span className="rounded-lg bg-zinc-50 px-2.5 py-1 font-mono text-xs text-zinc-600">
                {address.slice(0, 6)}...{address.slice(-4)}
              </span>
              <button
                onClick={disconnect}
                className="rounded-lg border border-evap-border px-3 py-1 text-xs text-zinc-500 transition-colors hover:bg-zinc-50"
              >
                Disconnect
              </button>
            </div>
          ) : (
            <button
              onClick={connect}
              className="rounded-lg bg-evap-cyan px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-evap-cyan/90"
            >
              Connect Wallet
            </button>
          )}
        </div>
      </header>

      {!connected || !address ? (
        <main className="mx-auto max-w-4xl px-4 py-20 text-center">
          <div className="text-5xl">{"\u2709"}</div>
          <h2 className="mt-4 text-2xl font-bold text-zinc-800">Mortal Messages</h2>
          <p className="mt-2 text-zinc-500">
            Send self-destructing messages that decay over time on EvaporChain.
            <br />
            Messages lose energy, fade away, and eventually evaporate forever.
          </p>
          <button
            onClick={connect}
            className="mt-6 rounded-lg bg-evap-cyan px-6 py-3 text-sm font-semibold text-white transition-colors hover:bg-evap-cyan/90"
          >
            Connect EvaporChain Wallet
          </button>
        </main>
      ) : (
        <main className="mx-auto max-w-4xl px-4 py-6">
          {/* Dashboard stats */}
          {stats && (
            <div className="mb-6 grid grid-cols-2 gap-3 sm:grid-cols-5">
              <StatCard label="Sent" value={stats.sent} color="text-zinc-700" />
              <StatCard label="Received" value={stats.received} color="text-zinc-700" />
              <StatCard label="Alive" value={stats.alive} color="text-evap-cyan" />
              <StatCard label="Evaporated" value={stats.evaporated} color="text-zinc-400" />
              <StatCard
                label="Energy Spent"
                value={`${stats.total_energy_spent.toFixed(0)} EVP`}
                color="text-evap-amber"
              />
            </div>
          )}

          {/* Tab navigation */}
          <div className="mb-6 flex gap-1 rounded-xl border border-evap-border bg-evap-surface p-1">
            {TABS.map((tab) => (
              <button
                key={tab.key}
                onClick={() => {
                  setActiveTab(tab.key);
                  setSelectedMessage(null);
                }}
                className={`flex-1 rounded-lg px-3 py-2 text-sm font-medium transition-colors ${
                  activeTab === tab.key
                    ? "bg-evap-cyan text-white shadow-sm"
                    : "text-zinc-500 hover:text-zinc-700"
                }`}
              >
                <span className="mr-1.5">{tab.icon}</span>
                {tab.label}
              </button>
            ))}
          </div>

          {/* Content area */}
          {selectedMessage ? (
            <MessageView
              messageId={selectedMessage}
              currentAddress={address}
              onBack={handleBack}
            />
          ) : (
            <>
              {activeTab === "inbox" && (
                <Inbox address={address} onSelectMessage={handleSelectMessage} />
              )}
              {activeTab === "compose" && (
                <ComposeMessage
                  senderAddress={address}
                  onSent={() => setActiveTab("sent")}
                />
              )}
              {activeTab === "sent" && (
                <SentMessages address={address} onSelectMessage={handleSelectMessage} />
              )}
              {activeTab === "ghosts" && <GhostArchive address={address} />}
            </>
          )}
        </main>
      )}
    </div>
  );
}

function StatCard({
  label,
  value,
  color,
}: {
  label: string;
  value: string | number;
  color: string;
}) {
  return (
    <div className="rounded-xl border border-evap-border bg-evap-surface p-3 text-center">
      <div className={`text-lg font-bold ${color}`}>{value}</div>
      <div className="text-[10px] text-zinc-400">{label}</div>
    </div>
  );
}

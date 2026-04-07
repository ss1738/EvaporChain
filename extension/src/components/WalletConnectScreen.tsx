/**
 * WalletConnect session manager screen.
 * Lists connected dApps, allows pairing via URI, and manages sessions.
 */

import { useState, useEffect, useCallback } from "react";
import { useWallet } from "@/hooks/useWallet";
import { WalletConnectManager, type WcSessionProposal, type WcSession } from "@/utils/walletconnect";
import { WcApprovalModal } from "./WcApprovalModal";

// Singleton manager — shared across the extension lifetime
const wcManager = new WalletConnectManager();
const WC_PROJECT_ID = "evaporchain-wallet-extension"; // Replace with real WC Cloud project ID

export function WalletConnectScreen() {
  const { setView, activeAccount, wcSessions, setWcSessions, setNotification } = useWallet();

  const [uri, setUri] = useState("");
  const [pairing, setPairing] = useState(false);
  const [proposal, setProposal] = useState<WcSessionProposal | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Initialize WC manager on mount
  useEffect(() => {
    const initWc = async () => {
      try {
        await wcManager.init(WC_PROJECT_ID);
        // Sync persisted sessions into zustand
        setWcSessions(wcManager.getSessions());
      } catch {
        // Init may fail if localStorage is unavailable
      }
    };
    initWc();
  }, [setWcSessions]);

  // Wire up proposal handler
  useEffect(() => {
    wcManager.onProposal = (p: WcSessionProposal) => {
      setProposal(p);
      setPairing(false);
    };
    wcManager.onDisconnect = ({ topic }) => {
      setWcSessions(wcManager.getSessions());
      setNotification(`dApp disconnected (${topic.slice(0, 8)}...)`);
    };
    return () => {
      wcManager.onProposal = null;
      wcManager.onDisconnect = null;
    };
  }, [setWcSessions, setNotification]);

  // ── Handlers ──

  const handlePair = useCallback(async () => {
    const trimmed = uri.trim();
    if (!trimmed) return;
    setError(null);
    setPairing(true);
    try {
      await wcManager.pair(trimmed);
      setUri("");
    } catch (e: any) {
      setError(e.message);
      setPairing(false);
    }
  }, [uri]);

  const handleApprove = useCallback(async () => {
    if (!proposal || !activeAccount) return;
    try {
      await wcManager.approveSession(proposal, [activeAccount.address]);
      setWcSessions(wcManager.getSessions());
      setProposal(null);
      setNotification("dApp connected");
    } catch (e: any) {
      setError(e.message);
    }
  }, [proposal, activeAccount, setWcSessions, setNotification]);

  const handleReject = useCallback(async () => {
    if (!proposal) return;
    await wcManager.rejectSession(proposal);
    setProposal(null);
  }, [proposal]);

  const handleDisconnect = useCallback(async (topic: string) => {
    try {
      await wcManager.disconnectSession(topic);
      setWcSessions(wcManager.getSessions());
    } catch (e: any) {
      setError(e.message);
    }
  }, [setWcSessions]);

  const handleDisconnectAll = useCallback(async () => {
    await wcManager.disconnectAll();
    setWcSessions([]);
  }, [setWcSessions]);

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-evap-border">
        <button
          onClick={() => setView("home")}
          className="text-zinc-400 hover:text-zinc-200 transition text-sm"
        >
          &#8592; Back
        </button>
        <h1 className="text-sm font-semibold text-zinc-100">Connected dApps</h1>
      </div>

      {/* QR / URI input */}
      <div className="px-4 pt-4 pb-3">
        <p className="text-[10px] uppercase tracking-wider text-zinc-500 mb-2">
          Scan QR to connect
        </p>
        <div className="flex gap-2">
          <input
            type="text"
            value={uri}
            onChange={(e) => setUri(e.target.value)}
            placeholder="Paste WC URI (wc:... or wcv2:...)"
            className="flex-1 px-3 py-2 rounded-lg bg-zinc-800 border border-evap-border text-xs text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-evap-cyan/50"
          />
          <button
            onClick={handlePair}
            disabled={pairing || !uri.trim()}
            className="px-4 py-2 rounded-lg bg-evap-cyan text-xs font-medium text-black hover:bg-evap-cyan/90 transition disabled:opacity-40"
          >
            {pairing ? "..." : "Connect"}
          </button>
        </div>
        {error && (
          <p className="text-[10px] text-red-400 mt-1">{error}</p>
        )}
      </div>

      {/* Sessions list */}
      <div className="flex-1 overflow-y-auto px-4 pb-4">
        {wcSessions.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center py-12">
            <div className="w-12 h-12 rounded-full bg-evap-surface border border-evap-border flex items-center justify-center mb-3">
              <span className="text-xl text-zinc-600">&#9741;</span>
            </div>
            <p className="text-xs text-zinc-500 max-w-[200px]">
              No connected dApps — scan a QR code to connect
            </p>
          </div>
        ) : (
          <div className="space-y-2">
            {wcSessions.map((session) => (
              <SessionCard
                key={session.topic}
                session={session}
                onDisconnect={() => handleDisconnect(session.topic)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Disconnect All */}
      {wcSessions.length > 1 && (
        <div className="px-4 pb-4">
          <button
            onClick={handleDisconnectAll}
            className="w-full py-2.5 rounded-lg bg-red-900/20 border border-red-800/30 text-xs font-medium text-red-400 hover:bg-red-900/30 transition"
          >
            Disconnect All ({wcSessions.length})
          </button>
        </div>
      )}

      {/* Approval modal */}
      {proposal && (
        <WcApprovalModal
          proposal={proposal}
          onApprove={handleApprove}
          onReject={handleReject}
        />
      )}
    </div>
  );
}

// ── Session card ──

function SessionCard({ session, onDisconnect }: { session: WcSession; onDisconnect: () => void }) {
  const connectedDate = new Date(session.connectedAt).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });

  const chains = Object.values(session.namespaces).flatMap((ns) => ns.chains);

  return (
    <div className="flex items-center gap-3 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
      {/* Icon */}
      <div className="w-9 h-9 rounded-lg bg-evap-border flex items-center justify-center flex-shrink-0">
        {session.peer.icons.length > 0 ? (
          <img
            src={session.peer.icons[0]}
            alt={session.peer.name}
            className="w-7 h-7 rounded"
          />
        ) : (
          <span className="text-sm text-zinc-500">&#9741;</span>
        )}
      </div>

      {/* Info */}
      <div className="flex-1 min-w-0">
        <p className="text-xs font-medium text-zinc-200 truncate">
          {session.peer.name}
        </p>
        <p className="text-[10px] text-zinc-500 truncate">{session.peer.url}</p>
        <div className="flex items-center gap-2 mt-1">
          {chains.map((chain) => (
            <span
              key={chain}
              className="px-1.5 py-0.5 rounded bg-evap-cyan/10 text-[9px] text-evap-cyan"
            >
              {chain}
            </span>
          ))}
          <span className="text-[9px] text-zinc-600">
            Connected {connectedDate}
          </span>
        </div>
      </div>

      {/* Disconnect */}
      <button
        onClick={onDisconnect}
        className="px-2.5 py-1.5 rounded-lg bg-zinc-800 hover:bg-red-900/30 border border-evap-border hover:border-red-800/40 text-[10px] text-zinc-400 hover:text-red-400 transition flex-shrink-0"
      >
        Disconnect
      </button>
    </div>
  );
}

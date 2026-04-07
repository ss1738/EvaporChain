import { useState, useEffect, useCallback } from "react";
import { getMessage } from "@/utils/api";
import type { MortalMessage } from "@/utils/types";
import BoostModal from "./BoostModal";

interface Props {
  messageId: string;
  currentAddress: string;
  onBack: () => void;
}

export default function MessageView({ messageId, currentAddress, onBack }: Props) {
  const [message, setMessage] = useState<MortalMessage | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showBoost, setShowBoost] = useState(false);

  const fetchMessage = useCallback(async () => {
    try {
      const data = await getMessage(messageId);
      setMessage(data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load message");
    } finally {
      setLoading(false);
    }
  }, [messageId]);

  useEffect(() => {
    fetchMessage();
    const interval = setInterval(fetchMessage, 6000);
    return () => clearInterval(interval);
  }, [fetchMessage]);

  if (loading) {
    return <div className="py-12 text-center text-sm text-zinc-400">Loading message...</div>;
  }

  if (error || !message) {
    return (
      <div className="space-y-4">
        <button onClick={onBack} className="text-sm text-evap-cyan hover:underline">
          &larr; Back
        </button>
        <div className="rounded-lg border border-evap-red/20 bg-red-50 p-3 text-sm text-evap-red">
          {error || "Message not found"}
        </div>
      </div>
    );
  }

  const isGhost = message.status === "ghost";
  const isGrace = message.status === "grace";
  const pct = message.energy_percent;
  const canBoost = message.recipient === currentAddress && !isGhost;

  const statusColor = isGhost
    ? "text-zinc-400"
    : isGrace
      ? "text-evap-amber"
      : "text-evap-cyan";

  const barColor = isGhost
    ? "bg-zinc-300"
    : isGrace
      ? "bg-evap-amber"
      : "bg-evap-cyan";

  return (
    <div className="space-y-4">
      <button onClick={onBack} className="text-sm text-evap-cyan hover:underline">
        &larr; Back
      </button>

      <div className="rounded-2xl border border-evap-border bg-evap-surface p-6 shadow-sm">
        {/* Header */}
        <div className="flex items-start justify-between">
          <div>
            <div className="text-xs text-zinc-500">
              From: <span className="font-mono">{message.sender}</span>
            </div>
            <div className="mt-0.5 text-xs text-zinc-500">
              To: <span className="font-mono">{message.recipient}</span>
            </div>
          </div>
          <span
            className={`rounded-full px-2.5 py-0.5 text-xs font-semibold uppercase ${
              isGhost
                ? "bg-zinc-100 text-zinc-400"
                : isGrace
                  ? "bg-amber-50 text-evap-amber"
                  : "bg-cyan-50 text-evap-cyan"
            }`}
          >
            {message.status}
          </span>
        </div>

        {/* Message content */}
        <div className="mt-6">
          {isGhost ? (
            <p className="text-lg italic text-zinc-300">[evaporated]</p>
          ) : (
            <p
              className="whitespace-pre-wrap text-base leading-relaxed text-zinc-800"
              style={{ opacity: Math.max(0.15, pct / 100) }}
            >
              {message.content}
            </p>
          )}
        </div>

        {/* Energy bar */}
        <div className="mt-6">
          <div className="flex items-center justify-between text-xs text-zinc-500">
            <span className={statusColor}>
              Energy: {message.energy.toFixed(1)} / {message.max_energy.toFixed(1)} EVP
            </span>
            <span className="font-mono">{pct.toFixed(1)}%</span>
          </div>
          <div className="mt-1 h-3 overflow-hidden rounded-full bg-zinc-100">
            <div
              className={`h-full rounded-full transition-all duration-1000 ${barColor} ${
                isGrace ? "animate-pulse-low" : ""
              }`}
              style={{ width: `${pct}%` }}
            />
          </div>
        </div>

        {/* Metadata */}
        <div className="mt-4 grid grid-cols-2 gap-3 text-xs text-zinc-500 sm:grid-cols-4">
          <div className="rounded-lg bg-zinc-50 p-2">
            <div className="text-zinc-400">Created</div>
            <div className="mt-0.5 font-medium">{new Date(message.created_at).toLocaleString()}</div>
          </div>
          <div className="rounded-lg bg-zinc-50 p-2">
            <div className="text-zinc-400">Half-Life</div>
            <div className="mt-0.5 font-mono font-medium">{message.half_life} epochs</div>
          </div>
          <div className="rounded-lg bg-zinc-50 p-2">
            <div className="text-zinc-400">Energy</div>
            <div className="mt-0.5 font-mono font-medium">{message.energy.toFixed(2)} EVP</div>
          </div>
          <div className="rounded-lg bg-zinc-50 p-2">
            <div className="text-zinc-400">Status</div>
            <div className={`mt-0.5 font-medium ${statusColor}`}>{message.status}</div>
          </div>
        </div>

        {/* Boost button */}
        {canBoost && (
          <button
            onClick={() => setShowBoost(true)}
            className="mt-4 w-full rounded-lg border border-evap-cyan bg-evap-cyan/5 px-4 py-2 text-sm font-medium text-evap-cyan transition-colors hover:bg-evap-cyan/10"
          >
            Boost Energy to Extend Life
          </button>
        )}
      </div>

      {showBoost && (
        <BoostModal
          messageId={message.id}
          currentEnergy={message.energy}
          onClose={() => setShowBoost(false)}
          onBoosted={() => {
            setShowBoost(false);
            fetchMessage();
          }}
        />
      )}
    </div>
  );
}

import { useState, useEffect, useCallback } from "react";
import { getSentMessages } from "@/utils/api";
import type { MortalMessage } from "@/utils/types";
import MessageCard from "./MessageCard";

interface Props {
  address: string;
  onSelectMessage: (id: string) => void;
}

export default function SentMessages({ address, onSelectMessage }: Props) {
  const [messages, setMessages] = useState<MortalMessage[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchSent = useCallback(async () => {
    try {
      const data = await getSentMessages(address);
      setMessages(data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load sent messages");
    } finally {
      setLoading(false);
    }
  }, [address]);

  useEffect(() => {
    fetchSent();
    const interval = setInterval(fetchSent, 6000);
    return () => clearInterval(interval);
  }, [fetchSent]);

  const alive = messages.filter((m) => m.status !== "ghost").length;
  const evaporated = messages.filter((m) => m.status === "ghost").length;

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold text-zinc-800">
          Sent Messages
        </h2>
        <div className="flex gap-3 text-xs text-zinc-500">
          <span className="text-evap-cyan">{alive} alive</span>
          <span className="text-zinc-400">{evaporated} evaporated</span>
        </div>
      </div>

      {loading && (
        <div className="py-12 text-center text-sm text-zinc-400">Loading sent messages...</div>
      )}

      {error && (
        <div className="rounded-lg border border-evap-red/20 bg-red-50 p-3 text-sm text-evap-red">
          {error}
        </div>
      )}

      {!loading && !error && messages.length === 0 && (
        <div className="py-12 text-center text-sm text-zinc-400">
          You haven't sent any messages yet.
        </div>
      )}

      <div className="space-y-3">
        {messages.map((msg) => (
          <MessageCard
            key={msg.id}
            message={msg}
            perspective="sent"
            onClick={onSelectMessage}
          />
        ))}
      </div>
    </div>
  );
}

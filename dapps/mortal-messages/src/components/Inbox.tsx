import { useState, useEffect, useCallback } from "react";
import { getInbox } from "@/utils/api";
import type { MortalMessage } from "@/utils/types";
import MessageCard from "./MessageCard";

interface Props {
  address: string;
  onSelectMessage: (id: string) => void;
}

export default function Inbox({ address, onSelectMessage }: Props) {
  const [messages, setMessages] = useState<MortalMessage[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchInbox = useCallback(async () => {
    try {
      const data = await getInbox(address);
      setMessages(data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load inbox");
    } finally {
      setLoading(false);
    }
  }, [address]);

  useEffect(() => {
    fetchInbox();
    const interval = setInterval(fetchInbox, 6000);
    return () => clearInterval(interval);
  }, [fetchInbox]);

  const activeMessages = messages.filter((m) => m.status !== "ghost");
  const graceMessages = messages.filter((m) => m.status === "grace");

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold text-zinc-800">
          Inbox {"\u2709"}
        </h2>
        <div className="flex gap-3 text-xs text-zinc-500">
          <span className="text-evap-cyan">{activeMessages.length} active</span>
          {graceMessages.length > 0 && (
            <span className="text-evap-amber">{graceMessages.length} fading</span>
          )}
        </div>
      </div>

      {loading && (
        <div className="py-12 text-center text-sm text-zinc-400">Loading messages...</div>
      )}

      {error && (
        <div className="rounded-lg border border-evap-red/20 bg-red-50 p-3 text-sm text-evap-red">
          {error}
        </div>
      )}

      {!loading && !error && messages.length === 0 && (
        <div className="py-12 text-center text-sm text-zinc-400">
          No messages yet. Your inbox is empty.
        </div>
      )}

      <div className="space-y-3">
        {messages.map((msg) => (
          <MessageCard
            key={msg.id}
            message={msg}
            perspective="inbox"
            onClick={onSelectMessage}
          />
        ))}
      </div>
    </div>
  );
}

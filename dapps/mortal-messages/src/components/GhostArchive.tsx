import { useState, useEffect, useCallback } from "react";
import { getInbox, getSentMessages } from "@/utils/api";
import type { MortalMessage } from "@/utils/types";

interface Props {
  address: string;
}

export default function GhostArchive({ address }: Props) {
  const [ghosts, setGhosts] = useState<MortalMessage[]>([]);
  const [loading, setLoading] = useState(true);

  const fetchGhosts = useCallback(async () => {
    try {
      const [inbox, sent] = await Promise.all([
        getInbox(address),
        getSentMessages(address),
      ]);
      const allGhosts = [...inbox, ...sent]
        .filter((m) => m.status === "ghost")
        .sort((a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime());

      // Deduplicate by id
      const seen = new Set<string>();
      const unique = allGhosts.filter((m) => {
        if (seen.has(m.id)) return false;
        seen.add(m.id);
        return true;
      });

      setGhosts(unique);
    } catch {
      /* silently handle */
    } finally {
      setLoading(false);
    }
  }, [address]);

  useEffect(() => {
    fetchGhosts();
  }, [fetchGhosts]);

  return (
    <div className="space-y-4">
      <h2 className="text-xl font-semibold text-zinc-800">
        Ghost Archive {"\uD83D\uDC7B"}
      </h2>
      <p className="text-sm text-zinc-500">
        Messages that have fully evaporated. Their content is gone forever.
      </p>

      {loading && (
        <div className="py-12 text-center text-sm text-zinc-400">Loading ghost archive...</div>
      )}

      {!loading && ghosts.length === 0 && (
        <div className="py-12 text-center text-sm text-zinc-400">
          No evaporated messages yet. All your messages are still alive!
        </div>
      )}

      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {ghosts.map((ghost) => {
          const isSender = ghost.sender === address;
          return (
            <div
              key={ghost.id}
              className="rounded-xl border border-zinc-200 bg-evap-surface p-4 opacity-60"
            >
              <div className="flex items-center gap-2">
                <span className="text-lg text-zinc-300">{"\uD83D\uDC7B"}</span>
                <span className="rounded-full bg-zinc-100 px-2 py-0.5 text-[10px] font-semibold uppercase text-zinc-400">
                  Ghost
                </span>
              </div>

              <p className="mt-3 text-sm italic text-zinc-300">[evaporated]</p>

              <div className="mt-3 space-y-1 text-[10px] text-zinc-400">
                <div>
                  {isSender ? "Sent to" : "Received from"}:{" "}
                  <span className="font-mono">
                    {shortAddr(isSender ? ghost.recipient : ghost.sender)}
                  </span>
                </div>
                <div>Created: {new Date(ghost.created_at).toLocaleDateString()}</div>
                <div>
                  Original energy: {ghost.max_energy.toFixed(1)} EVP | Half-life:{" "}
                  {ghost.half_life} epochs
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function shortAddr(addr: string): string {
  if (addr.length <= 12) return addr;
  return addr.slice(0, 6) + "..." + addr.slice(-4);
}

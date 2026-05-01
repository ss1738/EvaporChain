"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { Search } from "lucide-react";
import { normaliseHex } from "@/lib/api";

// Sniffs the input and routes to /block, /tx, or /address accordingly.
// Heuristics:
//   - Pure digits             → /block/<n>
//   - 64-hex (32 bytes)       → /tx/<hash>   (default for full-length hex)
//   - 40-hex (20 bytes)       → /address/<addr> (EVM-style — pads to 32 left)
//   - "0x"-prefixed hex       → strip prefix and re-evaluate
//   - Anything else           → /address/<input>  (chain accepts long-form ids)
export function SearchBar() {
  const [q, setQ] = useState("");
  const router = useRouter();

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const raw = q.trim();
    if (!raw) return;
    if (/^\d+$/.test(raw)) {
      router.push(`/block/${raw}`);
      return;
    }
    const h = normaliseHex(raw);
    if (/^[0-9a-f]{64}$/.test(h)) {
      // Disambiguation: tx hashes and 32-byte object/address ids both have
      // 64 hex chars. We default to the tx page; users can navigate from
      // there to address/object views via the linked detail panes.
      router.push(`/tx/${h}`);
      return;
    }
    if (/^[0-9a-f]{40}$/.test(h)) {
      // EVM-style 20-byte address — node accepts it and pads internally.
      router.push(`/address/${h}`);
      return;
    }
    router.push(`/address/${encodeURIComponent(raw)}`);
  };

  return (
    <form onSubmit={submit} className="relative">
      <Search className="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-zinc-400" />
      <input
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder="Block height · 0xtxhash · address"
        className="w-full rounded-lg border border-evap-border bg-white py-2 pl-9 pr-3 text-sm placeholder-zinc-400 focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
      />
    </form>
  );
}

import type { MortalMessage } from "@/utils/types";

interface Props {
  message: MortalMessage;
  perspective: "inbox" | "sent";
  onClick: (id: string) => void;
}

export default function MessageCard({ message, perspective, onClick }: Props) {
  const isGhost = message.status === "ghost";
  const isGrace = message.status === "grace";
  const energyPct = message.energy_percent;

  const statusColor = isGhost
    ? "text-zinc-400"
    : isGrace
      ? "text-evap-amber"
      : "text-evap-cyan";

  const statusIcon = isGhost ? "\uD83D\uDC7B" : isGrace ? "\u26A0" : "\u2709";

  const borderColor = isGhost
    ? "border-zinc-200"
    : isGrace
      ? "border-evap-amber/30"
      : "border-evap-cyan/20";

  const preview = isGhost
    ? "[evaporated]"
    : message.content.length > 80
      ? message.content.slice(0, 80) + "..."
      : message.content;

  const counterparty =
    perspective === "inbox"
      ? `From: ${shortAddr(message.sender)}`
      : `To: ${shortAddr(message.recipient)}`;

  return (
    <button
      onClick={() => onClick(message.id)}
      className={`w-full rounded-xl border ${borderColor} bg-evap-surface p-4 text-left transition-shadow hover:shadow-md ${
        isGrace ? "animate-pulse-low" : ""
      }`}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className={`text-lg ${statusColor}`}>{statusIcon}</span>
          <div>
            <div className="text-xs font-medium text-zinc-500">{counterparty}</div>
            <p
              className="mt-0.5 text-sm text-zinc-800"
              style={{ opacity: isGhost ? 0.3 : Math.max(0.2, energyPct / 100) }}
            >
              {preview}
            </p>
          </div>
        </div>
        <div className="shrink-0 text-right">
          <span
            className={`inline-block rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${
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
      </div>

      {/* Energy bar */}
      <div className="mt-3 flex items-center gap-2">
        <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-zinc-100">
          <div
            className={`h-full rounded-full transition-all ${
              isGhost ? "bg-zinc-300" : isGrace ? "bg-evap-amber" : "bg-evap-cyan"
            }`}
            style={{ width: `${energyPct}%` }}
          />
        </div>
        <span className="text-[10px] font-mono text-zinc-400">
          {energyPct.toFixed(0)}%
        </span>
      </div>

      <div className="mt-2 flex items-center justify-between text-[10px] text-zinc-400">
        <span>{new Date(message.created_at).toLocaleDateString()}</span>
        <span>{message.energy.toFixed(1)} / {message.max_energy.toFixed(1)} EVP</span>
      </div>
    </button>
  );
}

function shortAddr(addr: string): string {
  if (addr.length <= 12) return addr;
  return addr.slice(0, 6) + "..." + addr.slice(-4);
}

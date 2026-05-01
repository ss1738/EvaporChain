import type { MortalMessage } from "@/utils/types";

interface Props {
  message: MortalMessage;
  currentEpoch?: number;
}

// Emerald → red bar showing current_energy / max_energy ratio with an
// "evaporates in ~N epochs" estimate. Pure client-side projection from
// half-life: epochs_remaining ≈ half_life · log2(current/threshold).
// Mirrors the substrate's getDecayForecast curve without a network round-trip.
export default function DecayForecastBar({ message, currentEpoch }: Props) {
  const ratio = message.max_energy > 0 ? message.energy / message.max_energy : 0;
  const pct = Math.max(0, Math.min(100, ratio * 100));

  // Decay threshold for ghosting is ~1% of max in the chain's current
  // genesis params. log2(current/threshold) gives the expected number of
  // half-lives; multiplying by half_life converts to epochs.
  const threshold = Math.max(message.max_energy * 0.01, 0.001);
  const epochsRemaining =
    message.energy <= threshold
      ? 0
      : Math.max(0, Math.round(message.half_life * Math.log2(message.energy / threshold)));

  // Tone: emerald above 60%, amber 25-60%, red below.
  const tone =
    pct >= 60
      ? { bar: "bg-evap-green", chip: "text-evap-green" }
      : pct >= 25
        ? { bar: "bg-evap-amber", chip: "text-evap-amber" }
        : { bar: "bg-evap-red", chip: "text-evap-red" };

  return (
    <div className="rounded-lg border border-evap-border bg-evap-surface p-3">
      <div className="flex items-center justify-between text-[11px] text-zinc-500">
        <span className={tone.chip}>
          {pct.toFixed(1)}% energy
        </span>
        <span className="font-mono">
          {message.energy.toFixed(1)} / {message.max_energy.toFixed(1)} EVP
        </span>
      </div>
      <div className="mt-1 h-2 overflow-hidden rounded-full bg-zinc-100">
        <div
          className={`h-full rounded-full transition-all duration-700 ${tone.bar}`}
          style={{ width: `${pct}%` }}
        />
      </div>
      <p className="mt-1 text-[10px] text-zinc-400">
        {message.status === "ghost"
          ? "Already evaporated"
          : epochsRemaining === 0
            ? "Evaporates this epoch"
            : `Evaporates in ~${epochsRemaining} epochs · half-life ${message.half_life}${
                currentEpoch ? ` · now epoch ${currentEpoch}` : ""
              }`}
      </p>
    </div>
  );
}

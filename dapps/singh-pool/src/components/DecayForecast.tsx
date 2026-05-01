import type { SinghPoolPosition } from "@/lib/api";

interface Props {
  position: SinghPoolPosition;
  currentEpoch: number;
}

// Pure-compute decay forecast: how many epochs until the position
// auto-evicts? The substrate ghosts an object when current_energy <= 1% of
// max_energy. Half-life decay gives epochs ≈ half_life · log2(current/threshold).
// If a patronage covenant is active, immunity overrides the forecast.
export function DecayForecast({ position, currentEpoch }: Props) {
  const threshold = Math.max(position.maxEnergy * 0.01, 1);
  const epochs =
    position.energy <= threshold
      ? 0
      : Math.max(
          0,
          Math.round(position.halfLife * Math.log2(position.energy / threshold)),
        );

  const immune = position.patronageCovenantEpochs > 0;
  const immuneEpoch = currentEpoch + position.patronageCovenantEpochs;

  return (
    <div className="rounded-lg border border-evap-border bg-white p-3">
      <div className="flex items-center justify-between text-[11px] text-zinc-500">
        <span className="font-medium">Decay Forecast</span>
        <span className="font-mono">epoch {currentEpoch}</span>
      </div>
      {position.state === "Ghost" ? (
        <p className="mt-1 text-xs text-zinc-400">Already evaporated</p>
      ) : immune ? (
        <p className="mt-1 text-xs text-evap-purple">
          Patronage immunity active until epoch {immuneEpoch}
        </p>
      ) : epochs === 0 ? (
        <p className="mt-1 text-xs text-evap-red">Evaporates this epoch</p>
      ) : (
        <p className="mt-1 text-xs text-zinc-700">
          Evaporates in ~{epochs} epochs · half-life {position.halfLife}
        </p>
      )}
    </div>
  );
}

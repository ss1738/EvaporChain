import Link from "next/link";
import type { SinghPoolPosition } from "@/lib/api";
import { EnergyBar } from "./EnergyBar";

interface Props {
  position: SinghPoolPosition;
}

// Listing card for an LP position. Shows the price range, current energy
// reserve, state, and links into the position-detail page where the LP can
// refresh or open a patronage covenant.
export function PositionCard({ position }: Props) {
  const stateTone =
    position.state === "Active"
      ? "border-evap-green/30 bg-emerald-50 text-evap-green"
      : position.state === "Grace"
        ? "border-evap-amber/30 bg-amber-50 text-evap-amber"
        : "border-zinc-200 bg-zinc-100 text-zinc-500";

  return (
    <Link
      href={`/positions/${encodeURIComponent(position.id)}`}
      className="block rounded-xl border border-evap-border bg-white p-4 transition hover:shadow-md"
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-xs font-semibold text-zinc-900">
            {position.tokenA} / {position.tokenB}
          </p>
          <p className="text-[10px] font-mono text-zinc-400">#{position.id.slice(0, 10)}</p>
        </div>
        <span className={`rounded-full border px-2 py-0.5 text-[10px] font-medium ${stateTone}`}>
          {position.state}
        </span>
      </div>

      <div className="mt-3 grid grid-cols-2 gap-2 text-[10px] text-zinc-500">
        <div className="rounded-md bg-zinc-50 px-2 py-1">
          <p className="font-semibold text-zinc-700">{position.lowerTick}</p>
          <p>Lower tick</p>
        </div>
        <div className="rounded-md bg-zinc-50 px-2 py-1">
          <p className="font-semibold text-zinc-700">{position.upperTick}</p>
          <p>Upper tick</p>
        </div>
      </div>

      <div className="mt-3">
        <EnergyBar
          energy={position.energy}
          maxEnergy={position.maxEnergy}
          state={position.state}
        />
      </div>

      <div className="mt-3 flex items-center justify-between text-[10px] text-zinc-500">
        <span>
          Liquidity: <span className="font-mono">{position.liquidity.toLocaleString()}</span>
        </span>
        <span>
          Fees: <span className="font-mono text-evap-cyan">{position.feesEarned.toLocaleString()}</span>
        </span>
      </div>

      {position.patronageCovenantEpochs > 0 && (
        <div className="mt-2 inline-flex items-center gap-1 rounded-full border border-evap-purple/30 bg-purple-50 px-2 py-0.5 text-[10px] font-medium text-evap-purple">
          ✦ Patronage immunity ({position.patronageCovenantEpochs} epochs)
        </div>
      )}
    </Link>
  );
}

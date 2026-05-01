import { useEffect, useRef } from "react";
import type { Nft } from "@/utils/types";
import { generateNftArt } from "@/utils/art";
import { DemurrageChip } from "./DemurrageChip";
import { HlwaSupplyChip } from "./HlwaSupplyChip";

interface NftCardProps {
  nft: Nft;
  onRefresh?: (nft: Nft) => void;
  onTransfer?: (nft: Nft) => void;
  onSponsor?: (nft: Nft) => void;
  isOwner?: boolean;
  currentEpoch?: number;
}

export function NftCard({ nft, onRefresh, onTransfer, onSponsor, isOwner, currentEpoch }: NftCardProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const energyPercent = nft.max_energy > 0
    ? Math.round((nft.current_energy / nft.max_energy) * 100)
    : 0;

  useEffect(() => {
    if (canvasRef.current) {
      generateNftArt(canvasRef.current, nft.id, energyPercent, nft.state);
    }
  }, [nft.id, energyPercent, nft.state]);

  const barColor = energyPercent > 50
    ? "bg-evap-green"
    : energyPercent > 20
    ? "bg-evap-amber"
    : energyPercent > 0
    ? "bg-evap-red"
    : "bg-evap-ghost";

  const stateColors: Record<string, string> = {
    Active: "bg-evap-green/10 text-evap-green border-evap-green/20",
    Grace: "bg-evap-amber/10 text-evap-amber border-evap-amber/20",
    Ghost: "bg-zinc-100 text-zinc-500 border-zinc-200",
  };

  return (
    <div className="bg-evap-surface rounded-xl border border-evap-border overflow-hidden shadow-sm hover:shadow-md transition group">
      {/* Art */}
      <div className="relative">
        <canvas
          ref={canvasRef}
          width={300}
          height={200}
          className="w-full h-48 object-cover"
        />
        <div className="absolute top-2 right-2 flex flex-col items-end gap-1">
          <span className={`text-[10px] px-2 py-0.5 rounded-full border ${stateColors[nft.state] ?? ""}`}>
            {nft.state}
          </span>
          {currentEpoch !== undefined && (
            <HlwaSupplyChip nft={nft} currentEpoch={currentEpoch} />
          )}
        </div>
        {nft.state !== "Ghost" && (
          <div className="absolute bottom-0 left-0 right-0 h-1">
            <div
              className={`h-full ${barColor} transition-all duration-1000 ${energyPercent <= 5 ? "animate-pulse" : ""}`}
              style={{ width: `${Math.max(energyPercent, 1)}%` }}
            />
          </div>
        )}
      </div>

      {/* Info */}
      <div className="p-4">
        <div className="flex items-start justify-between mb-2">
          <div>
            <h3 className="text-sm font-semibold text-zinc-900">{nft.name}</h3>
            <p className="text-[11px] text-zinc-500">{nft.collection}</p>
          </div>
          <span className="text-[10px] text-zinc-400 font-mono">#{nft.id}</span>
        </div>

        {/* Energy stats */}
        <div className="grid grid-cols-3 gap-2 mb-3">
          <Stat label="Energy" value={`${energyPercent}%`} color={barColor.replace("bg-", "text-")} />
          <Stat label="Half-life" value={`${nft.half_life}`} />
          <Stat label="Remaining" value={nft.state === "Ghost" ? "—" : `${nft.epochs_remaining} ep`} />
        </div>

        {/* Energy bar */}
        <div className="w-full h-2 bg-zinc-100 rounded-full overflow-hidden mb-3">
          <div
            className={`h-full rounded-full ${barColor} transition-all duration-1000`}
            style={{ width: `${Math.max(energyPercent, 1)}%` }}
          />
        </div>

        {/* Owner */}
        <div className="mb-3 flex items-center justify-between">
          <p className="text-[10px] text-zinc-400 font-mono truncate">
            Owner: {nft.owner.slice(0, 10)}...{nft.owner.slice(-6)}
          </p>
          {currentEpoch !== undefined && nft.state !== "Ghost" && (
            <DemurrageChip
              ownerAddress={nft.owner}
              lastTouchedEpoch={nft.last_refreshed > 0 ? nft.last_refreshed : nft.minted_epoch}
              currentEpoch={currentEpoch}
            />
          )}
        </div>

        {/* Actions */}
        {isOwner && nft.state !== "Ghost" && (
          <div className="space-y-2">
            <div className="flex gap-2">
              {onRefresh && (
                <button
                  onClick={() => onRefresh(nft)}
                  className="flex-1 py-2 rounded-lg bg-evap-cyan/10 text-evap-cyan text-xs font-medium hover:bg-evap-cyan/20 transition border border-evap-cyan/20"
                >
                  ⟳ Refresh
                </button>
              )}
              {onTransfer && (
                <button
                  onClick={() => onTransfer(nft)}
                  className="flex-1 py-2 rounded-lg bg-evap-purple/10 text-evap-purple text-xs font-medium hover:bg-evap-purple/20 transition border border-evap-purple/20"
                >
                  ↗ Transfer
                </button>
              )}
            </div>
            {onSponsor && (
              <button
                onClick={() => onSponsor(nft)}
                className="w-full py-2 rounded-lg bg-gradient-to-r from-evap-cyan/10 to-evap-purple/10 text-evap-purple text-xs font-medium hover:from-evap-cyan/20 hover:to-evap-purple/20 transition border border-evap-purple/20"
              >
                ✦ Sponsor (Patronage)
              </button>
            )}
          </div>
        )}
        {!isOwner && nft.state !== "Ghost" && onSponsor && (
          <button
            onClick={() => onSponsor(nft)}
            className="w-full py-2 rounded-lg bg-gradient-to-r from-evap-cyan/10 to-evap-purple/10 text-evap-purple text-xs font-medium hover:from-evap-cyan/20 hover:to-evap-purple/20 transition border border-evap-purple/20"
          >
            ✦ Sponsor this NFT
          </button>
        )}
        {nft.state === "Ghost" && onRefresh && (
          <button
            onClick={() => onRefresh(nft)}
            className="w-full py-2 rounded-lg bg-zinc-100 text-zinc-600 text-xs font-medium hover:bg-zinc-200 transition border border-zinc-200"
          >
            Resurrect
          </button>
        )}
      </div>
    </div>
  );
}

function Stat({ label, value, color }: { label: string; value: string; color?: string }) {
  return (
    <div className="text-center">
      <p className={`text-xs font-semibold ${color ?? "text-zinc-700"}`}>{value}</p>
      <p className="text-[10px] text-zinc-400">{label}</p>
    </div>
  );
}

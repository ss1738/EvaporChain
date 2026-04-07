import { useEffect, useState, useMemo } from "react";
import { useWallet } from "@/hooks/useWallet";
import { EnergyBar } from "./EnergyBar";
import { energyPercent } from "@/utils/format";
import { Header } from "./Header";
import { QuickRefresh } from "./QuickRefresh";
import type { NftItem } from "@/utils/api";

type SortMode = "urgent" | "newest" | "collection";
type FilterState = "All" | "Active" | "Grace" | "Ghost";

export function NftGallery() {
  const { nfts, refreshNfts, selectNft, setView } = useWallet();
  const [sort, setSort] = useState<SortMode>("urgent");
  const [filter, setFilter] = useState<FilterState>("All");

  useEffect(() => {
    refreshNfts();
    const interval = setInterval(refreshNfts, 10_000);
    return () => clearInterval(interval);
  }, [refreshNfts]);

  const filtered = useMemo(() => {
    let list = filter === "All" ? nfts : nfts.filter(n => n.state === filter);

    switch (sort) {
      case "urgent":
        list = [...list].sort((a, b) => a.current_energy - b.current_energy);
        break;
      case "newest":
        list = [...list].sort((a, b) => b.created_epoch - a.created_epoch);
        break;
      case "collection":
        list = [...list].sort((a, b) => a.collection.localeCompare(b.collection));
        break;
    }

    return list;
  }, [nfts, sort, filter]);

  return (
    <div className="flex flex-col h-full relative">
      <Header />
      <QuickRefresh />

      <div className="px-4 pt-4 pb-2 flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold text-zinc-100">NFT Gallery</h2>
          <p className="text-xs text-zinc-500">{nfts.length} NFTs owned</p>
        </div>
        <button
          onClick={() => setView("home")}
          className="text-xs text-zinc-500 hover:text-zinc-300"
        >
          &larr; Back
        </button>
      </div>

      {/* Sort pills */}
      <div className="px-4 pb-2 flex gap-1.5 flex-wrap">
        <SortPill label="Urgent" active={sort === "urgent"} onClick={() => setSort("urgent")} />
        <SortPill label="Newest" active={sort === "newest"} onClick={() => setSort("newest")} />
        <SortPill label="Collection" active={sort === "collection"} onClick={() => setSort("collection")} />
      </div>

      {/* Filter pills */}
      <div className="px-4 pb-3 flex gap-1.5">
        {(["All", "Active", "Grace", "Ghost"] as FilterState[]).map(f => (
          <FilterPill key={f} label={f} active={filter === f} onClick={() => setFilter(f)} />
        ))}
      </div>

      {/* NFT grid */}
      <div className="flex-1 overflow-y-auto px-4 pb-4">
        {filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-12">
            <span className="text-3xl mb-3">🖼</span>
            <p className="text-sm text-zinc-500">No NFTs yet</p>
            <p className="text-xs text-zinc-600 mt-1">Mint your first on the marketplace</p>
          </div>
        ) : (
          <div className="grid grid-cols-2 gap-2">
            {filtered.map(nft => (
              <NftCard key={nft.id} nft={nft} onTap={() => selectNft(nft)} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function NftCard({ nft, onTap }: { nft: NftItem; onTap: () => void }) {
  const isGhost = nft.state === "Ghost";
  const isGrace = nft.state === "Grace";
  const percent = energyPercent(nft.current_energy, nft.max_energy);

  const stateIcon = isGhost ? "\u{1F480}" : isGrace ? "\u{231B}" : "";
  const stateBadgeClass = isGhost
    ? "bg-evap-ghost/10 text-evap-ghost"
    : isGrace
    ? "bg-evap-amber/10 text-evap-amber"
    : "bg-evap-green/10 text-evap-green";

  const cardBorderClass = isGhost
    ? "border-zinc-700 opacity-60 grayscale"
    : isGrace
    ? "border-amber-500/40"
    : "border-evap-border";

  return (
    <button
      onClick={onTap}
      className={`text-left px-2.5 py-2.5 rounded-lg bg-evap-surface border transition hover:border-evap-cyan/40 ${cardBorderClass}`}
    >
      {/* Image placeholder */}
      <div className={`w-full aspect-square rounded-md bg-zinc-800 mb-2 flex items-center justify-center ${isGhost ? "grayscale" : ""}`}>
        {nft.image_url ? (
          <img
            src={nft.image_url}
            alt={nft.name}
            className="w-full h-full object-cover rounded-md"
          />
        ) : (
          <span className="text-2xl text-zinc-600">🖼</span>
        )}
      </div>

      {/* Name + collection */}
      <p className="text-[11px] font-semibold text-zinc-200 truncate">{nft.name}</p>
      <p className="text-[9px] text-zinc-500 truncate mb-1.5">{nft.collection}</p>

      {/* State badge */}
      <div className="flex items-center justify-between mb-1.5">
        <span className={`text-[9px] px-1.5 py-0.5 rounded-full ${stateBadgeClass}`}>
          {stateIcon} {nft.state}
        </span>
        <span className="text-[9px] text-zinc-500">{percent}%</span>
      </div>

      {/* Mini energy bar */}
      <EnergyBar current={nft.current_energy} max={nft.max_energy} showLabel={false} size="sm" />

      {/* Countdown */}
      {nft.epochs_remaining > 0 && (
        <p className="text-[9px] text-zinc-500 mt-1 text-center">
          ~{nft.epochs_remaining} epochs left
        </p>
      )}
    </button>
  );
}

function SortPill({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className={`text-[10px] px-2.5 py-1 rounded-full border transition ${
        active
          ? "bg-evap-cyan/10 border-evap-cyan/40 text-evap-cyan"
          : "border-evap-border text-zinc-500 hover:text-zinc-300"
      }`}
    >
      {label}
    </button>
  );
}

function FilterPill({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  const colorMap: Record<string, string> = {
    All: active ? "bg-evap-cyan/10 border-evap-cyan/40 text-evap-cyan" : "",
    Active: active ? "bg-evap-green/10 border-evap-green/40 text-evap-green" : "",
    Grace: active ? "bg-evap-amber/10 border-evap-amber/40 text-evap-amber" : "",
    Ghost: active ? "bg-evap-ghost/10 border-evap-ghost/40 text-evap-ghost" : "",
  };

  return (
    <button
      onClick={onClick}
      className={`text-[10px] px-2.5 py-1 rounded-full border transition ${
        active
          ? colorMap[label]
          : "border-evap-border text-zinc-500 hover:text-zinc-300"
      }`}
    >
      {label}
    </button>
  );
}

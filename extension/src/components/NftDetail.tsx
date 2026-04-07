import { useState } from "react";
import { useWallet } from "@/hooks/useWallet";
import { EnergyBar } from "./EnergyBar";
import { energyPercent, shortAddress, energyStatus } from "@/utils/format";
import { Header } from "./Header";
import { NftRefreshModal } from "./NftRefreshModal";
import { api } from "@/utils/api";

export function NftDetail() {
  const { selectedNft, setView, refreshNfts, setNotification, setError, activeAccount, loading } = useWallet();
  const [showRefresh, setShowRefresh] = useState(false);
  const [showTransfer, setShowTransfer] = useState(false);
  const [transferTo, setTransferTo] = useState("");
  const [transferring, setTransferring] = useState(false);

  if (!selectedNft) {
    return (
      <div className="flex flex-col h-full items-center justify-center">
        <p className="text-sm text-zinc-500">No NFT selected</p>
        <button
          onClick={() => setView("nfts")}
          className="mt-2 text-xs text-evap-cyan hover:underline"
        >
          Back to gallery
        </button>
      </div>
    );
  }

  const nft = selectedNft;
  const percent = energyPercent(nft.current_energy, nft.max_energy);
  const isOwner = activeAccount?.address === nft.owner;

  const isGhost = nft.state === "Ghost";
  const isGrace = nft.state === "Grace";

  const stateIcon = isGhost ? "\u{1F480}" : isGrace ? "\u{231B}" : "\u{2705}";
  const stateBadgeClass = isGhost
    ? "bg-evap-ghost/10 text-evap-ghost border-evap-ghost/30"
    : isGrace
    ? "bg-evap-amber/10 text-evap-amber border-evap-amber/30"
    : "bg-evap-green/10 text-evap-green border-evap-green/30";

  const handleTransfer = async () => {
    if (!transferTo.trim()) return;
    setTransferring(true);
    try {
      const result = await api.transferNft(nft.id, transferTo.trim());
      if (result.success) {
        setNotification(`NFT transferred to ${shortAddress(transferTo)}`);
        refreshNfts();
        setView("nfts");
      } else {
        setError(result.message);
      }
    } catch (e: any) {
      setError(e.message);
    } finally {
      setTransferring(false);
      setShowTransfer(false);
    }
  };

  const handleRefreshConfirm = async (energy: number) => {
    try {
      const result = await api.refreshNft(nft.id, energy);
      if (result.success) {
        setNotification(`Refreshed NFT with ${energy.toLocaleString()} energy`);
        refreshNfts();
        // Update selected NFT optimistically
        const updated = await api.getNft(nft.id);
        useWallet.getState().selectNft(updated);
      } else {
        setError(result.message);
      }
    } catch (e: any) {
      setError(e.message);
    }
    setShowRefresh(false);
  };

  return (
    <div className="flex flex-col h-full">
      <Header />

      <div className="px-4 pt-4 pb-2 flex items-center justify-between">
        <button
          onClick={() => setView("nfts")}
          className="text-xs text-zinc-500 hover:text-zinc-300"
        >
          &larr; Gallery
        </button>
        <span className={`text-[10px] px-2.5 py-1 rounded-full border ${stateBadgeClass}`}>
          {stateIcon} {nft.state}
        </span>
      </div>

      <div className="flex-1 overflow-y-auto px-4 pb-4 space-y-4">
        {/* NFT Image */}
        <div className={`w-full aspect-square rounded-xl bg-zinc-800 flex items-center justify-center ${isGhost ? "grayscale opacity-60" : ""}`}>
          {nft.image_url ? (
            <img
              src={nft.image_url}
              alt={nft.name}
              className="w-full h-full object-cover rounded-xl"
            />
          ) : (
            <span className="text-5xl text-zinc-600">🖼</span>
          )}
        </div>

        {/* Name + collection + ID */}
        <div>
          <h2 className="text-lg font-semibold text-zinc-100">{nft.name}</h2>
          <p className="text-xs text-zinc-400">{nft.collection}</p>
          <p className="text-[10px] text-zinc-600 font-mono mt-0.5">
            ID: {nft.id.slice(0, 12)}...{nft.id.slice(-6)}
          </p>
        </div>

        {/* Large energy bar */}
        <div className="px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <div className="flex items-center justify-between mb-2">
            <span className="text-xs font-medium text-zinc-300">Energy</span>
            <span className="text-xs font-semibold text-zinc-200">{percent}%</span>
          </div>
          <EnergyBar current={nft.current_energy} max={nft.max_energy} size="md" />
        </div>

        {/* Energy stats grid */}
        <div className="grid grid-cols-2 gap-2">
          <StatCard label="Current Energy" value={nft.current_energy.toLocaleString()} />
          <StatCard label="Max Energy" value={nft.max_energy.toLocaleString()} />
          <StatCard label="Half-life" value={`${nft.half_life} epochs`} />
          <StatCard label="Epochs Remaining" value={nft.epochs_remaining > 0 ? `~${nft.epochs_remaining}` : "Evaporated"} />
          <StatCard label="Decay" value={`${nft.decay_percentage.toFixed(1)}%`} />
          <StatCard label="Status" value={energyStatus(percent)} />
        </div>

        {/* Owner */}
        <div className="px-3 py-2.5 rounded-lg bg-evap-surface border border-evap-border">
          <p className="text-[10px] text-zinc-500 mb-0.5">Owner</p>
          <p className="text-xs text-zinc-300 font-mono break-all">{nft.owner}</p>
        </div>

        {/* Evaporation countdown */}
        {nft.epochs_remaining > 0 && (
          <div className={`px-3 py-2.5 rounded-lg border ${
            nft.epochs_remaining <= 5
              ? "bg-red-500/10 border-red-500/30"
              : isGrace
              ? "bg-evap-amber/10 border-evap-amber/30"
              : "bg-evap-surface border-evap-border"
          }`}>
            <p className={`text-xs text-center font-medium ${
              nft.epochs_remaining <= 5 ? "text-red-400" : isGrace ? "text-evap-amber" : "text-zinc-300"
            }`}>
              Evaporates in ~{nft.epochs_remaining} epochs
            </p>
          </div>
        )}

        {/* Actions */}
        {isOwner && (
          <div className="flex gap-2">
            <button
              onClick={() => setShowRefresh(true)}
              className="flex-1 py-2.5 rounded-lg bg-evap-cyan/10 border border-evap-cyan/30 text-evap-cyan text-xs font-medium hover:bg-evap-cyan/20 transition"
            >
              Refresh Energy
            </button>
            <button
              onClick={() => setShowTransfer(!showTransfer)}
              className="flex-1 py-2.5 rounded-lg bg-evap-purple/10 border border-evap-purple/30 text-evap-purple text-xs font-medium hover:bg-evap-purple/20 transition"
            >
              Transfer
            </button>
          </div>
        )}

        {/* Transfer input */}
        {showTransfer && isOwner && (
          <div className="px-3 py-3 rounded-lg bg-evap-surface border border-evap-border space-y-2">
            <p className="text-xs text-zinc-400">Transfer to address:</p>
            <input
              type="text"
              value={transferTo}
              onChange={e => setTransferTo(e.target.value)}
              placeholder="Recipient address..."
              className="w-full px-3 py-2 rounded-lg bg-zinc-800 border border-evap-border text-xs text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-evap-purple/50"
            />
            <div className="flex gap-2">
              <button
                onClick={() => setShowTransfer(false)}
                className="flex-1 py-2 rounded-lg border border-evap-border text-zinc-500 text-xs hover:text-zinc-300 transition"
              >
                Cancel
              </button>
              <button
                onClick={handleTransfer}
                disabled={!transferTo.trim() || transferring}
                className="flex-1 py-2 rounded-lg bg-evap-purple text-white text-xs font-medium disabled:opacity-50 hover:bg-evap-purple/90 transition"
              >
                {transferring ? "Sending..." : "Confirm"}
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Refresh modal */}
      {showRefresh && (
        <NftRefreshModal
          nft={nft}
          onConfirm={handleRefreshConfirm}
          onClose={() => setShowRefresh(false)}
        />
      )}
    </div>
  );
}

function StatCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="px-3 py-2.5 rounded-lg bg-evap-surface border border-evap-border">
      <p className="text-[10px] text-zinc-500 mb-0.5">{label}</p>
      <p className="text-xs font-semibold text-zinc-200">{value}</p>
    </div>
  );
}

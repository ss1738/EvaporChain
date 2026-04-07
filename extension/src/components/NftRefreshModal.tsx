import { useState } from "react";
import { EnergyBar } from "./EnergyBar";
import type { NftItem } from "@/utils/api";

interface NftRefreshModalProps {
  nft: NftItem;
  onConfirm: (energy: number) => void;
  onClose: () => void;
}

const QUICK_AMOUNTS = [1_000, 5_000, 10_000, 50_000];

export function NftRefreshModal({ nft, onConfirm, onClose }: NftRefreshModalProps) {
  const [amount, setAmount] = useState<number>(0);
  const [inputValue, setInputValue] = useState("");

  const handleInputChange = (val: string) => {
    setInputValue(val);
    const parsed = parseInt(val, 10);
    setAmount(isNaN(parsed) || parsed < 0 ? 0 : parsed);
  };

  const handleQuickAmount = (val: number) => {
    setAmount(val);
    setInputValue(val.toLocaleString());
  };

  const newEnergy = Math.min(nft.current_energy + amount, nft.max_energy);

  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/60"
        onClick={onClose}
      />

      {/* Modal */}
      <div className="relative w-full max-w-[360px] bg-zinc-900 border-t border-evap-border rounded-t-2xl px-4 pt-4 pb-6 space-y-4">
        {/* Header */}
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-zinc-100">Refresh Energy</h3>
          <button
            onClick={onClose}
            className="text-zinc-500 hover:text-zinc-300 text-lg leading-none"
          >
            &times;
          </button>
        </div>

        {/* NFT info */}
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-lg bg-zinc-800 flex items-center justify-center flex-shrink-0">
            {nft.image_url ? (
              <img src={nft.image_url} alt={nft.name} className="w-full h-full object-cover rounded-lg" />
            ) : (
              <span className="text-lg">🖼</span>
            )}
          </div>
          <div className="min-w-0">
            <p className="text-xs font-medium text-zinc-200 truncate">{nft.name}</p>
            <p className="text-[10px] text-zinc-500">{nft.collection}</p>
          </div>
        </div>

        {/* Current energy */}
        <div className="px-3 py-2.5 rounded-lg bg-evap-surface border border-evap-border">
          <p className="text-[10px] text-zinc-500 mb-1">Current Energy</p>
          <EnergyBar current={nft.current_energy} max={nft.max_energy} size="sm" />
        </div>

        {/* Energy input */}
        <div>
          <label className="text-[10px] text-zinc-500 block mb-1">Energy Amount</label>
          <input
            type="number"
            value={inputValue}
            onChange={e => handleInputChange(e.target.value)}
            placeholder="Enter energy amount..."
            className="w-full px-3 py-2.5 rounded-lg bg-zinc-800 border border-evap-border text-sm text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-evap-cyan/50"
          />
        </div>

        {/* Quick amount buttons */}
        <div className="flex gap-2">
          {QUICK_AMOUNTS.map(val => (
            <button
              key={val}
              onClick={() => handleQuickAmount(val)}
              className={`flex-1 py-1.5 rounded-lg border text-[10px] font-medium transition ${
                amount === val
                  ? "bg-evap-cyan/10 border-evap-cyan/40 text-evap-cyan"
                  : "border-evap-border text-zinc-500 hover:text-zinc-300 hover:border-zinc-600"
              }`}
            >
              {val >= 1000 ? `${val / 1000}k` : val}
            </button>
          ))}
        </div>

        {/* Preview */}
        {amount > 0 && (
          <div className="px-3 py-2.5 rounded-lg bg-evap-surface border border-evap-border">
            <p className="text-[10px] text-zinc-500 mb-1">After Refresh</p>
            <EnergyBar current={newEnergy} max={nft.max_energy} size="sm" />
            <p className="text-[10px] text-zinc-400 mt-1">
              {nft.current_energy.toLocaleString()} + {amount.toLocaleString()} = {newEnergy.toLocaleString()} / {nft.max_energy.toLocaleString()}
            </p>
          </div>
        )}

        {/* Actions */}
        <div className="flex gap-2">
          <button
            onClick={onClose}
            className="flex-1 py-2.5 rounded-lg border border-evap-border text-zinc-500 text-xs font-medium hover:text-zinc-300 transition"
          >
            Cancel
          </button>
          <button
            onClick={() => amount > 0 && onConfirm(amount)}
            disabled={amount <= 0}
            className="flex-1 py-2.5 rounded-lg bg-evap-cyan text-zinc-900 text-xs font-semibold disabled:opacity-40 hover:bg-evap-cyan/90 transition"
          >
            Confirm Refresh
          </button>
        </div>
      </div>
    </div>
  );
}

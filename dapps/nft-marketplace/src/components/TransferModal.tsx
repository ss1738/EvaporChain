import { useState } from "react";
import { api } from "@/utils/api";
import type { Nft } from "@/utils/types";

interface TransferModalProps {
  nft: Nft;
  onClose: () => void;
  onTransferred: () => void;
}

export function TransferModal({ nft, onClose, onTransferred }: TransferModalProps) {
  const [to, setTo] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const handleTransfer = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");

    if (!to.trim() || !to.startsWith("0x")) return setError("Enter a valid 0x address");

    setLoading(true);
    try {
      const result = await api.transferNft(nft.id, to.trim());
      if (result.success) {
        onTransferred();
        onClose();
      } else {
        setError(result.message);
      }
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/40 flex items-center justify-center z-50 p-4" onClick={onClose}>
      <div
        className="bg-white rounded-2xl shadow-xl w-full max-w-sm overflow-hidden"
        onClick={e => e.stopPropagation()}
      >
        <div className="px-6 py-4 border-b border-evap-border">
          <h2 className="text-lg font-semibold text-zinc-900">Transfer NFT</h2>
          <p className="text-xs text-zinc-500 mt-0.5">Send "{nft.name}" to another address</p>
        </div>

        <form onSubmit={handleTransfer} className="px-6 py-4 space-y-4">
          <div className="px-3 py-2 rounded-lg bg-zinc-50 border border-zinc-100">
            <div className="flex items-center justify-between">
              <span className="text-xs font-medium text-zinc-700">{nft.name}</span>
              <span className="text-[10px] text-zinc-400">#{nft.id}</span>
            </div>
          </div>

          <div>
            <label className="text-xs font-medium text-zinc-700 mb-1 block">Recipient Address</label>
            <input
              type="text"
              placeholder="0x..."
              value={to}
              onChange={e => setTo(e.target.value)}
              className="input font-mono"
              autoFocus
            />
          </div>

          <div className="px-3 py-2 rounded-lg bg-amber-50 border border-amber-100">
            <p className="text-[11px] text-amber-700">
              This action is irreversible. The NFT will be transferred to the recipient.
            </p>
          </div>

          {error && (
            <p className="text-xs text-evap-red bg-red-50 px-3 py-2 rounded-lg">{error}</p>
          )}

          <div className="flex gap-2 pt-2">
            <button
              type="button"
              onClick={onClose}
              className="flex-1 py-2.5 rounded-lg border border-evap-border text-sm text-zinc-600 hover:bg-zinc-50 transition"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={loading || !to}
              className="flex-1 py-2.5 rounded-lg bg-gradient-to-r from-evap-purple to-evap-cyan text-sm font-semibold text-white hover:opacity-90 transition disabled:opacity-50"
            >
              {loading ? "Transferring..." : "Transfer"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

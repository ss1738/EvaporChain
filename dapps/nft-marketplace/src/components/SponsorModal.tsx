import { useState } from "react";
import type { Nft } from "@/utils/types";
import { api } from "@/utils/api";

interface Props {
  nft: Nft;
  currentEpoch: number;
  patronageNsHex: string;
  onClose: () => void;
  onSponsored: () => void;
}

// Sponsor an NFT by opening a Patronage Covenant against its underlying
// object. The covenant pre-funds N EVAP per epoch for K epochs and grants
// auto-eviction immunity for the duration. Calls POST /api/patronage/pledge.
export function SponsorModal({ nft, currentEpoch, patronageNsHex, onClose, onSponsored }: Props) {
  const [donationPerEpoch, setDonationPerEpoch] = useState(5);
  const [epochs, setEpochs] = useState(60);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const objectIdHex = `nft-${nft.id.toString(16).padStart(40, "0")}`;
  const totalPreFunded = donationPerEpoch * epochs;

  const handleSponsor = async () => {
    setSubmitting(true);
    setError(null);
    try {
      const res = await api.pledgePatronage({
        object_id_hex: objectIdHex,
        namespace_id_hex: patronageNsHex,
        donation_per_epoch: donationPerEpoch,
        epochs,
        current_epoch: currentEpoch,
      });
      if (res.status !== "pledged") {
        throw new Error(res.detail || "pledge failed");
      }
      onSponsored();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Pledge failed");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
      <div className="w-full max-w-sm rounded-2xl border border-evap-border bg-evap-surface p-6 shadow-xl">
        <h3 className="text-lg font-semibold text-zinc-900">Sponsor this NFT</h3>
        <p className="mt-1 text-xs text-zinc-500">
          Open a Patronage Covenant. The NFT becomes immune to auto-eviction for the
          covenant&rsquo;s duration even if its energy decays below threshold.
        </p>

        <div className="mt-4 rounded-lg bg-evap-cyan/5 border border-evap-cyan/20 p-3 text-xs text-evap-cyan">
          <span className="font-mono">{nft.name}</span>
          <span className="ml-2 text-[10px] text-zinc-500">#{nft.id}</span>
        </div>

        <div className="mt-4 grid grid-cols-2 gap-3">
          <div>
            <label className="block text-xs font-medium text-zinc-700">EVAP/epoch</label>
            <input
              type="number"
              min={1}
              value={donationPerEpoch}
              onChange={(e) => setDonationPerEpoch(Math.max(1, Number(e.target.value)))}
              className="mt-1 w-full rounded-lg border border-evap-border bg-evap-surface px-3 py-2 text-sm focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-zinc-700">Epochs</label>
            <input
              type="number"
              min={1}
              value={epochs}
              onChange={(e) => setEpochs(Math.max(1, Number(e.target.value)))}
              className="mt-1 w-full rounded-lg border border-evap-border bg-evap-surface px-3 py-2 text-sm focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
            />
          </div>
        </div>

        <div className="mt-4 rounded-lg bg-zinc-50 p-3 text-xs text-zinc-600">
          Pre-funded total:{" "}
          <span className="font-mono font-semibold text-zinc-900">{totalPreFunded} EVAP</span>
          <p className="mt-1 text-[10px] text-zinc-400">
            Expires at epoch {currentEpoch + epochs}. Unused surplus is refundable on revoke.
          </p>
        </div>

        {error && <p className="mt-3 text-sm text-evap-red">{error}</p>}

        <div className="mt-6 flex gap-3">
          <button
            onClick={onClose}
            className="flex-1 rounded-lg border border-evap-border px-4 py-2 text-sm font-medium text-zinc-600 hover:bg-zinc-50"
          >
            Cancel
          </button>
          <button
            onClick={handleSponsor}
            disabled={submitting}
            className="flex-1 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple px-4 py-2 text-sm font-medium text-white hover:opacity-90 disabled:opacity-50"
          >
            {submitting ? "Pledging..." : "Sponsor"}
          </button>
        </div>
      </div>
    </div>
  );
}

import { useState } from "react";
import { api } from "@/utils/api";

interface MintModalProps {
  ownerAddress: string | null;
  onClose: () => void;
  onMinted: () => void;
}

export function MintModal({ ownerAddress, onClose, onMinted }: MintModalProps) {
  const [name, setName] = useState("");
  const [collection, setCollection] = useState("");
  const [metadata, setMetadata] = useState("");
  const [energy, setEnergy] = useState("10000");
  const [halfLife, setHalfLife] = useState("100");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const handleMint = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");

    if (!name.trim()) return setError("Name is required");
    if (!metadata.trim()) return setError("Description is required");

    const energyVal = parseInt(energy);
    const halfLifeVal = parseInt(halfLife);
    if (isNaN(energyVal) || energyVal < 1) return setError("Energy must be at least 1");
    if (isNaN(halfLifeVal) || halfLifeVal < 1) return setError("Half-life must be at least 1");

    setLoading(true);
    try {
      const result = await api.mintNft({
        name: name.trim(),
        collection: collection.trim() || undefined,
        metadata: metadata.trim(),
        energy: energyVal,
        half_life: halfLifeVal,
        owner: ownerAddress ?? undefined,
      });

      if (result.success) {
        onMinted();
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

  // Estimate lifespan
  const energyVal = parseInt(energy) || 0;
  const halfLifeVal = parseInt(halfLife) || 1;
  const lifespanEpochs = halfLifeVal > 0
    ? Math.ceil(halfLifeVal * Math.log2(Math.max(1, energyVal)))
    : 0;

  return (
    <div className="fixed inset-0 bg-black/40 flex items-center justify-center z-50 p-4" onClick={onClose}>
      <div
        className="bg-white rounded-2xl shadow-xl w-full max-w-md overflow-hidden"
        onClick={e => e.stopPropagation()}
      >
        <div className="px-6 py-4 border-b border-evap-border">
          <h2 className="text-lg font-semibold text-zinc-900">Mint Mortal NFT</h2>
          <p className="text-xs text-zinc-500 mt-0.5">Create an NFT that decays over time</p>
        </div>

        <form onSubmit={handleMint} className="px-6 py-4 space-y-4">
          <Field label="Name" required>
            <input
              type="text"
              placeholder="My Decaying Art"
              value={name}
              onChange={e => setName(e.target.value)}
              maxLength={200}
              className="input"
            />
          </Field>

          <Field label="Collection" hint="Optional">
            <input
              type="text"
              placeholder="Genesis Collection"
              value={collection}
              onChange={e => setCollection(e.target.value)}
              className="input"
            />
          </Field>

          <Field label="Description" required>
            <textarea
              placeholder="Describe your NFT..."
              value={metadata}
              onChange={e => setMetadata(e.target.value)}
              rows={3}
              maxLength={10000}
              className="input resize-none"
            />
          </Field>

          <div className="grid grid-cols-2 gap-3">
            <Field label="Initial Energy" hint="How much life">
              <input
                type="number"
                min={1}
                max={1000000000}
                value={energy}
                onChange={e => setEnergy(e.target.value)}
                className="input"
              />
            </Field>
            <Field label="Half-life (epochs)" hint="Decay rate">
              <input
                type="number"
                min={1}
                max={10000}
                value={halfLife}
                onChange={e => setHalfLife(e.target.value)}
                className="input"
              />
            </Field>
          </div>

          {/* Lifespan estimate */}
          <div className="px-3 py-2 rounded-lg bg-zinc-50 border border-zinc-100">
            <div className="flex items-center justify-between">
              <span className="text-[11px] text-zinc-500">Estimated lifespan</span>
              <span className="text-xs font-semibold text-zinc-700">
                ~{lifespanEpochs.toLocaleString()} epochs
              </span>
            </div>
            <div className="flex items-center justify-between mt-1">
              <span className="text-[11px] text-zinc-500">Energy decay</span>
              <span className="text-[11px] text-zinc-500">
                50% every {halfLifeVal.toLocaleString()} epochs
              </span>
            </div>
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
              disabled={loading}
              className="flex-1 py-2.5 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple text-sm font-semibold text-white hover:opacity-90 transition disabled:opacity-50"
            >
              {loading ? "Minting..." : "Mint NFT"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

function Field({ label, hint, required, children }: {
  label: string; hint?: string; required?: boolean; children: React.ReactNode;
}) {
  return (
    <div>
      <div className="flex items-center gap-1 mb-1">
        <label className="text-xs font-medium text-zinc-700">{label}</label>
        {required && <span className="text-evap-red text-[10px]">*</span>}
        {hint && <span className="text-[10px] text-zinc-400">({hint})</span>}
      </div>
      {children}
    </div>
  );
}

import { useState } from "react";
import { createProposal } from "@/utils/api";

interface Props {
  proposerAddress: string | null;
  onClose: () => void;
  onCreated: () => void;
}

const CATEGORIES = [
  { value: "parameter", label: "Parameter Change" },
  { value: "treasury", label: "Treasury Spend" },
  { value: "upgrade", label: "Protocol Upgrade" },
  { value: "community", label: "Community" },
];

const HALF_LIVES = [
  { value: 50, label: "50 epochs (~2 days)" },
  { value: 100, label: "100 epochs (~4 days)" },
  { value: 200, label: "200 epochs (~8 days)" },
  { value: 500, label: "500 epochs (~20 days)" },
];

export function CreateProposalModal({ proposerAddress, onClose, onCreated }: Props) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [category, setCategory] = useState("parameter");
  const [energy, setEnergy] = useState("5000");
  const [halfLife, setHalfLife] = useState(100);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async () => {
    if (!proposerAddress) {
      setError("Connect your wallet first");
      return;
    }
    if (!title.trim()) {
      setError("Title is required");
      return;
    }
    if (!description.trim()) {
      setError("Description is required");
      return;
    }

    setSubmitting(true);
    setError(null);
    try {
      const result = await createProposal({
        title: title.trim(),
        description: description.trim(),
        proposer: proposerAddress,
        energy: parseInt(energy) || 5000,
        half_life: halfLife,
        category,
      });
      if (result.success) {
        onCreated();
        onClose();
      } else {
        setError(result.message ?? "Failed to create proposal");
      }
    } catch {
      setError("Failed to create proposal");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center px-4">
      <div className="fixed inset-0 bg-black/30" onClick={onClose} />
      <div className="relative bg-white rounded-2xl border border-evap-border w-full max-w-lg p-6 shadow-xl max-h-[90vh] overflow-y-auto">
        <h2 className="text-lg font-bold text-zinc-900 mb-4">Create Proposal</h2>

        {/* Title */}
        <div className="mb-3">
          <label className="text-[10px] text-zinc-400 uppercase tracking-wider block mb-1">Title</label>
          <input
            type="text"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Increase block gas limit to 50M"
            maxLength={120}
            className="w-full px-3 py-2 rounded-lg border border-evap-border text-sm text-zinc-900 focus:outline-none focus:border-evap-purple transition-colors"
          />
        </div>

        {/* Description */}
        <div className="mb-3">
          <label className="text-[10px] text-zinc-400 uppercase tracking-wider block mb-1">Description</label>
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="Explain the motivation, impact, and implementation details..."
            rows={4}
            className="w-full px-3 py-2 rounded-lg border border-evap-border text-sm text-zinc-900 focus:outline-none focus:border-evap-purple transition-colors resize-none"
          />
        </div>

        {/* Category */}
        <div className="mb-3">
          <label className="text-[10px] text-zinc-400 uppercase tracking-wider block mb-1">Category</label>
          <div className="grid grid-cols-2 gap-2">
            {CATEGORIES.map((c) => (
              <button
                key={c.value}
                onClick={() => setCategory(c.value)}
                className={`py-2 rounded-lg border text-xs font-medium transition-colors ${
                  category === c.value
                    ? "border-evap-purple bg-evap-purple/5 text-evap-purple"
                    : "border-evap-border text-zinc-500 hover:border-zinc-300"
                }`}
              >
                {c.label}
              </button>
            ))}
          </div>
        </div>

        {/* Energy */}
        <div className="mb-3">
          <label className="text-[10px] text-zinc-400 uppercase tracking-wider block mb-1">
            Initial Energy (EVAP)
          </label>
          <input
            type="number"
            value={energy}
            onChange={(e) => setEnergy(e.target.value)}
            min="1000"
            className="w-full px-3 py-2 rounded-lg border border-evap-border text-sm text-zinc-900 focus:outline-none focus:border-evap-purple transition-colors"
          />
          <p className="text-[10px] text-zinc-400 mt-1">
            Higher energy = longer proposal lifetime. Minimum 1,000 EVAP.
          </p>
        </div>

        {/* Half-Life */}
        <div className="mb-4">
          <label className="text-[10px] text-zinc-400 uppercase tracking-wider block mb-1">Decay Rate</label>
          <div className="grid grid-cols-2 gap-2">
            {HALF_LIVES.map((hl) => (
              <button
                key={hl.value}
                onClick={() => setHalfLife(hl.value)}
                className={`py-2 rounded-lg border text-xs font-medium transition-colors ${
                  halfLife === hl.value
                    ? "border-evap-cyan bg-evap-cyan/5 text-evap-cyan"
                    : "border-evap-border text-zinc-500 hover:border-zinc-300"
                }`}
              >
                {hl.label}
              </button>
            ))}
          </div>
          <p className="text-[10px] text-zinc-400 mt-1">
            Energy halves every N epochs. Shorter half-life = more urgent voting.
          </p>
        </div>

        {!proposerAddress && (
          <div className="mb-4 px-3 py-2 rounded-lg bg-evap-amber/10 text-[10px] text-evap-amber">
            Connect your wallet to create a proposal
          </div>
        )}

        {error && (
          <div className="mb-4 px-3 py-2 rounded-lg bg-evap-red/10 text-[10px] text-evap-red">
            {error}
          </div>
        )}

        <div className="flex items-center gap-2">
          <button
            onClick={onClose}
            className="flex-1 py-2.5 rounded-xl border border-evap-border text-sm text-zinc-500 hover:bg-zinc-50 transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            disabled={submitting || !proposerAddress}
            className="flex-1 py-2.5 rounded-xl bg-evap-purple text-white text-sm font-medium hover:bg-evap-purple/90 transition-colors disabled:opacity-50"
          >
            {submitting ? "Creating..." : "Create Proposal"}
          </button>
        </div>
      </div>
    </div>
  );
}

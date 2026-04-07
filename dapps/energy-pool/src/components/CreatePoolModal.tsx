import { useState } from "react";
import { api } from "@/utils/api";
import type { DistributionStrategy } from "@/utils/types";

interface CreatePoolModalProps {
  onClose: () => void;
  onCreated: () => void;
}

export function CreatePoolModal({ onClose, onCreated }: CreatePoolModalProps) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [objectIds, setObjectIds] = useState("");
  const [strategy, setStrategy] = useState<DistributionStrategy>("equal");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async () => {
    if (!name.trim()) {
      setError("Pool name is required");
      return;
    }

    const targets = objectIds
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);

    if (targets.length === 0) {
      setError("Add at least one object ID to protect");
      return;
    }

    setSubmitting(true);
    setError(null);

    try {
      const result = await api.createPool({
        name: name.trim(),
        description: description.trim(),
        target_objects: targets,
        strategy,
      });
      if (result.success) {
        onCreated();
        onClose();
      } else {
        setError(result.message);
      }
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : "Failed to create pool";
      setError(message);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm">
      <div className="bg-white rounded-2xl border border-evap-border shadow-xl w-full max-w-lg mx-4 p-6">
        <div className="flex items-center justify-between mb-6">
          <h2 className="text-base font-bold text-zinc-900">Create Energy Pool</h2>
          <button
            onClick={onClose}
            className="text-zinc-400 hover:text-zinc-600 text-lg"
          >
            x
          </button>
        </div>

        <div className="space-y-4">
          {/* Name */}
          <div>
            <label className="block text-xs font-medium text-zinc-700 mb-1">
              Pool Name
            </label>
            <input
              className="input"
              placeholder="e.g. Community Art Guardians"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </div>

          {/* Description */}
          <div>
            <label className="block text-xs font-medium text-zinc-700 mb-1">
              Description
            </label>
            <textarea
              className="input min-h-[80px] resize-none"
              placeholder="What will this pool protect?"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          </div>

          {/* Target Objects */}
          <div>
            <label className="block text-xs font-medium text-zinc-700 mb-1">
              Target Object IDs
            </label>
            <input
              className="input"
              placeholder="obj_abc123, obj_def456, ..."
              value={objectIds}
              onChange={(e) => setObjectIds(e.target.value)}
            />
            <p className="text-[10px] text-zinc-400 mt-1">
              Comma-separated object IDs to protect from evaporation
            </p>
          </div>

          {/* Strategy */}
          <div>
            <label className="block text-xs font-medium text-zinc-700 mb-2">
              Distribution Strategy
            </label>
            <div className="flex gap-3">
              <button
                onClick={() => setStrategy("equal")}
                className={`flex-1 px-3 py-2.5 rounded-lg border text-xs font-medium transition ${
                  strategy === "equal"
                    ? "border-evap-cyan bg-evap-cyan/5 text-evap-cyan"
                    : "border-evap-border text-zinc-500 hover:border-zinc-300"
                }`}
              >
                <div className="font-semibold">Equal Split</div>
                <div className="text-[9px] mt-0.5 opacity-70">
                  Energy distributed evenly across objects
                </div>
              </button>
              <button
                onClick={() => setStrategy("priority-low-energy")}
                className={`flex-1 px-3 py-2.5 rounded-lg border text-xs font-medium transition ${
                  strategy === "priority-low-energy"
                    ? "border-evap-cyan bg-evap-cyan/5 text-evap-cyan"
                    : "border-evap-border text-zinc-500 hover:border-zinc-300"
                }`}
              >
                <div className="font-semibold">Priority Low-Energy</div>
                <div className="text-[9px] mt-0.5 opacity-70">
                  Most energy to objects closest to evaporation
                </div>
              </button>
            </div>
          </div>
        </div>

        {error && (
          <p className="text-xs text-evap-red mt-4">{error}</p>
        )}

        <div className="flex gap-3 mt-6">
          <button
            onClick={onClose}
            className="flex-1 px-4 py-2.5 rounded-lg border border-evap-border text-xs font-medium text-zinc-600 hover:bg-zinc-50 transition"
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            disabled={submitting}
            className="flex-1 px-4 py-2.5 rounded-lg bg-gradient-to-r from-evap-cyan to-evap-green text-xs font-semibold text-white hover:opacity-90 transition disabled:opacity-50"
          >
            {submitting ? "Creating..." : "Create Pool"}
          </button>
        </div>
      </div>
    </div>
  );
}

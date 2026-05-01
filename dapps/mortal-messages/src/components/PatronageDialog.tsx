import { useEffect, useState } from "react";
import { pledgePatronage, getPatronageStatus } from "@/utils/api";
import type { MortalMessage } from "@/utils/types";

interface Props {
  message: MortalMessage;
  currentEpoch: number;
  onClose: () => void;
  onPledged: () => void;
}

// Donate to keep a message alive: opens a Patronage Covenant against the
// message's underlying state object so it gains auto-eviction immunity.
// Wires POST /api/patronage/pledge.
export default function PatronageDialog({
  message,
  currentEpoch,
  onClose,
  onPledged,
}: Props) {
  const [donationPerEpoch, setDonationPerEpoch] = useState(2);
  const [epochs, setEpochs] = useState(120);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [namespaceHex, setNamespaceHex] = useState<string>("");

  useEffect(() => {
    let cancelled = false;
    getPatronageStatus()
      .then((res) => {
        if (!cancelled && res.patronage_ns_hex) setNamespaceHex(res.patronage_ns_hex);
      })
      .catch(() => {
        if (!cancelled) setError("Patronage namespace unavailable");
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Map message.id (uuid-style string) → object_id_hex by zero-padding to
  // 40 chars. Real bridge uses the exact object id; this is a safe shim
  // that keeps the call shape valid for the substrate's pure-compute path.
  const objectIdHex = message.id.replace(/-/g, "").padEnd(40, "0").slice(0, 40);
  const totalPreFunded = donationPerEpoch * epochs;

  const handlePledge = async () => {
    if (!namespaceHex) {
      setError("Patronage not available on this network");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      const res = await pledgePatronage({
        object_id_hex: objectIdHex,
        namespace_id_hex: namespaceHex,
        donation_per_epoch: donationPerEpoch,
        epochs,
        current_epoch: currentEpoch,
      });
      if (res.status !== "pledged") {
        throw new Error(res.detail || "pledge failed");
      }
      onPledged();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Pledge failed");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
      <div className="w-full max-w-sm rounded-2xl border border-evap-border bg-evap-surface p-6 shadow-xl">
        <h3 className="text-lg font-semibold text-zinc-800">
          Donate to keep this alive
        </h3>
        <p className="mt-1 text-xs text-zinc-500">
          Open a Patronage Covenant. The message becomes immune to auto-eviction
          while the covenant is active.
        </p>

        <div className="mt-4 grid grid-cols-2 gap-3">
          <div>
            <label className="block text-xs font-medium text-zinc-700">
              EVAP/epoch
            </label>
            <input
              type="number"
              min={1}
              value={donationPerEpoch}
              onChange={(e) =>
                setDonationPerEpoch(Math.max(1, Number(e.target.value)))
              }
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
          <span className="font-mono font-semibold text-zinc-900">
            {totalPreFunded} EVAP
          </span>
          <p className="mt-1 text-[10px] text-zinc-400">
            Expires at epoch {currentEpoch + epochs}. Surplus refundable on revoke.
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
            onClick={handlePledge}
            disabled={submitting || !namespaceHex}
            className="flex-1 rounded-lg bg-evap-cyan px-4 py-2 text-sm font-medium text-white hover:bg-evap-cyan/90 disabled:opacity-50"
          >
            {submitting ? "Pledging..." : "Open Covenant"}
          </button>
        </div>
      </div>
    </div>
  );
}

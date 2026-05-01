import { useState } from "react";
import { sendMessage } from "@/utils/api";
import LifespanPicker from "./LifespanPicker";
import SenderDemurrageWarning from "./SenderDemurrageWarning";

interface Props {
  senderAddress: string;
  onSent: () => void;
}

export default function ComposeMessage({ senderAddress, onSent }: Props) {
  const [to, setTo] = useState("");
  const [content, setContent] = useState("");
  const [energy, setEnergy] = useState(100);
  const [halfLife, setHalfLife] = useState(1440);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);

  const handleSend = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!to.trim() || !content.trim()) return;

    setSubmitting(true);
    setError(null);
    setSuccess(false);

    try {
      await sendMessage({
        to: to.trim(),
        content: content.trim(),
        energy,
        half_life: halfLife,
      });
      setSuccess(true);
      setTo("");
      setContent("");
      setEnergy(100);
      setHalfLife(1440);
      onSent();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to send message");
    } finally {
      setSubmitting(false);
    }
  };

  const estimatedLifeMinutes = halfLife * 10;
  const estimatedCostPerDay = energy / (estimatedLifeMinutes / 1440);

  return (
    <div className="space-y-4">
      <h2 className="text-xl font-semibold text-zinc-800">
        Compose Message {"\uD83D\uDCAC"}
      </h2>

      <form onSubmit={handleSend} className="space-y-4">
        {/* Sender info */}
        <div className="rounded-lg bg-zinc-50 px-3 py-2 text-xs text-zinc-500">
          From: <span className="font-mono">{senderAddress}</span>
        </div>

        {/* Defensive UX: warn if sender owes demurrage. */}
        <SenderDemurrageWarning senderAddress={senderAddress} />

        {/* Recipient */}
        <div>
          <label className="block text-sm font-medium text-zinc-700">
            Recipient Address
          </label>
          <input
            type="text"
            value={to}
            onChange={(e) => setTo(e.target.value)}
            placeholder="evap1x7k2..."
            className="mt-1 w-full rounded-lg border border-evap-border bg-evap-surface px-3 py-2 text-sm placeholder:text-zinc-300 focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
            required
          />
        </div>

        {/* Message content */}
        <div>
          <label className="block text-sm font-medium text-zinc-700">Message</label>
          <textarea
            value={content}
            onChange={(e) => setContent(e.target.value)}
            rows={4}
            maxLength={500}
            placeholder="Write your mortal message..."
            className="mt-1 w-full resize-none rounded-lg border border-evap-border bg-evap-surface px-3 py-2 text-sm placeholder:text-zinc-300 focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
            required
          />
          <div className="mt-1 text-right text-xs text-zinc-400">
            {content.length}/500
          </div>
        </div>

        {/* Energy */}
        <div>
          <label className="block text-sm font-medium text-zinc-700">
            Energy (EVP)
          </label>
          <input
            type="number"
            min={1}
            max={10000}
            value={energy}
            onChange={(e) => setEnergy(Math.max(1, Number(e.target.value)))}
            className="mt-1 w-full rounded-lg border border-evap-border bg-evap-surface px-3 py-2 text-sm focus:border-evap-cyan focus:outline-none focus:ring-1 focus:ring-evap-cyan"
          />
          <p className="mt-1 text-xs text-zinc-400">
            More energy = longer initial lifespan. Cost: ~{estimatedCostPerDay.toFixed(2)} EVP/day
          </p>
        </div>

        {/* Half-life picker */}
        <LifespanPicker value={halfLife} onChange={setHalfLife} />

        {error && (
          <div className="rounded-lg border border-evap-red/20 bg-red-50 p-3 text-sm text-evap-red">
            {error}
          </div>
        )}

        {success && (
          <div className="rounded-lg border border-evap-green/20 bg-green-50 p-3 text-sm text-evap-green">
            Message sent successfully! It will start decaying immediately.
          </div>
        )}

        <button
          type="submit"
          disabled={submitting || !to.trim() || !content.trim()}
          className="w-full rounded-lg bg-evap-cyan px-4 py-2.5 text-sm font-semibold text-white transition-colors hover:bg-evap-cyan/90 disabled:opacity-50"
        >
          {submitting ? "Sending..." : "Send Mortal Message"}
        </button>
      </form>
    </div>
  );
}

import { useEffect, useState } from "react";
import { useWallet, type Toast } from "@/hooks/useWallet";

/**
 * Transient bottom-center toast surface for tx finalisation.
 *
 * Mounted once at the popup root (popup/App.tsx). Reads the toast queue
 * from the wallet store; each toast slides up on mount and auto-dismisses
 * 4 seconds later. Green for finalised, red for rejected. Tx hash chip
 * is click-to-copy.
 *
 * The store's `pollTxStatuses` calls `pushToast` whenever a pending tx
 * newly transitions to `finalised` or `rejected`.
 *
 * Style note: the patronage / refresh-pool palette this app already uses
 * is light/clean; we mirror that here. The dashed `evap-` Tailwind
 * tokens are defined in tailwind.config.js — `evap-green`, `evap-red`,
 * `evap-surface`, `evap-border`, `evap-cyan`.
 */
export function TxToastContainer() {
  const toasts = useWallet(s => s.toasts);

  if (toasts.length === 0) return null;

  return (
    <div className="fixed inset-x-0 bottom-3 z-50 flex flex-col items-center gap-2 pointer-events-none">
      {toasts.map(t => (
        <TxToast key={t.id} toast={t} />
      ))}
    </div>
  );
}

const AUTO_DISMISS_MS = 4000;

function TxToast({ toast }: { toast: Toast }) {
  const dismissToast = useWallet(s => s.dismissToast);
  // Slide-up animation: start hidden, flip on mount.
  const [visible, setVisible] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    // Two-frame delay so the transition from translate-y-full → 0
    // actually animates — without this the element mounts in the
    // final position and skips the slide.
    const enterId = requestAnimationFrame(() => {
      requestAnimationFrame(() => setVisible(true));
    });
    const dismissId = setTimeout(() => {
      setVisible(false);
      // Wait for the leave transition before removing from store.
      setTimeout(() => dismissToast(toast.id), 200);
    }, AUTO_DISMISS_MS);
    return () => {
      cancelAnimationFrame(enterId);
      clearTimeout(dismissId);
    };
  }, [toast.id, dismissToast]);

  const isFinalised = toast.kind === "finalised";
  const palette = isFinalised
    ? "bg-evap-surface border-evap-green/40 text-zinc-100"
    : "bg-evap-surface border-evap-red/40 text-zinc-100";
  const icon = isFinalised ? "✓" : "✗";
  const iconColor = isFinalised ? "text-evap-green" : "text-evap-red";
  const label = isFinalised ? "Tx finalised" : "Tx rejected";

  const shortHash = `${toast.hash.slice(0, 8)}…${toast.hash.slice(-6)}`;

  const handleCopy = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(toast.hash);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      // Clipboard API unavailable — silently ignore.
    }
  };

  return (
    <div
      className={`pointer-events-auto w-[19rem] max-w-[92%] rounded-lg border px-3 py-2 shadow-lg transition-all duration-200 ${palette} ${
        visible ? "translate-y-0 opacity-100" : "translate-y-full opacity-0"
      }`}
    >
      <div className="flex items-start gap-2">
        <span className={`text-base leading-none ${iconColor}`}>{icon}</span>
        <div className="flex-1 min-w-0">
          <p className="text-xs font-semibold leading-tight">
            {label} <span className="text-zinc-400 font-normal">— {toast.summary}</span>
          </p>
          {!isFinalised && toast.error && (
            <p className="mt-0.5 text-[10px] font-mono text-evap-red truncate" title={toast.error}>
              {toast.error}
            </p>
          )}
          <button
            onClick={handleCopy}
            className="mt-1 inline-flex items-center gap-1 text-[10px] font-mono px-1.5 py-0.5 rounded border border-evap-border bg-evap-bg text-evap-cyan hover:border-evap-cyan/60 transition"
            title="Click to copy full tx hash"
          >
            <span>{shortHash}</span>
            <span className="text-zinc-500">{copied ? "copied" : "copy"}</span>
          </button>
        </div>
        <button
          onClick={() => dismissToast(toast.id)}
          className="text-zinc-500 hover:text-zinc-300 text-xs leading-none"
          title="Dismiss"
        >
          ×
        </button>
      </div>
    </div>
  );
}

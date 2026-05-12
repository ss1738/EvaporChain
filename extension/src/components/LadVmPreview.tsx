import { useEffect, useState } from "react";
import { useWallet } from "@/hooks/useWallet";
import { api, type LadAction, type LadMode, type LadSimResp } from "@/utils/api";

/**
 * LAD-VM resource-lifecycle preview.
 *
 * Probe surface for the substructural-resource type system: pick a mode
 * (linear / affine / decaying) + an action (use / drop / tick) and the
 * preview returns the lifecycle outcome at the current epoch.
 *
 * Endpoint: POST /api/lad_vm/simulate (api.rs L1335). Pure compute, no
 * signature required. Request fields per LadSimQuery (L1300-1311):
 *   { mode, value, created_at_epoch, decay_window?, current_epoch, action }
 * Response fields per LadSimResp (L1314-1329):
 *   { status, action, mode, outcome, returned_value, is_evaporated_at_query,
 *     created_at_epoch, current_epoch, decay_window, detail }
 *
 * Wiring: SendScreen now exposes a "Send to object" mode that resolves
 * a target `StateObject`. The component is gated by callers on
 * `is_lad_typed === true` — the previous `import.meta.env.DEV` gate is
 * gone. Callers may pass `initialMode` to pre-fill the mode dropdown
 * from the resolved object's `lad_mode`.
 */
export function LadVmPreview({ initialMode }: { initialMode?: LadMode } = {}) {
  return <LadVmPreviewInner initialMode={initialMode} />;
}

function LadVmPreviewInner({ initialMode }: { initialMode?: LadMode }) {
  const { chainStatus } = useWallet();

  const currentEpoch = chainStatus?.epoch ?? 0;
  // Default created_at_epoch = current - 5 (clamped at 0). Decaying-mode
  // resources are usually mid-lifecycle when the user opens the preview.
  const defaultCreatedAt = Math.max(0, currentEpoch - 5);

  const [mode, setMode] = useState<LadMode>(initialMode ?? "decaying");
  const [action, setAction] = useState<LadAction>("use");
  const [value, setValue] = useState<string>("42");
  const [createdAtEpoch, setCreatedAtEpoch] = useState<string>(String(defaultCreatedAt));
  const [decayWindow, setDecayWindow] = useState<string>("10");
  const [currentEpochInput, setCurrentEpochInput] = useState<string>(String(currentEpoch));
  const [result, setResult] = useState<LadSimResp | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Re-sync auto-filled epoch fields if chain status arrives after mount.
  useEffect(() => {
    if (chainStatus) {
      setCreatedAtEpoch((prev) => (prev === "0" ? String(defaultCreatedAt) : prev));
      setCurrentEpochInput((prev) => (prev === "0" ? String(currentEpoch) : prev));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chainStatus]);

  const handleSimulate = async () => {
    setError(null);
    setBusy(true);
    setResult(null);
    try {
      const v = parseInt(value, 10);
      const created = parseInt(createdAtEpoch, 10);
      const cur = parseInt(currentEpochInput, 10);
      const dw = mode === "decaying" ? parseInt(decayWindow, 10) : undefined;

      if (!Number.isFinite(v) || v < 0) throw new Error("value must be ≥ 0");
      if (!Number.isFinite(created) || created < 0) throw new Error("created_at_epoch must be ≥ 0");
      if (!Number.isFinite(cur) || cur < 0) throw new Error("current_epoch must be ≥ 0");
      if (mode === "decaying" && (!Number.isFinite(dw!) || dw! <= 0)) {
        throw new Error("decay_window must be > 0 for decaying mode");
      }

      const resp = await api.simulateLadVm({
        mode,
        action,
        value: v,
        created_at_epoch: created,
        current_epoch: cur,
        decay_window: dw,
      });
      setResult(resp);
    } catch (e: any) {
      setError(e.message || "Simulation failed");
    } finally {
      setBusy(false);
    }
  };

  const outcomeColor =
    !result ? "text-zinc-400"
      : result.status === "ok" && (result.outcome === "ok" || result.outcome === "ticked-fresh") ? "text-evap-green"
      : result.outcome.startsWith("ticked-") ? "text-evap-amber"
      : "text-evap-red";

  return (
    <div className="rounded-lg border border-evap-purple/30 bg-evap-surface p-3 mt-3">
      <div className="flex items-center justify-between mb-2">
        <div className="flex items-center gap-2">
          <span className="text-[10px] font-semibold px-2 py-0.5 rounded-full bg-evap-purple/15 text-evap-purple border border-evap-purple/40">
            LAD-VM
          </span>
          <span className="text-xs font-medium text-zinc-300 uppercase tracking-wide">
            Resource preview
          </span>
        </div>
        <span className="text-[8px] text-zinc-600 uppercase tracking-wider">LAD-typed</span>
      </div>

      <p className="text-xs text-zinc-500 leading-snug mb-3">
        Probe substructural-resource lifecycle. Linear must be used once,
        affine can drop, decaying evaporates past its window.
      </p>

      <div className="grid grid-cols-2 gap-2 mb-2">
        <Field label="Mode">
          <select
            value={mode}
            onChange={(e) => setMode(e.target.value as LadMode)}
            className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-xs text-zinc-200 focus:outline-none focus:border-evap-purple/60"
          >
            <option value="linear">linear</option>
            <option value="affine">affine</option>
            <option value="decaying">decaying</option>
          </select>
        </Field>
        <Field label="Action">
          <select
            value={action}
            onChange={(e) => setAction(e.target.value as LadAction)}
            className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-xs text-zinc-200 focus:outline-none focus:border-evap-purple/60"
          >
            <option value="use">use</option>
            <option value="drop">drop</option>
            <option value="tick">tick</option>
          </select>
        </Field>
        <Field label="Value">
          <input
            type="number"
            min="0"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-xs text-zinc-200 focus:outline-none focus:border-evap-purple/60"
          />
        </Field>
        <Field label="Created at epoch">
          <input
            type="number"
            min="0"
            value={createdAtEpoch}
            onChange={(e) => setCreatedAtEpoch(e.target.value)}
            className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-xs text-zinc-200 focus:outline-none focus:border-evap-purple/60"
          />
        </Field>
        {mode === "decaying" && (
          <Field label="Decay window">
            <input
              type="number"
              min="1"
              value={decayWindow}
              onChange={(e) => setDecayWindow(e.target.value)}
              className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-xs text-zinc-200 focus:outline-none focus:border-evap-purple/60"
            />
          </Field>
        )}
        <Field label="Current epoch">
          <input
            type="number"
            min="0"
            value={currentEpochInput}
            onChange={(e) => setCurrentEpochInput(e.target.value)}
            className="w-full bg-evap-bg border border-evap-border rounded px-2 py-1 text-xs text-zinc-200 focus:outline-none focus:border-evap-purple/60"
          />
        </Field>
      </div>

      {error && (
        <div className="rounded-md bg-evap-red/10 border border-evap-red/30 px-2 py-1.5 mb-2">
          <p className="text-xs text-evap-red">{error}</p>
        </div>
      )}

      <button
        onClick={handleSimulate}
        disabled={busy}
        className="w-full py-1.5 rounded bg-evap-purple/15 border border-evap-purple/40 text-xs font-semibold text-evap-purple hover:bg-evap-purple/25 transition disabled:opacity-50"
      >
        {busy ? "Simulating…" : "Simulate"}
      </button>

      {result && (
        <div className="mt-2 rounded-md bg-evap-bg border border-evap-border p-2 space-y-1">
          <div className="flex items-center justify-between">
            <span className="text-[10px] text-zinc-500 uppercase tracking-wider">Outcome</span>
            <span className={`text-xs font-semibold ${outcomeColor}`}>
              {result.outcome}
            </span>
          </div>
          <div className="flex items-center justify-between">
            <span className="text-[10px] text-zinc-500 uppercase tracking-wider">Returned value</span>
            <span className="text-xs text-zinc-300 font-mono">
              {result.returned_value ?? "—"}
            </span>
          </div>
          <div className="flex items-center justify-between">
            <span className="text-[10px] text-zinc-500 uppercase tracking-wider">Evaporated</span>
            <span className={`text-xs font-mono ${result.is_evaporated_at_query ? "text-evap-red" : "text-evap-green"}`}>
              {result.is_evaporated_at_query ? "yes" : "no"}
            </span>
          </div>
          {result.detail && (
            <p className="text-[10px] text-zinc-500 leading-snug pt-1 border-t border-evap-border/60">
              {result.detail}
            </p>
          )}
        </div>
      )}
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="text-[8px] text-zinc-500 uppercase tracking-wider mb-0.5">{label}</p>
      {children}
    </div>
  );
}

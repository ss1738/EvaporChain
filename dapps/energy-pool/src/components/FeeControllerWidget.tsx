import { useEffect, useState } from "react";
import { api } from "@/utils/api";
import type { FeeControllerStatus, FeeControllerStepResponse } from "@/utils/types";

// Lyapunov fee-controller widget. Surfaces the chain's current base fee and
// drift (V_after - V_before) so LPs see when the chain is at fee equilibrium.
// Reads /api/fee_controller/status, simulates a single step at the current
// target gas to compute drift.
export function FeeControllerWidget() {
  const [status, setStatus] = useState<FeeControllerStatus | null>(null);
  const [drift, setDrift] = useState<FeeControllerStepResponse | null>(null);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const s = await api.getFeeControllerStatus();
        if (cancelled) return;
        setStatus(s);
        // Simulate a one-block step at target gas to compute Lyapunov drift.
        const step = await api.stepFeeController({
          gas_used: s.target_gas,
          epochs_elapsed: 1,
        });
        if (!cancelled) setDrift(step);
      } catch {
        /* informational widget; preserve last-known state */
      }
    };
    tick();
    const id = setInterval(tick, 12_000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  if (!status) return null;

  const delta = drift?.lyapunov_delta ?? null;
  const converging = delta !== null && delta < 0;
  const equilibrium = delta !== null && Math.abs(delta) < 1e-6;

  const tone = equilibrium
    ? "border-evap-green/30 bg-emerald-50 text-evap-green"
    : converging
      ? "border-evap-cyan/30 bg-cyan-50 text-evap-cyan"
      : "border-evap-amber/30 bg-amber-50 text-evap-amber";

  return (
    <div className={`rounded-xl border ${tone} p-4`}>
      <div className="flex items-center justify-between">
        <div>
          <p className="text-xs font-semibold">Fee Controller</p>
          <p className="text-[10px] opacity-80">
            Lyapunov-stable base fee — LPs can see when fees are at equilibrium.
          </p>
        </div>
        <div className="text-right">
          <p className="text-lg font-bold">
            {status.base_fee.toLocaleString()} <span className="text-xs">μEVAP</span>
          </p>
          <p className="text-[10px] opacity-80">base fee</p>
        </div>
      </div>

      <div className="mt-3 grid grid-cols-3 gap-2 text-[10px]">
        <Stat label="Energy" value={status.energy.toLocaleString()} />
        <Stat label="Target gas" value={status.target_gas.toLocaleString()} />
        <Stat
          label="V drift"
          value={delta === null ? "—" : delta.toFixed(3)}
          mono
        />
      </div>

      <p className="mt-2 text-[10px] opacity-80">
        {equilibrium
          ? "At equilibrium — fees stable."
          : converging
            ? "Converging — V is shrinking."
            : "Pressure imbalance — base fee will adjust."}
      </p>
    </div>
  );
}

function Stat({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="rounded-md bg-white/40 px-2 py-1">
      <p className={`font-semibold ${mono ? "font-mono" : ""}`}>{value}</p>
      <p className="opacity-70">{label}</p>
    </div>
  );
}

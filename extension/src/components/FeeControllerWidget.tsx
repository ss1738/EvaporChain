import { useEffect, useMemo } from "react";
import { useWallet } from "@/hooks/useWallet";

/**
 * Compact Lyapunov-stable fee controller card.
 *
 * Reads GET /api/fee_controller/status (api.rs §get_fee_controller_status,
 * ~L4672). The status response is a serde_json::Value containing:
 *   { status, energy, base_fee, target_energy, target_gas, fee_response_ppm }
 *
 * Pressure is derived locally as energy / target_energy. The drift
 * arrow uses the cached `feeDriftHistory` populated by the store —
 * sum of recent samples; negative = converging.
 */
export function FeeControllerWidget() {
  const { feeStatus, feeDriftHistory, refreshFeeStatus } = useWallet();

  useEffect(() => {
    refreshFeeStatus();
  }, [refreshFeeStatus]);

  const pressure = useMemo(() => {
    if (!feeStatus || feeStatus.target_energy === 0) return null;
    return feeStatus.energy / feeStatus.target_energy;
  }, [feeStatus]);

  const pressureBand: "low" | "med" | "high" =
    pressure == null ? "low" : pressure < 0.66 ? "low" : pressure < 1.33 ? "med" : "high";

  // Sum of recent drift samples — negative = converging.
  const recentDrift = feeDriftHistory.slice(-8).reduce((a, b) => a + b, 0);
  const converging = recentDrift <= 0;

  // Sparkline: scale to band [-1, 1] using max abs.
  const spark = feeDriftHistory.slice(-12);
  const maxAbs = Math.max(1, ...spark.map((v) => Math.abs(v)));

  return (
    <div className="mx-4 mt-3 px-3 py-2.5 rounded-lg bg-evap-surface border border-evap-border">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="text-[10px] text-zinc-500 uppercase tracking-wider">Base fee</p>
          <div className="flex items-baseline gap-1.5 mt-0.5">
            <span className="text-lg font-bold text-zinc-100 tabular-nums">
              {feeStatus ? feeStatus.base_fee.toLocaleString() : "—"}
            </span>
            <span className="text-[10px] text-zinc-500">wei/gas</span>
          </div>
        </div>

        <div className="flex flex-col items-end gap-1 shrink-0">
          <PressurePill band={pressureBand} />
          <DriftIndicator converging={converging} delta={recentDrift} />
        </div>
      </div>

      {/* Sparkline */}
      {spark.length > 1 && (
        <div className="mt-2 h-6 flex items-end gap-[2px]">
          {spark.map((v, i) => {
            const heightPct = (Math.abs(v) / maxAbs) * 100;
            const isNeg = v < 0;
            return (
              <div
                key={i}
                className="flex-1 flex flex-col justify-center"
                title={`Δ V = ${v.toFixed(2)}`}
              >
                <div className="h-3 flex items-end">
                  <div
                    className={`w-full rounded-t-sm ${isNeg ? "bg-evap-green/60" : "bg-evap-amber/60"}`}
                    style={{ height: `${Math.max(heightPct, 6)}%` }}
                  />
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Footer detail */}
      {feeStatus && (
        <div className="flex justify-between items-center mt-2 pt-2 border-t border-evap-border">
          <span className="text-[10px] text-zinc-500">
            E {feeStatus.energy.toLocaleString()} / target {feeStatus.target_energy.toLocaleString()}
          </span>
          <span className="text-[10px] text-zinc-500">
            ρ {feeStatus.fee_response_ppm.toLocaleString()} ppm
          </span>
        </div>
      )}
    </div>
  );
}

function PressurePill({ band }: { band: "low" | "med" | "high" }) {
  const cls =
    band === "low"
      ? "bg-evap-green/10 border-evap-green/30 text-evap-green"
      : band === "med"
      ? "bg-evap-amber/10 border-evap-amber/30 text-evap-amber"
      : "bg-evap-red/10 border-evap-red/30 text-evap-red";
  const label = band === "low" ? "Low pressure" : band === "med" ? "Med pressure" : "High pressure";
  return (
    <span className={`text-[10px] px-1.5 py-0.5 rounded-full border ${cls}`}>{label}</span>
  );
}

function DriftIndicator({ converging, delta }: { converging: boolean; delta: number }) {
  return (
    <span
      className={`text-[10px] flex items-center gap-1 ${
        converging ? "text-evap-green" : "text-evap-amber"
      }`}
      title={`Σ ΔV (last 8) = ${delta.toFixed(2)}`}
    >
      <span>{converging ? "↘" : "↗"}</span>
      <span>{converging ? "Fees converging" : "Fees diverging"}</span>
    </span>
  );
}

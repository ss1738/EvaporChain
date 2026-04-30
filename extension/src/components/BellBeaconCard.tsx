import { useEffect, useState } from "react";
import { useWallet } from "@/hooks/useWallet";

/**
 * Bell-Beacon — quantum-randomness certification card.
 *
 * The CHSH inequality bounds the strength of correlations between two
 * spacelike-separated measurements under any local-realist theory:
 *
 *     S = E(a,b) − E(a,b′) + E(a′,b) + E(a′,b′)
 *     |S| ≤ 2  (local realism)
 *
 * Quantum mechanics permits S up to 2√2 ≈ 2.828. The chain rejects any
 * beacon update whose attested S does not exceed the local-realism
 * floor — so a "Certified" badge means the validators' attested
 * measurements are quantum-mechanically incompatible with any classical
 * source.
 *
 * Endpoints (api.rs):
 *   POST /api/bell_beacon — L3447 — request: BellBeaconQuery
 *     { e_ab, e_ab_prime, e_a_prime_b, e_a_prime_b_prime, threshold_milli? }
 *     response: BellBeaconResp { status, s_value_milli, threshold_milli,
 *     bell_certified, detail } (L3438-3445).
 *
 * There is no read-only GET /api/bell or /api/beacon for the Bell-
 * Beacon module — beacon S-values are computed per-block and would
 * need to be exposed via the chain header to be queryable. Until
 * that's surfaced, this card runs ONE simulation on mount with the
 * documented worked-example values from /api/bell_beacon's API doc
 * entry (api.rs L5884) so users can see the design-target S-value.
 *
 * Read-only. No signing. Mounted from EnergyDashboard near the
 * refresh-pool button.
 */
export function BellBeaconCard() {
  const { bellSValue, bellThreshold, bellCertified, refreshBellBeacon } = useWallet();
  const [hasMounted, setHasMounted] = useState(false);

  useEffect(() => {
    if (!hasMounted) {
      setHasMounted(true);
      // ONE read on mount — see file header.
      refreshBellBeacon();
    }
  }, [hasMounted, refreshBellBeacon]);

  const s = bellSValue ?? 0;
  const threshold = bellThreshold ?? 2000;
  const certified = bellCertified === true;
  const sDecimal = (s / 1000).toFixed(3);
  const thresholdDecimal = (threshold / 1000).toFixed(2);
  const ratio = threshold > 0 ? Math.min(s / threshold, 1.5) : 0;

  return (
    <div className="rounded-lg bg-evap-surface border border-evap-border p-3">
      <div className="flex items-center justify-between mb-2">
        <div className="flex items-center gap-2">
          <span className="text-[9px] font-semibold px-2 py-0.5 rounded-full bg-evap-cyan/15 text-evap-cyan border border-evap-cyan/40">
            Bell
          </span>
          <span className="text-[10px] font-medium text-zinc-300 uppercase tracking-wide">
            Beacon health
          </span>
        </div>
        {bellSValue !== null && (
          certified ? (
            <span className="text-[9px] font-semibold px-2 py-0.5 rounded-full bg-evap-green/15 text-evap-green border border-evap-green/40">
              Certified ✓
            </span>
          ) : (
            <span className="text-[9px] font-semibold px-2 py-0.5 rounded-full bg-evap-amber/15 text-evap-amber border border-evap-amber/40">
              Local-realism floor
            </span>
          )
        )}
      </div>

      {/* Big S */}
      <div className="flex items-baseline gap-2 mb-2">
        <span className="text-2xl font-bold text-zinc-100">
          {bellSValue === null ? "—" : sDecimal}
        </span>
        <span className="text-[10px] text-zinc-500">S-value</span>
      </div>

      {/* Threshold bar */}
      <div className="relative w-full h-1.5 bg-evap-bg border border-evap-border rounded-full overflow-hidden mb-1">
        <div
          className={`h-full ${certified ? "bg-evap-green" : "bg-evap-amber"} transition-all`}
          style={{ width: `${Math.min((ratio / 1.5) * 100, 100)}%` }}
        />
        {/* Local-realism threshold line at 2/2.828 ≈ 70.7% of the bar's max. */}
        <div
          className="absolute top-0 bottom-0 w-px bg-evap-purple/80"
          style={{ left: `${(1 / 1.5) * 100}%` }}
          title={`Local-realism threshold: ${thresholdDecimal}`}
        />
      </div>
      <div className="flex justify-between text-[9px] text-zinc-500 mb-2">
        <span>0</span>
        <span className="text-evap-purple">|S|=2 ({thresholdDecimal})</span>
        <span>2.83</span>
      </div>

      <p className="text-[10px] text-zinc-500 leading-snug">
        CHSH bound: <span className="font-mono">|S| ≤ 2</span> for any
        local-realist source. <span className="font-mono">S &gt; 2</span>{" "}
        certifies quantum-mechanical correlation in the beacon.
      </p>
      <p className="text-[9px] text-zinc-600 leading-snug mt-1.5">
        This card shows the design-target S. Per-block validator-attested
        S is reported in the chain header.
      </p>
    </div>
  );
}

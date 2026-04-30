import { useEffect, useMemo, useState } from "react";
import { api, type HlwaSupplyResp } from "@/utils/api";
import { useWallet } from "@/hooks/useWallet";

/**
 * HLWA — Half-Life Wrapped Asset card.
 *
 * Endpoints (api.rs):
 *   POST /api/hlwa/effective_supply  — L4374 — pure compute. Body:
 *        { current_supply, origin_attested_supply, last_attested_epoch,
 *          attestation_lambda_epochs, current_epoch }
 *        Resp: { status, effective_supply, current_supply, excess_to_burn,
 *                current_epoch }
 *   POST /api/hlwa/re_attest         — L4401 — pure simulation; returns
 *        { status, new_attested_supply, new_last_attested_epoch,
 *          effective_supply_after }. Read-only — no broadcast.
 *
 * Neither accepts a signature. The card is self-contained: fetches on
 * mount and on user re-attest. Default demo params come from the api.rs
 * doc example (1M / 500-epoch λ / 100-epoch elapsed).
 */
export function HlwaCard() {
  const { chainStatus, refreshChainStatus } = useWallet();

  // Default demo asset — from api.rs doc example.
  const [currentSupply, setCurrentSupply] = useState(1_000_000);
  const [originAttested, setOriginAttested] = useState(1_000_000);
  const [lastAttestedEpoch, setLastAttestedEpoch] = useState(0);
  const lambdaEpochs = 500;

  const [supply, setSupply] = useState<HlwaSupplyResp | null>(null);
  const [loading, setLoading] = useState(false);
  const [reAttesting, setReAttesting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Use chain epoch when available; fall back to 100 (matches doc example).
  const currentEpoch = chainStatus?.epoch ?? 100;

  const fetchSupply = useMemo(
    () => async () => {
      setLoading(true);
      setError(null);
      try {
        const resp = await api.getHlwaEffectiveSupply({
          current_supply: currentSupply,
          origin_attested_supply: originAttested,
          last_attested_epoch: lastAttestedEpoch,
          attestation_lambda_epochs: lambdaEpochs,
          current_epoch: currentEpoch,
        });
        setSupply(resp);
        if (resp.status !== "ok") setError(resp.detail || "supply lookup failed");
      } catch (e: any) {
        setError(e.message);
      } finally {
        setLoading(false);
      }
    },
    [currentSupply, originAttested, lastAttestedEpoch, currentEpoch],
  );

  useEffect(() => {
    refreshChainStatus();
  }, [refreshChainStatus]);

  useEffect(() => {
    fetchSupply();
  }, [fetchSupply]);

  const handleReAttest = async () => {
    setReAttesting(true);
    setError(null);
    try {
      const resp = await api.reAttestHlwa({
        current_supply: currentSupply,
        origin_attested_supply: originAttested,
        last_attested_epoch: lastAttestedEpoch,
        attestation_lambda_epochs: lambdaEpochs,
        current_epoch: currentEpoch,
      });
      if (resp.status === "ok") {
        // Apply the simulated re-attestation locally — no broadcast.
        setOriginAttested(resp.new_attested_supply);
        setLastAttestedEpoch(resp.new_last_attested_epoch);
        // Re-fetch effective supply with the updated values.
        const refreshed = await api.getHlwaEffectiveSupply({
          current_supply: currentSupply,
          origin_attested_supply: resp.new_attested_supply,
          last_attested_epoch: resp.new_last_attested_epoch,
          attestation_lambda_epochs: lambdaEpochs,
          current_epoch: currentEpoch,
        });
        setSupply(refreshed);
      } else {
        setError(resp.detail || "re-attest failed");
      }
    } catch (e: any) {
      setError(e.message);
    } finally {
      setReAttesting(false);
    }
  };

  const ratio =
    supply && supply.current_supply > 0
      ? supply.effective_supply / supply.current_supply
      : 0;
  const ratioPct = Math.max(0, Math.min(100, ratio * 100));
  const excess = supply?.excess_to_burn ?? 0;

  return (
    <div className="px-3 py-3 rounded-lg border border-zinc-200 bg-white space-y-3">
      <div className="flex items-start justify-between">
        <div>
          <h3 className="text-xs font-semibold text-zinc-800">HLWA wrapped asset</h3>
          <p className="text-[10px] text-zinc-500 mt-0.5">
            Effective supply decays as attestation freshness ages (λ-decay).
          </p>
        </div>
        <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-cyan-50 text-cyan-700 border border-cyan-200">
          λ {lambdaEpochs} ep
        </span>
      </div>

      {error && (
        <div className="px-2 py-1.5 rounded bg-red-50 border border-red-200">
          <p className="text-[10px] text-red-600">{error}</p>
        </div>
      )}

      <div className="grid grid-cols-2 gap-3">
        <Stat label="Current supply" value={currentSupply.toLocaleString()} unit="HLWA" />
        <Stat
          label="Effective supply"
          value={supply ? supply.effective_supply.toLocaleString() : "—"}
          unit="HLWA"
          highlight
        />
        <Stat label="Last attested" value={`Epoch ${lastAttestedEpoch.toLocaleString()}`} />
        <Stat label="Current epoch" value={`Epoch ${currentEpoch.toLocaleString()}`} />
      </div>

      {/* Ratio bar */}
      <div>
        <div className="flex justify-between text-[9px] text-zinc-500 uppercase tracking-wider mb-1">
          <span>Effective / current</span>
          <span className="tabular-nums">{(ratio * 100).toFixed(1)}%</span>
        </div>
        <div className="w-full h-2 rounded-full bg-zinc-100 overflow-hidden">
          <div
            className={`h-full transition-all duration-500 ${
              ratioPct > 80
                ? "bg-emerald-500"
                : ratioPct > 50
                ? "bg-amber-500"
                : "bg-red-500"
            }`}
            style={{ width: `${ratioPct}%` }}
          />
        </div>
      </div>

      {/* Excess-to-burn warning */}
      {excess > 0 && (
        <div className="px-2 py-1.5 rounded bg-amber-50 border border-amber-200">
          <p className="text-[10px] text-amber-700">
            <span className="font-semibold">{excess.toLocaleString()} HLWA</span> excess
            above effective supply — bridge would burn this on next anti-inflation gate.
          </p>
        </div>
      )}

      <div className="flex gap-2">
        <button
          onClick={fetchSupply}
          disabled={loading}
          className="flex-1 py-1.5 rounded-lg bg-zinc-100 text-zinc-700 text-[11px] font-medium hover:bg-zinc-200 transition disabled:opacity-50"
        >
          {loading ? "…" : "Refresh"}
        </button>
        <button
          onClick={handleReAttest}
          disabled={reAttesting || loading}
          className="flex-1 py-1.5 rounded-lg bg-cyan-600 text-white text-[11px] font-medium hover:bg-cyan-700 transition disabled:opacity-50"
        >
          {reAttesting ? "Re-attesting…" : "Re-attest from origin"}
        </button>
      </div>

      <p className="text-[9px] text-zinc-400 leading-snug">
        Re-attestation is simulated read-only on this endpoint — no
        cross-chain broadcast. The chain endpoint
        <span className="font-mono"> /api/hlwa/re_attest </span>
        returns the post-attestation state without mutating chain state.
      </p>
    </div>
  );
}

function Stat({
  label,
  value,
  unit,
  highlight,
}: {
  label: string;
  value: string;
  unit?: string;
  highlight?: boolean;
}) {
  return (
    <div>
      <p className="text-[9px] text-zinc-500 uppercase tracking-wider">{label}</p>
      <div className="flex items-baseline gap-1 mt-0.5">
        <span
          className={`text-sm font-semibold tabular-nums ${
            highlight ? "text-cyan-700" : "text-zinc-800"
          }`}
        >
          {value}
        </span>
        {unit && <span className="text-[9px] text-zinc-500">{unit}</span>}
      </div>
    </div>
  );
}

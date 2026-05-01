import { useEffect, useState } from "react";
import type { Nft } from "@/utils/types";
import { api } from "@/utils/api";

interface Props {
  nft: Nft;
  currentEpoch: number;
}

// HLWA-wrapped NFTs (sourced from a bridge) decay against the chain's λ.
// We detect them heuristically by their collection name carrying the
// "hlwa-" prefix (the pattern the bridge contract uses) and then call the
// pure-compute /api/hlwa/effective_supply primitive with reasonable
// origin-attested defaults to surface the decay ratio.
export function HlwaSupplyChip({ nft, currentEpoch }: Props) {
  const [ratio, setRatio] = useState<number | null>(null);

  const isHlwa = nft.collection.toLowerCase().startsWith("hlwa-");

  useEffect(() => {
    if (!isHlwa) return;
    let cancelled = false;
    const lastAttested = nft.last_refreshed > 0 ? nft.last_refreshed : nft.minted_epoch;
    const lambda = nft.half_life > 0 ? nft.half_life : 100;
    api
      .hlwaEffectiveSupply({
        current_supply: nft.max_energy || 1_000_000,
        origin_attested_supply: nft.max_energy || 1_000_000,
        last_attested_epoch: lastAttested,
        attestation_lambda_epochs: lambda,
        current_epoch: currentEpoch,
      })
      .then((res) => {
        if (cancelled) return;
        if (res.current_supply > 0) {
          setRatio(res.effective_supply / res.current_supply);
        }
      })
      .catch(() => {
        if (!cancelled) setRatio(null);
      });
    return () => {
      cancelled = true;
    };
  }, [isHlwa, nft.id, nft.last_refreshed, nft.minted_epoch, nft.half_life, nft.max_energy, currentEpoch]);

  if (!isHlwa || ratio === null) return null;

  const pct = Math.max(0, Math.min(100, Math.round(ratio * 100)));
  const tone =
    pct >= 80
      ? "border-evap-green/30 bg-emerald-50 text-evap-green"
      : pct >= 40
        ? "border-evap-amber/30 bg-amber-50 text-evap-amber"
        : "border-evap-red/30 bg-red-50 text-evap-red";

  return (
    <span
      className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[10px] font-medium ${tone}`}
      title={`Bridge-attested supply has λ-decayed; effective_supply / current_supply ≈ ${pct}%.`}
    >
      HLWA {pct}%
    </span>
  );
}

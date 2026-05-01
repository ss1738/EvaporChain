import { useEffect, useState } from "react";
import { api } from "@/utils/api";

interface Props {
  ownerAddress: string;
  // Owner balance is opaque to the marketplace, so we feed the chip a
  // reasonable upper-bound estimate (1000 EVAP) — the substrate primitive
  // is a pure-compute helper that scales linearly with `balance`, so the
  // shape of the warning is what matters here.
  balance?: number;
  lastTouchedEpoch: number;
  currentEpoch: number;
  // λ_base is a chain-level genesis param. Until /api/fee_controller/status
  // exposes it the marketplace uses a typical mainnet target of 1000 ppm.
  lambdaBasePpm?: number;
  threshold?: number;
}

// Small chip rendered next to an NFT's owner address. Shows the demurrage
// the owner currently owes per the substrate's pure-compute helper. If the
// number is non-zero we render a warning chip — it's a UX cue that the
// owner has not paid demurrage and the NFT is at higher auto-eviction risk.
export function DemurrageChip({
  ownerAddress,
  balance = 1_000,
  lastTouchedEpoch,
  currentEpoch,
  lambdaBasePpm = 1_000,
  threshold = 100,
}: Props) {
  const [owed, setOwed] = useState<number | null>(null);
  const [disabled, setDisabled] = useState(false);

  useEffect(() => {
    let cancelled = false;
    if (currentEpoch <= lastTouchedEpoch) {
      setOwed(0);
      return () => {
        cancelled = true;
      };
    }
    api
      .computeDemurrage({
        balance,
        last_touched_epoch: lastTouchedEpoch,
        current_epoch: currentEpoch,
        lambda_base_ppm: lambdaBasePpm,
        threshold,
      })
      .then((res) => {
        if (cancelled) return;
        setDisabled(res.is_disabled);
        setOwed(res.owed);
      })
      .catch(() => {
        if (!cancelled) setOwed(null);
      });
    return () => {
      cancelled = true;
    };
  }, [ownerAddress, balance, lastTouchedEpoch, currentEpoch, lambdaBasePpm, threshold]);

  if (owed === null || disabled || owed <= 0) return null;

  return (
    <span
      className="ml-2 inline-flex items-center gap-1 rounded-full border border-evap-amber/30 bg-amber-50 px-2 py-0.5 text-[10px] font-medium text-evap-amber"
      title={`Owner owes ~${owed} EVAP demurrage at epoch ${currentEpoch}.`}
    >
      <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-evap-amber" />
      Owes {owed} EVAP demurrage
    </span>
  );
}

import { useEffect, useState } from "react";
import { computeDemurrage, getStatus } from "@/utils/api";

interface Props {
  senderAddress: string;
}

// Defensive UX: if the sender already owes demurrage, the chain may evict
// outbound objects faster than expected — warn before posting. Pure
// client-side: calls /api/demurrage/owed against a conservative balance
// estimate so the warning surfaces real chain economics, not theatre.
export default function SenderDemurrageWarning({ senderAddress }: Props) {
  const [owed, setOwed] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    let cancelTimers: (() => void) | null = null;
    const run = async () => {
      try {
        const status = await getStatus();
        if (cancelled) return;
        // Without an authoritative `last_touched_epoch` for this sender we
        // approximate by assuming they haven't paid demurrage in the last
        // ~1 day-equivalent of epochs (1440 ≈ 1 day at 1m/epoch). Owed will
        // come back as 0 if the chain has demurrage disabled.
        const lastTouched = Math.max(0, status.epoch - 1440);
        const res = await computeDemurrage({
          balance: 1_000,
          last_touched_epoch: lastTouched,
          current_epoch: status.epoch,
          lambda_base_ppm: 1_000,
          threshold: 100,
        });
        if (cancelled) return;
        if (res.is_disabled) {
          setOwed(0);
          return;
        }
        setOwed(res.owed);
      } catch {
        if (!cancelled) setOwed(null);
      }
    };
    run();
    const id = setInterval(run, 30_000);
    cancelTimers = () => clearInterval(id);
    return () => {
      cancelled = true;
      if (cancelTimers) cancelTimers();
    };
  }, [senderAddress]);

  if (owed === null || owed <= 0) return null;

  return (
    <div className="rounded-lg border border-evap-amber/30 bg-amber-50 p-3 text-xs text-evap-amber">
      <p className="font-semibold">Unpaid demurrage detected</p>
      <p className="mt-0.5 text-[11px] text-amber-900/80">
        Your account currently owes ~{owed} EVAP demurrage. The chain may charge
        this against an outbound object before delivery — your message could
        evaporate sooner than expected. Settle demurrage from your wallet first.
      </p>
    </div>
  );
}

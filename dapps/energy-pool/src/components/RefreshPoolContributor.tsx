import { useEffect, useState } from "react";
import { api } from "@/utils/api";
import type { Pool, RefreshPoolStatus } from "@/utils/types";

interface Props {
  pool: Pool;
  walletAddress: string | null;
  onContributed?: () => void;
}

// Lets pool LPs redirect a fraction of their stake into the protocol-owned
// refresh pool as a public good. Calls POST /api/refresh_pool/contribute when
// the node exposes it; otherwise falls back to a transfer to the patronage
// namespace's vault. Surfaces the live total_accrued from /api/refresh_pool.
export function RefreshPoolContributor({ pool, walletAddress, onContributed }: Props) {
  const [poolStatus, setPoolStatus] = useState<RefreshPoolStatus | null>(null);
  const [fraction, setFraction] = useState(5); // 5% of pool's energy
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const tick = () => {
      api
        .getRefreshPool()
        .then((res) => {
          if (!cancelled) setPoolStatus(res);
        })
        .catch(() => {
          /* informational only */
        });
    };
    tick();
    const id = setInterval(tick, 15_000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  const amount = Math.max(1, Math.floor((pool.total_energy * fraction) / 100));

  const handleContribute = async () => {
    if (!walletAddress) {
      setError("Connect a wallet first");
      return;
    }
    setSubmitting(true);
    setError(null);
    setSuccess(null);
    try {
      const res = await api.contributeToRefreshPool({
        from: walletAddress,
        amount,
        pool_id: pool.id,
        memo: `pool:${pool.id}/refresh-pool-public-good`,
      });
      if (!res.success) {
        throw new Error(res.message || "contribution failed");
      }
      setSuccess(`Contributed ${amount} EVAP to the refresh pool`);
      onContributed?.();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Contribution failed");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="rounded-xl border border-evap-border bg-white p-4">
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-xs font-semibold text-zinc-900">
            Public-Good: Protocol Refresh Pool
          </p>
          <p className="mt-1 text-[10px] text-zinc-500">
            Redirect a fraction of pool yield to the chain&rsquo;s refresh pool.
            Funds object refreshes across all dApps.
          </p>
        </div>
        <p className="text-sm font-bold text-evap-cyan">
          {poolStatus ? `${poolStatus.total_accrued.toLocaleString()} EVAP` : "—"}
        </p>
      </div>

      <div className="mt-3 flex items-center gap-3">
        <input
          type="range"
          min={1}
          max={50}
          step={1}
          value={fraction}
          onChange={(e) => setFraction(Number(e.target.value))}
          className="flex-1"
        />
        <span className="w-14 text-right text-xs font-mono text-zinc-700">
          {fraction}% · {amount.toLocaleString()} EVAP
        </span>
      </div>

      {error && <p className="mt-2 text-xs text-evap-red">{error}</p>}
      {success && <p className="mt-2 text-xs text-evap-green">{success}</p>}

      <button
        onClick={handleContribute}
        disabled={submitting || !walletAddress}
        className="mt-3 w-full rounded-lg bg-gradient-to-r from-evap-cyan to-evap-purple px-4 py-2 text-xs font-semibold text-white hover:opacity-90 disabled:opacity-50"
      >
        {submitting ? "Contributing..." : "Contribute to Refresh Pool"}
      </button>
    </div>
  );
}

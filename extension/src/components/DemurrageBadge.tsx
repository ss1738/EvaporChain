import { useEffect, useState } from "react";
import { AlertTriangle } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";

/**
 * Compact pill that shows demurrage owed on the active account.
 *
 * Backed by POST /api/demurrage/owed (api.rs §post_demurrage_owed) which
 * is a pure compute: returns { owed, rate_ppm, remaining_balance,
 * is_disabled, ... }. We use is_disabled to hide the badge when the
 * chain has demurrage off.
 *
 * `last_touched_epoch` is now exposed on /api/address/:addr (api.rs
 * §AddressDetailResponse) — though the node still serves `0` until the
 * Account struct carries the field. The badge therefore renders in
 * production once accountLastTouched > 0 AND demurrageOwed > 0; until
 * then it stays hidden so we don't over-charge from genesis.
 *
 * The "Settle now" button calls POST /api/tx/settle_demurrage which
 * debits the owed amount from the account and credits the protocol-
 * owned refresh pool under namespace "DEMU".
 */
export function DemurrageBadge() {
  const {
    demurrageOwed,
    balance,
    chainStatus,
    accountLastTouched,
    refreshDemurrage,
    settleDemurrage,
    loading,
  } = useWallet();
  const [open, setOpen] = useState(false);

  useEffect(() => {
    refreshDemurrage();
  }, [refreshDemurrage]);

  // Render only when we have a real touch epoch AND owed > 0. Until
  // Account.last_touched_epoch is populated server-side this stays
  // hidden in production — that's the correct behaviour.
  const hasTouch = accountLastTouched != null && accountLastTouched > 0;
  if (!hasTouch) return null;
  if (demurrageOwed == null || demurrageOwed <= 0) return null;

  const high = demurrageOwed > Math.max(1, balance * 0.01);

  return (
    <>
      <button
        onClick={() => setOpen(true)}
        className={`inline-flex items-center gap-1 text-[9px] px-2 py-0.5 rounded-full border ${
          high
            ? "bg-evap-red/10 text-evap-red border-evap-red/30 hover:border-evap-red/60"
            : "bg-evap-amber/10 text-evap-amber border-evap-amber/30 hover:border-evap-amber/60"
        } transition`}
        title="Demurrage owed on idle balance"
      >
        <span><AlertTriangle className="w-3.5 h-3.5" strokeWidth={1.5} /></span>
        <span className="font-semibold tabular-nums">
          {demurrageOwed.toLocaleString()} EVAP owed
        </span>
      </button>

      {open && (
        <DemurrageModal
          owed={demurrageOwed}
          balance={balance}
          currentEpoch={chainStatus?.epoch ?? 0}
          lastTouchedEpoch={accountLastTouched ?? 0}
          settling={loading}
          onSettle={async () => {
            const resp = await settleDemurrage();
            if (resp && resp.status !== "error") {
              setOpen(false);
            }
          }}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}

function DemurrageModal({
  owed,
  balance,
  currentEpoch,
  lastTouchedEpoch,
  settling,
  onSettle,
  onClose,
}: {
  owed: number;
  balance: number;
  currentEpoch: number;
  lastTouchedEpoch: number;
  settling: boolean;
  onSettle: () => void;
  onClose: () => void;
}) {
  const sinceEpoch = lastTouchedEpoch;
  const elapsed = Math.max(0, currentEpoch - sinceEpoch);
  // Genesis params used in the store.
  const lambdaBasePpm = 1;
  const threshold = 1024;

  return (
    <div
      className="fixed inset-0 z-50 bg-black/70 flex items-end justify-center"
      onClick={onClose}
    >
      <div
        className="w-full max-w-sm bg-evap-surface border-t border-evap-border rounded-t-xl p-4 space-y-3"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-start justify-between">
          <div>
            <h3 className="text-sm font-semibold text-zinc-100">Demurrage owed</h3>
            <p className="text-xs text-zinc-500 mt-0.5">
              Idle-balance fee on the EvaporChain substrate.
            </p>
          </div>
          <button onClick={onClose} className="text-zinc-500 hover:text-zinc-300 text-sm">×</button>
        </div>

        <div className="grid grid-cols-2 gap-3">
          <Field label="Owed">
            <span className="text-evap-amber tabular-nums">{owed.toLocaleString()} EVAP</span>
          </Field>
          <Field label="Balance after">
            <span className="text-zinc-200 tabular-nums">
              {Math.max(0, balance - owed).toLocaleString()} EVAP
            </span>
          </Field>
          <Field label="Elapsed epochs">
            <span className="text-zinc-300 tabular-nums">{elapsed.toLocaleString()}</span>
          </Field>
          <Field label="Since epoch">
            <span className="text-zinc-300 tabular-nums">{sinceEpoch}</span>
          </Field>
          <Field label="Rate (λ_base)">
            <span className="text-zinc-300 tabular-nums">{lambdaBasePpm} ppm/epoch</span>
          </Field>
          <Field label="Threshold">
            <span className="text-zinc-300 tabular-nums">{threshold.toLocaleString()}</span>
          </Field>
        </div>

        <div className="px-3 py-2 rounded-lg bg-evap-cyan/5 border border-evap-cyan/20">
          <p className="text-[9px] text-zinc-400 leading-snug">
            Settling now debits <span className="font-mono">{owed.toLocaleString()}</span> EVAP
            from your balance and credits the protocol-owned refresh pool
            under namespace <span className="font-mono">DEMU</span>. The owed amount
            is recomputed at every block from <span className="font-mono">last_touched_epoch</span>
            and the piecewise-log rate.
          </p>
        </div>

        <button
          onClick={onSettle}
          disabled={settling}
          className="w-full py-2 rounded-lg bg-evap-cyan/15 border border-evap-cyan/40 text-xs font-semibold text-evap-cyan hover:bg-evap-cyan/25 transition disabled:opacity-50"
        >
          {settling ? "Settling…" : "Settle now"}
        </button>
      </div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="text-[9px] text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-xs font-semibold mt-0.5">{children}</p>
    </div>
  );
}

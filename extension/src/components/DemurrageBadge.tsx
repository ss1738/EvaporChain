import { useEffect, useState } from "react";
import { useWallet } from "@/hooks/useWallet";

/**
 * Compact pill that shows demurrage owed on the active account.
 *
 * Backed by POST /api/demurrage/owed (api.rs §post_demurrage_owed, L5366)
 * which is a pure compute: returns { owed, rate_ppm, remaining_balance,
 * is_disabled, ... }. Note the response does NOT contain a `capped`
 * field — the brief said {owed, capped} but the real shape is owed +
 * is_disabled + remaining_balance. We use is_disabled to hide the badge
 * when the chain has demurrage off.
 *
 * IMPORTANT: GET /api/address/:addr (§AddressDetailResponse, L7389-7397)
 * does NOT expose `last_touched_epoch`. Without it, a real owed value
 * cannot be computed. The badge is therefore gated behind
 * import.meta.env.DEV until last_touched_epoch is added to the address
 * response; the demo value uses last_touched_epoch=0.
 *
 * No demurrage/settle endpoint exists in api.rs — the modal's "Settle now"
 * action is disabled with a tooltip explaining settlement happens on the
 * next refresh tx.
 */
export function DemurrageBadge() {
  const { demurrageOwed, balance, chainStatus, refreshDemurrage } = useWallet();
  const [open, setOpen] = useState(false);

  // Hide outside dev mode — see header comment.
  const enabled = import.meta.env.DEV;

  useEffect(() => {
    if (!enabled) return;
    refreshDemurrage();
  }, [enabled, refreshDemurrage]);

  if (!enabled) return null;
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
        <span>⚠</span>
        <span className="font-semibold tabular-nums">
          {demurrageOwed.toLocaleString()} EVAP owed
        </span>
      </button>

      {open && (
        <DemurrageModal
          owed={demurrageOwed}
          balance={balance}
          currentEpoch={chainStatus?.epoch ?? 0}
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
  onClose,
}: {
  owed: number;
  balance: number;
  currentEpoch: number;
  onClose: () => void;
}) {
  // last_touched_epoch is unknown at this layer (see header comment).
  // We display 0 to match what refreshDemurrage actually sent.
  const sinceEpoch = 0;
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
            <p className="text-[10px] text-zinc-500 mt-0.5">
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
            Settlement is automatic on your next refresh tx — there is no settle endpoint
            yet on this node. The owed amount is recomputed at every block from
            <span className="font-mono"> last_touched_epoch </span>
            and the piecewise-log rate.
          </p>
        </div>

        <button
          disabled
          title="Settlement happens on next refresh tx"
          className="w-full py-2 rounded-lg bg-evap-cyan/10 border border-evap-cyan/20 text-[11px] font-medium text-evap-cyan/40 cursor-not-allowed"
        >
          Settle now (unavailable)
        </button>
      </div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="text-[9px] text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[11px] font-semibold mt-0.5">{children}</p>
    </div>
  );
}

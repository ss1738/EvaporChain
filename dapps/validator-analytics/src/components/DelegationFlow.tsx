import type { DelegationRow } from "@/lib/api";
import { fmtEvap, shortAddr } from "@/lib/api";

/** Delegation flow over the last N epochs. The chain only commits to bonded
 *  / unbonding amounts per delegator (no historical event stream), so we
 *  bucket entries by `delegated_at_epoch` and surface unbonding pressure. */
export function DelegationFlow({
  rows,
  epochSpan = 20,
}: {
  rows: DelegationRow[];
  epochSpan?: number;
}) {
  if (rows.length === 0) {
    return (
      <div className="rounded-xl border border-evap-border bg-white p-4 text-xs text-zinc-500">
        No active delegations.
      </div>
    );
  }
  const maxEpoch = rows.reduce((m, r) => Math.max(m, r.delegated_at_epoch), 0);
  const minEpoch = Math.max(0, maxEpoch - epochSpan + 1);
  const inflowByEpoch = new Map<number, number>();
  let unbonding = 0;
  for (const r of rows) {
    if (r.delegated_at_epoch >= minEpoch) {
      inflowByEpoch.set(
        r.delegated_at_epoch,
        (inflowByEpoch.get(r.delegated_at_epoch) ?? 0) + r.amount,
      );
    }
    unbonding += r.unbonding_amount;
  }
  const max = Math.max(1, ...inflowByEpoch.values());
  const epochs = Array.from({ length: epochSpan }, (_, i) => minEpoch + i);

  return (
    <div className="rounded-xl border border-evap-border bg-white p-4">
      <div className="flex items-center justify-between">
        <p className="text-xs font-semibold text-zinc-900">Delegation Inflow (last {epochSpan} epochs)</p>
        {unbonding > 0 && (
          <span className="text-[10px] text-evap-amber">
            {fmtEvap(unbonding)} EVAP unbonding
          </span>
        )}
      </div>
      <div className="mt-3 flex h-20 items-end gap-px">
        {epochs.map((e) => {
          const v = inflowByEpoch.get(e) ?? 0;
          const h = (v / max) * 100;
          return (
            <div
              key={e}
              title={`epoch ${e}: ${fmtEvap(v)} EVAP`}
              className="flex-1 rounded-t-sm bg-evap-cyan/70"
              style={{ height: `${Math.max(h, v > 0 ? 4 : 0)}%` }}
            />
          );
        })}
      </div>
      <div className="mt-1 flex justify-between text-[9px] text-zinc-400">
        <span>epoch {minEpoch}</span>
        <span>epoch {maxEpoch}</span>
      </div>
      <div className="mt-3 max-h-48 overflow-y-auto">
        <table className="w-full text-[11px]">
          <thead>
            <tr className="text-left text-[9px] uppercase text-zinc-400">
              <th className="py-1">Delegator</th>
              <th className="py-1 text-right">Amount</th>
              <th className="py-1 text-right">Since</th>
              <th className="py-1 text-right">Unbonding</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.delegator + r.delegated_at_epoch} className="border-t border-evap-border/60">
                <td className="py-1 font-mono text-zinc-700">{shortAddr(r.delegator)}</td>
                <td className="py-1 text-right">{fmtEvap(r.amount)}</td>
                <td className="py-1 text-right text-zinc-500">e{r.delegated_at_epoch}</td>
                <td className="py-1 text-right text-evap-amber">
                  {r.unbonding_amount > 0 ? fmtEvap(r.unbonding_amount) : ""}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

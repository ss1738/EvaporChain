import Link from "next/link";
import type { ValidatorRow as VRow } from "@/lib/api";
import { fmtEvap, shortAddr } from "@/lib/api";
import { Sparkline } from "./Sparkline";

export function ValidatorRow({
  v,
  uptime,
  blockHist,
}: {
  v: VRow;
  uptime: number;
  blockHist?: number[];
}) {
  const status = v.jailed ? "jailed" : v.bls_registered ? "active" : "no-bls";
  const tone = v.jailed
    ? "bg-red-50 text-evap-red"
    : v.bls_registered
      ? "bg-green-50 text-evap-green"
      : "bg-amber-50 text-evap-amber";
  return (
    <tr className="border-t border-evap-border hover:bg-zinc-50">
      <td className="px-3 py-2 font-mono text-xs">#{v.id}</td>
      <td className="px-3 py-2 font-mono text-[11px] text-zinc-600">{shortAddr(v.address)}</td>
      <td className="px-3 py-2 text-xs text-right">{fmtEvap(v.stake)}</td>
      <td className="px-3 py-2 text-xs text-right">{fmtEvap(v.effective_stake)}</td>
      <td className="px-3 py-2 text-xs text-right">{fmtEvap(v.blocks_produced)}</td>
      <td className="px-3 py-2 text-xs text-right">
        <span className={uptime > 0.9 ? "text-evap-green" : uptime > 0.5 ? "text-evap-amber" : "text-evap-red"}>
          {(uptime * 100).toFixed(1)}%
        </span>
      </td>
      <td className="px-3 py-2 text-xs text-right">{v.health_score.toFixed(2)}</td>
      <td className="px-3 py-2 text-xs text-right">{fmtEvap(v.total_slashed)}</td>
      <td className="px-3 py-2">
        <Sparkline values={blockHist ?? []} width={80} height={20} />
      </td>
      <td className="px-3 py-2">
        <span className={`inline-flex rounded-md px-2 py-0.5 text-[10px] font-semibold ${tone}`}>{status}</span>
      </td>
      <td className="px-3 py-2 text-right">
        <Link href={`/validators/${v.id}`} className="text-[11px] font-medium text-evap-cyan hover:underline">
          View →
        </Link>
      </td>
    </tr>
  );
}

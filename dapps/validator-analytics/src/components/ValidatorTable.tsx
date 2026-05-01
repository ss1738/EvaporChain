"use client";

import { useMemo, useState } from "react";
import type { ValidatorRow as VRow } from "@/lib/api";
import { ValidatorRow } from "./ValidatorRow";

type SortKey = "id" | "stake" | "effective_stake" | "blocks_produced" | "uptime" | "health" | "slashed";

export function ValidatorTable({
  rows,
  uptime,
  blockHist,
}: {
  rows: VRow[];
  uptime: Map<number, number>;
  blockHist: Map<number, number[]>;
}) {
  const [sortKey, setSortKey] = useState<SortKey>("stake");
  const [desc, setDesc] = useState(true);

  const sorted = useMemo(() => {
    const copy = [...rows];
    copy.sort((a, b) => {
      const va = pick(a, uptime, sortKey);
      const vb = pick(b, uptime, sortKey);
      return desc ? vb - va : va - vb;
    });
    return copy;
  }, [rows, uptime, sortKey, desc]);

  const click = (k: SortKey) => {
    if (k === sortKey) setDesc((d) => !d);
    else {
      setSortKey(k);
      setDesc(true);
    }
  };

  return (
    <div className="overflow-x-auto rounded-xl border border-evap-border bg-white">
      <table className="w-full text-sm">
        <thead className="bg-zinc-50">
          <tr className="text-left text-[10px] uppercase tracking-wide text-zinc-500">
            <Th onClick={() => click("id")} active={sortKey === "id"} desc={desc}>ID</Th>
            <th className="px-3 py-2">Address</th>
            <Th onClick={() => click("stake")} active={sortKey === "stake"} desc={desc} align="right">Stake</Th>
            <Th onClick={() => click("effective_stake")} active={sortKey === "effective_stake"} desc={desc} align="right">Effective</Th>
            <Th onClick={() => click("blocks_produced")} active={sortKey === "blocks_produced"} desc={desc} align="right">Blocks 24h</Th>
            <Th onClick={() => click("uptime")} active={sortKey === "uptime"} desc={desc} align="right">Uptime*</Th>
            <Th onClick={() => click("health")} active={sortKey === "health"} desc={desc} align="right">Health</Th>
            <Th onClick={() => click("slashed")} active={sortKey === "slashed"} desc={desc} align="right">Slashed</Th>
            <th className="px-3 py-2">Latency</th>
            <th className="px-3 py-2">Status</th>
            <th className="px-3 py-2"></th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((v) => (
            <ValidatorRow
              key={v.id}
              v={v}
              uptime={uptime.get(v.id) ?? 0}
              blockHist={blockHist.get(v.id)}
            />
          ))}
        </tbody>
      </table>
      <p className="border-t border-evap-border px-3 py-2 text-[10px] text-zinc-400">
        * Uptime is a proxy: blocks_produced ÷ max(blocks_produced) across the active set. Chain
        does not expose per-validator missed-slot counts.
      </p>
    </div>
  );
}

function Th({
  children,
  onClick,
  active,
  desc,
  align,
}: {
  children: React.ReactNode;
  onClick: () => void;
  active: boolean;
  desc: boolean;
  align?: "right";
}) {
  return (
    <th
      onClick={onClick}
      className={`cursor-pointer select-none px-3 py-2 hover:text-zinc-900 ${align === "right" ? "text-right" : ""} ${active ? "text-zinc-900" : ""}`}
    >
      {children} {active ? (desc ? "▾" : "▴") : ""}
    </th>
  );
}

function pick(v: VRow, uptime: Map<number, number>, k: SortKey): number {
  switch (k) {
    case "id": return v.id;
    case "stake": return v.stake;
    case "effective_stake": return v.effective_stake;
    case "blocks_produced": return v.blocks_produced;
    case "uptime": return uptime.get(v.id) ?? 0;
    case "health": return v.health_score;
    case "slashed": return v.total_slashed;
  }
}

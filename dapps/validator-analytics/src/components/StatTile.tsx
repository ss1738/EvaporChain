import type { ReactNode } from "react";

export function StatTile({
  label,
  value,
  hint,
  tone = "text-zinc-900",
  icon,
}: {
  label: string;
  value: ReactNode;
  hint?: string;
  tone?: string;
  icon?: ReactNode;
}) {
  return (
    <div className="rounded-xl border border-evap-border bg-white px-4 py-3">
      <div className="flex items-center justify-between">
        <p className="text-[10px] uppercase tracking-wide text-zinc-400">{label}</p>
        {icon}
      </div>
      <p className={`mt-1 text-xl font-bold ${tone}`}>{value}</p>
      {hint && <p className="mt-0.5 text-[10px] text-zinc-400">{hint}</p>}
    </div>
  );
}

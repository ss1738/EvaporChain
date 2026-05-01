// Tx-status pill matching the chain's TxStatus state machine
// (pending → mempool → included → finalised, plus rejected). Colours mirror
// the wallet's status palette so explorers and the wallet stay legible.

type Status = "pending" | "mempool" | "included" | "finalised" | "rejected";

const styles: Record<Status, string> = {
  pending: "bg-zinc-100 text-zinc-600 border-zinc-200",
  mempool: "bg-amber-50 text-evap-amber border-amber-200",
  included: "bg-cyan-50 text-evap-cyan border-cyan-200",
  finalised: "bg-emerald-50 text-evap-green border-emerald-200",
  rejected: "bg-red-50 text-evap-red border-red-200",
};

const labels: Record<Status, string> = {
  pending: "pending",
  mempool: "mempool",
  included: "included",
  finalised: "finalised",
  rejected: "rejected",
};

export function StateBadge({ state }: { state: string }) {
  const s = (state in styles ? state : "pending") as Status;
  return (
    <span
      className={`inline-flex items-center rounded-full border px-2 py-0.5 font-mono text-[10px] font-medium ${styles[s]}`}
    >
      {labels[s]}
    </span>
  );
}

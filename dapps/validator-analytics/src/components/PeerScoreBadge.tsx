/** Peer score badge: green ≥ 0, amber 0…-50, red below. SCORE_BAN_THRESHOLD is -100. */
export function PeerScoreBadge({ score }: { score: number }) {
  let tone = "bg-zinc-100 text-zinc-600";
  if (score >= 10) tone = "bg-green-50 text-evap-green";
  else if (score >= 0) tone = "bg-zinc-50 text-zinc-700";
  else if (score > -50) tone = "bg-amber-50 text-evap-amber";
  else tone = "bg-red-50 text-evap-red";
  return (
    <span className={`inline-flex items-center rounded-md px-2 py-0.5 text-[10px] font-mono font-semibold ${tone}`}>
      {score >= 0 ? "+" : ""}
      {score}
    </span>
  );
}

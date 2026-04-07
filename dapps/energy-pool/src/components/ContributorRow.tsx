import type { Contributor } from "@/utils/types";

interface ContributorRowProps {
  contributor: Contributor;
  isCurrentUser: boolean;
}

export function ContributorRow({ contributor, isCurrentUser }: ContributorRowProps) {
  const rankIcon =
    contributor.rank === 1
      ? "[*]"
      : contributor.rank === 2
        ? "[+]"
        : contributor.rank === 3
          ? "[-]"
          : `#${contributor.rank}`;

  const rankColor =
    contributor.rank === 1
      ? "text-evap-amber"
      : contributor.rank === 2
        ? "text-zinc-400"
        : contributor.rank === 3
          ? "text-evap-ember"
          : "text-zinc-500";

  return (
    <div
      className={`flex items-center gap-3 px-4 py-3 rounded-lg border transition ${
        isCurrentUser
          ? "bg-evap-cyan/5 border-evap-cyan/20"
          : "bg-white border-evap-border"
      }`}
    >
      {/* Rank */}
      <div className={`w-8 text-center font-mono text-xs font-bold ${rankColor}`}>
        {rankIcon}
      </div>

      {/* Address */}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-xs font-mono text-zinc-700 truncate">
            {contributor.address.slice(0, 8)}...{contributor.address.slice(-6)}
          </span>
          {isCurrentUser && (
            <span className="text-[9px] px-1.5 py-0.5 rounded bg-evap-cyan/10 text-evap-cyan font-medium">
              You
            </span>
          )}
        </div>
        <div className="flex items-center gap-3 mt-0.5 text-[10px] text-zinc-400">
          <span>{contributor.staked_energy.toLocaleString()} energy staked</span>
          <span>{contributor.objects_saved} saved</span>
        </div>
      </div>

      {/* Guardian Points */}
      <div className="text-right">
        <div className="flex items-center gap-1 justify-end">
          <span className="text-xs text-evap-purple font-bold">
            {contributor.guardian_points.toLocaleString()}
          </span>
          <span className="text-[10px] text-zinc-400">{`{*}`}</span>
        </div>
        <p className="text-[9px] text-zinc-400">guardian pts</p>
      </div>
    </div>
  );
}

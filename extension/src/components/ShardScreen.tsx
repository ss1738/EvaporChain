import { useEffect, useMemo } from "react";
import { RotateCw } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";

/**
 * Shard visibility dashboard.
 *
 * Surfaces the chain's sharding subsystem:
 *   - GET /api/shards         (api.rs §get_shard_status, ~L8034)
 *     → num_shards + pending_cross_shard_messages
 *   - GET /api/shards/health  (api.rs §get_shard_health, ~L8047)
 *     → per-shard { shard_id, total_objects, live_objects,
 *                   total_energy, liveness_ratio, is_dead }
 *     + compaction_candidates count
 *
 * The wallet itself does NOT receive validator-set rosters from the
 * node, so we surface object counts and liveness ratios as the
 * shard's "health" signal — the closest proxy the public API exposes.
 *
 * Address→shard is computed locally because the node has no
 * /api/address/:addr/shard endpoint; see api.ts
 * `computeShardForAddress` for the formula (mirror of
 * `evaporchain_sharding::shard_for_object`).
 */
export function ShardScreen() {
  const {
    shards,
    shardsHealth,
    addressShard,
    activeAccount,
    refreshShards,
    setView,
  } = useWallet();

  useEffect(() => {
    refreshShards();
  }, [refreshShards]);

  const sortedShards = useMemo(() => {
    if (!shards) return [];
    return [...shards].sort((a, b) => a.shard_id - b.shard_id);
  }, [shards]);

  const noSharding = shardsHealth != null && shardsHealth.active === false;

  return (
    <div className="flex flex-col h-full bg-evap-bg">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-evap-border">
        <button
          onClick={() => setView("home")}
          className="text-zinc-400 hover:text-zinc-200 transition text-sm"
        >
          ←
        </button>
        <div className="flex-1">
          <h1 className="text-sm font-semibold text-zinc-200">Shards</h1>
          <p className="text-xs text-zinc-500">
            Per-shard health and your account's home shard.
          </p>
        </div>
        <button
          onClick={() => refreshShards()}
          className="text-xs px-2 py-1 rounded text-evap-cyan border border-evap-cyan/30 hover:border-evap-cyan/60 transition"
        ><RotateCw className="w-3.5 h-3.5" strokeWidth={1.5} /></button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {/* Summary card */}
        <div className="mx-4 mt-3 px-3 py-3 rounded-lg bg-evap-surface border border-evap-border">
          <p className="text-xs text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
            Sharding Status
          </p>
          {noSharding ? (
            <p className="text-xs text-zinc-500">
              Sharding is disabled on this node. Single-shard chain.
            </p>
          ) : !shardsHealth ? (
            <p className="text-xs text-zinc-600">Loading shard status…</p>
          ) : (
            <>
              <div className="flex items-baseline gap-2">
                <span className="text-2xl font-bold text-zinc-100 tabular-nums">
                  {shardsHealth.total_shards}
                </span>
                <span className="text-xs text-zinc-500">total shards</span>
              </div>
              <div className="grid grid-cols-3 gap-2 mt-3">
                <Stat
                  label="Healthy"
                  value={shardsHealth.healthy.toString()}
                  tone="emerald"
                />
                <Stat
                  label="Unhealthy"
                  value={shardsHealth.unhealthy.toString()}
                  tone={shardsHealth.unhealthy > 0 ? "red" : "default"}
                />
                <Stat
                  label="Pending msgs"
                  value={shardsHealth.pending_cross_shard_messages.toString()}
                  tone="cyan"
                />
              </div>
              {shardsHealth.compaction_candidates > 0 && (
                <p className="text-[10px] text-evap-amber mt-2">
                  {shardsHealth.compaction_candidates} shard
                  {shardsHealth.compaction_candidates === 1 ? "" : "s"} flagged
                  for compaction
                </p>
              )}
            </>
          )}
        </div>

        {/* Your shard */}
        {activeAccount && shardsHealth?.active && addressShard != null && (
          <div className="mx-4 mt-3 px-3 py-3 rounded-lg bg-evap-cyan/5 border border-evap-cyan/20">
            <p className="text-xs text-zinc-400 font-semibold mb-1 uppercase tracking-wider">
              Your Account
            </p>
            <p className="text-xs text-zinc-300">
              Your account is on{" "}
              <span className="font-semibold text-evap-cyan">
                shard {addressShard}
              </span>
              .
            </p>
            <p className="text-[10px] text-zinc-500 font-mono truncate mt-1">
              {activeAccount.address}
            </p>
          </div>
        )}

        {/* Per-shard list */}
        <div className="mx-4 mt-4 mb-4">
          <p className="text-xs text-zinc-400 font-semibold mb-2 uppercase tracking-wider">
            Per-shard health
          </p>
          {!shardsHealth ? (
            <p className="text-xs text-zinc-600 text-center py-8">
              Loading…
            </p>
          ) : sortedShards.length === 0 ? (
            <p className="text-xs text-zinc-600 text-center py-8">
              {noSharding
                ? "Single-shard chain — no per-shard rows."
                : "No per-shard data reported yet."}
            </p>
          ) : (
            <div className="space-y-1.5">
              {sortedShards.map((s) => {
                const isMine = addressShard === s.shard_id;
                const livenessPct = Math.round(s.liveness_ratio * 100);
                const tone = s.is_dead
                  ? "border-evap-red/30 bg-evap-red/5"
                  : isMine
                  ? "border-evap-cyan/40 bg-evap-cyan/5"
                  : "border-evap-border bg-evap-surface";
                const dotColor = s.is_dead
                  ? "bg-evap-red"
                  : livenessPct > 50
                  ? "bg-evap-green"
                  : "bg-evap-amber";

                return (
                  <div
                    key={s.shard_id}
                    className={`rounded-lg px-3 py-2.5 border ${tone}`}
                  >
                    <div className="flex items-center justify-between mb-1.5">
                      <div className="flex items-center gap-2">
                        <span className={`w-1.5 h-1.5 rounded-full ${dotColor}`} />
                        <span className="text-xs font-semibold text-zinc-200">
                          Shard {s.shard_id}
                        </span>
                        {isMine && (
                          <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-evap-cyan/15 text-evap-cyan border border-evap-cyan/30">
                            you
                          </span>
                        )}
                        {s.is_dead && (
                          <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-evap-red/15 text-evap-red border border-evap-red/30">
                            dead
                          </span>
                        )}
                      </div>
                      <span className="text-xs text-zinc-500 tabular-nums">
                        {livenessPct}% live
                      </span>
                    </div>
                    <div className="grid grid-cols-3 gap-2 text-xs">
                      <ShardStat
                        label="Objects"
                        value={`${s.live_objects}/${s.total_objects}`}
                      />
                      <ShardStat
                        label="Energy"
                        value={s.total_energy.toLocaleString()}
                      />
                      <ShardStat
                        label="State"
                        value={s.is_dead ? "compaction" : "active"}
                      />
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        <div className="mx-4 mb-4 px-3 py-2 rounded-lg bg-evap-cyan/5 border border-evap-cyan/20">
          <p className="text-[10px] text-zinc-400 leading-snug">
            Shard assignment is deterministic: each 20-byte object id (and
            account address) maps to one shard via its first two bytes. Cross-
            shard transfers are routed by the shard bridge and may take longer
            to finalise than same-shard ones.
          </p>
        </div>
      </div>
    </div>
  );
}

function Stat({
  label,
  value,
  tone = "default",
}: {
  label: string;
  value: string;
  tone?: "default" | "emerald" | "red" | "cyan";
}) {
  const valueColor =
    tone === "emerald"
      ? "text-evap-green"
      : tone === "red"
      ? "text-evap-red"
      : tone === "cyan"
      ? "text-evap-cyan"
      : "text-zinc-200";
  return (
    <div>
      <p className="text-[10px] text-zinc-500 uppercase tracking-wider">
        {label}
      </p>
      <p className={`text-sm font-semibold mt-0.5 tabular-nums ${valueColor}`}>
        {value}
      </p>
    </div>
  );
}

function ShardStat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="text-[10px] text-zinc-500 uppercase tracking-wider">
        {label}
      </p>
      <p className="text-xs font-medium text-zinc-300 tabular-nums mt-0.5 truncate">
        {value}
      </p>
    </div>
  );
}

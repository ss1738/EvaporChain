import { useEffect, useMemo } from "react";
import { ArrowLeft } from "lucide-react";
import { useWallet } from "@/hooks/useWallet";
import { EnergyBar } from "./EnergyBar";
import { energyStatus } from "@/utils/format";
import { Header } from "./Header";
import { QuickRefresh } from "./QuickRefresh";
import { api } from "@/utils/api";

export function ObjectsScreen() {
  const { objects, refreshObjects, setView, shardsHealth, refreshShards } = useWallet();

  useEffect(() => {
    refreshObjects();
    refreshShards();
    const interval = setInterval(refreshObjects, 5000);
    return () => clearInterval(interval);
  }, [refreshObjects, refreshShards]);

  // /api/objects does NOT include `shard_id` per object (api.rs has no
  // such field on ObjectResponse), so we compute shard locally from
  // the owner address using the same formula the chain uses for
  // 20-byte ids. Single-shard chains skip the chip.
  const shardForOwner = useMemo(() => {
    if (!shardsHealth?.active || shardsHealth.total_shards <= 1) {
      return (_owner: string): number | null => null;
    }
    const cache = new Map<string, number | null>();
    return (owner: string): number | null => {
      const cached = cache.get(owner);
      if (cached !== undefined) return cached;
      const assignment = api.computeShardForAddress(
        owner,
        shardsHealth.total_shards,
      );
      const id = assignment?.shard_id ?? null;
      cache.set(owner, id);
      return id;
    };
  }, [shardsHealth]);

  return (
    <div className="flex flex-col h-full relative">
      <Header />
      <QuickRefresh />
      <div className="px-4 pt-4 pb-2 flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold text-zinc-100">Objects</h2>
          <p className="text-xs text-zinc-500">{objects.length} owned objects</p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => setView("patronage")}
            className="text-xs px-2 py-1 rounded border border-evap-cyan/30 text-evap-cyan hover:border-evap-cyan/60 transition"
            title="Patronage Covenants — pre-fund eviction immunity"
          >
            Patronage
          </button>
          <button
            onClick={() => setView("home")}
            className="text-xs text-zinc-500 hover:text-zinc-300"
          >
            <><ArrowLeft className="inline w-3.5 h-3.5 mr-1 -mt-0.5" strokeWidth={1.5} />Back</>
          </button>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto px-4 pb-4 space-y-2">
        {objects.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-12">
            <span className="text-3xl mb-3">◈</span>
            <p className="text-sm text-zinc-500">No objects yet</p>
            <p className="text-xs text-zinc-600 mt-1">Create one via the testnet explorer</p>
          </div>
        ) : (
          objects
            .sort((a, b) => a.current_energy - b.current_energy) // most urgent first
            .map(obj => (
              <div
                key={obj.id}
                className="px-3 py-3 rounded-lg bg-evap-surface border border-evap-border"
              >
                <div className="flex items-center justify-between mb-2">
                  <div>
                    <p className="text-xs font-semibold text-zinc-200">{obj.name || "Object"}</p>
                    <p className="text-xs text-zinc-500 font-mono">
                      {obj.id.slice(0, 8)}...{obj.id.slice(-6)}
                    </p>
                  </div>
                  <div className="flex items-center gap-1">
                    {(() => {
                      const sid = shardForOwner(obj.owner);
                      if (sid == null) return null;
                      return (
                        <span
                          className="text-[10px] font-semibold px-1.5 py-0.5 rounded-full bg-evap-cyan/10 text-evap-cyan border border-evap-cyan/30"
                          title={`Object lives on shard ${sid} (computed from owner address; api.rs ObjectResponse has no shard_id field)`}
                        >
                          S{sid}
                        </span>
                      );
                    })()}
                    {obj.is_lad_typed === true && (
                      <span
                        className="text-[10px] font-semibold px-1.5 py-0.5 rounded-full bg-evap-purple/15 text-evap-purple border border-evap-purple/40"
                        title={
                          obj.lad_mode
                            ? `LAD-VM substructural mode: ${obj.lad_mode}`
                            : "LAD-VM substructural-resource type"
                        }
                      >
                        {obj.lad_mode ? `LAD · ${obj.lad_mode}` : "LAD"}
                      </span>
                    )}
                    <span className={`text-xs px-2 py-0.5 rounded-full ${
                      obj.state === "Active" ? "bg-evap-green/10 text-evap-green" :
                      obj.state === "Grace" ? "bg-evap-amber/10 text-evap-amber" :
                      obj.state === "Ghost" ? "bg-evap-ghost/10 text-evap-ghost" :
                      "bg-evap-purple/10 text-evap-purple"
                    }`}>
                      {obj.state}
                    </span>
                  </div>
                </div>

                <EnergyBar current={obj.current_energy} max={obj.max_energy} />

                <div className="flex items-center justify-between mt-2">
                  <span className="text-xs text-zinc-500">
                    Half-life: {obj.half_life} epochs
                  </span>
                  <span className="text-xs text-zinc-500">
                    {energyStatus(Math.round((obj.current_energy / obj.max_energy) * 100))}
                  </span>
                </div>
              </div>
            ))
        )}
      </div>
    </div>
  );
}
